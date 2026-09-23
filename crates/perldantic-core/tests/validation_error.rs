//! Error locations, line errors and the public `ValidationError`.
//!
//! Expected strings mirror pydantic's `errors()`, `json()` and `str()` output
//! (see upstream tests/test_errors.py), with the URL prefix pinned to `latest`.

use perldantic_core::{
    CoreError, Dict, ErrorType, ErrorsOptions, InputType, LocItem, Location, ValError,
    ValLineError, ValidationError, Value,
};

fn int_parsing(input: Value) -> ValLineError {
    ValLineError::new(ErrorType::new("int_parsing", None).unwrap(), input)
}

fn string_too_short(min_length: i64) -> ErrorType {
    let mut ctx = Dict::new();
    ctx.insert(Value::from("min_length"), Value::from(min_length));
    ErrorType::new("string_too_short", Some(&ctx)).unwrap()
}

#[test]
fn location_is_built_inside_out_and_rendered_outside_in() {
    let mut loc = Location::default();
    assert_eq!(loc.to_string(), "");
    loc.with_outer(LocItem::from(0_usize));
    loc.with_outer(LocItem::from("foo.bar"));
    loc.with_outer(LocItem::from("model"));
    assert_eq!(loc.to_string(), "model.`foo.bar`.0\n");
    assert_eq!(
        loc.items(),
        vec![
            LocItem::from("model"),
            LocItem::from("foo.bar"),
            LocItem::from(0_i64)
        ]
    );
    assert_eq!(
        serde_json::to_string(&loc).unwrap(),
        r#"["model","foo.bar",0]"#
    );
}

#[test]
fn val_error_with_outer_location_prefixes_every_line_error() {
    let err = ValError::LineErrors(vec![
        int_parsing(Value::from("x")).with_outer_location(0_usize),
        int_parsing(Value::from("y")).with_outer_location(1_usize),
    ])
    .with_outer_location("items");
    let ValError::LineErrors(lines) = err else {
        panic!("expected line errors")
    };
    let locs: Vec<String> = lines.iter().map(|l| l.location.to_string()).collect();
    assert_eq!(locs, ["items.0\n", "items.1\n"]);
    assert_eq!(lines[0].first_loc_item(), Some(&LocItem::from("items")));
}

#[test]
fn errors_list_matches_pydantic_error_details() {
    let error = ValidationError::new(
        "str",
        vec![ValLineError::new(string_too_short(3), Value::from("12"))],
        InputType::Python,
        false,
    );
    assert_eq!(error.error_count(), 1);
    assert_eq!(error.title(), "str");

    let details = error.errors(&ErrorsOptions {
        include_url: false,
        ..ErrorsOptions::default()
    });
    assert_eq!(details.len(), 1);
    let d = &details[0];
    assert_eq!(d.type_, "string_too_short");
    assert!(d.loc.is_empty());
    assert_eq!(d.msg, "String should have at least 3 characters");
    assert_eq!(d.input, Some(Value::from("12")));
    let mut ctx = Dict::new();
    ctx.insert(Value::from("min_length"), Value::from(3_i64));
    assert_eq!(d.ctx, Some(Value::Dict(ctx)));
    assert_eq!(d.url, None);
}

#[test]
fn json_output_matches_pydantic_field_order_and_options() {
    let error = ValidationError::new(
        "str",
        vec![ValLineError::new(string_too_short(3), Value::from("12"))],
        InputType::Python,
        false,
    );
    assert_eq!(
        error.to_json(&ErrorsOptions::default(), None),
        concat!(
            r#"[{"type":"string_too_short","loc":[],"msg":"String should have at least 3 characters","#,
            r#""input":"12","ctx":{"min_length":3},"#,
            r#""url":"https://errors.pydantic.dev/latest/v/string_too_short"}]"#
        )
    );
    let bare = ErrorsOptions {
        include_url: false,
        include_context: false,
        include_input: false,
    };
    assert_eq!(
        error.to_json(&bare, None),
        r#"[{"type":"string_too_short","loc":[],"msg":"String should have at least 3 characters"}]"#
    );
    assert!(
        error
            .to_json(&ErrorsOptions::default(), Some(2))
            .starts_with("[\n  {\n    \"type\": \"string_too_short\",")
    );
}

#[test]
fn display_matches_pydantic_str() {
    let error = ValidationError::new(
        "typed-dict",
        vec![
            int_parsing(Value::from("x"))
                .with_outer_location(0_usize)
                .with_outer_location("foo.bar"),
        ],
        InputType::Python,
        false,
    );
    assert_eq!(
        error.display(true, false),
        concat!(
            "1 validation error for typed-dict\n",
            "`foo.bar`.0\n",
            "  Input should be a valid integer, unable to parse string as an integer ",
            "[type=int_parsing, input_value='x', input_type=str]\n",
            "    For further information visit https://errors.pydantic.dev/latest/v/int_parsing"
        )
    );
    assert_eq!(
        error.display(false, true),
        concat!(
            "1 validation error for typed-dict\n",
            "`foo.bar`.0\n",
            "  Input should be a valid integer, unable to parse string as an integer [type=int_parsing]"
        )
    );
}

#[test]
fn display_pluralises_and_truncates_long_inputs() {
    let long = "a".repeat(100);
    let error = ValidationError::new(
        "Model",
        vec![
            ValLineError::new(
                ErrorType::new("missing", None).unwrap(),
                Value::Dict(Dict::new()),
            )
            .with_outer_location("a"),
            int_parsing(Value::from(long.as_str())).with_outer_location("b"),
        ],
        InputType::Python,
        false,
    );
    let expected_input = format!("'{}...{}'", "a".repeat(24), "a".repeat(23));
    assert_eq!(
        error.display(false, false),
        format!(
            "2 validation errors for Model\na\n  Field required [type=missing, input_value={{}}, input_type=dict]\n\
             b\n  Input should be a valid integer, unable to parse string as an integer \
             [type=int_parsing, input_value={expected_input}, input_type=str]"
        )
    );
}

#[test]
fn custom_errors_have_no_url() {
    let error = ValidationError::new(
        "function-plain[f()]",
        vec![ValLineError::new(
            ErrorType::new_custom_error("my_error", "this is a custom error", None),
            Value::from(42_i64),
        )],
        InputType::Python,
        false,
    );
    assert_eq!(error.errors(&ErrorsOptions::default())[0].url, None);
    assert_eq!(
        error.display(true, false),
        "1 validation error for function-plain[f()]\n  this is a custom error [type=my_error, input_value=42, input_type=int]"
    );
}

#[test]
fn default_factory_not_called_hides_input() {
    let error = ValidationError::new(
        "Model",
        vec![ValLineError::new(
            ErrorType::new("default_factory_not_called", None).unwrap(),
            Value::None,
        )],
        InputType::Python,
        false,
    );
    assert!(
        error
            .display(false, false)
            .ends_with("[type=default_factory_not_called]")
    );
}

#[test]
fn from_val_error_accepts_line_errors_only() {
    let ok = ValidationError::from_val_error(
        "int",
        ValError::LineErrors(vec![int_parsing(Value::from("x"))]),
        InputType::Json,
        false,
    )
    .unwrap();
    assert_eq!(ok.error_count(), 1);

    let internal = ValidationError::from_val_error(
        "int",
        ValError::InternalErr(CoreError::Internal("boom".into())),
        InputType::Json,
        false,
    )
    .unwrap_err();
    assert_eq!(internal, CoreError::Internal("boom".into()));

    let omit =
        ValidationError::from_val_error("int", ValError::Omit, InputType::Json, false).unwrap_err();
    assert_eq!(
        omit,
        CoreError::Schema(
            "Uncaught Omit error, please check your usage of `default` validators.".into()
        )
    );
}
