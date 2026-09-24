//! Perl input (docs/DIVERGENCES.md #8): host data validated with `InputType::Perl` gets Perl's
//! vocabulary in errors (codes, messages, contexts, input values and types), and a Perl array
//! satisfies a strict tuple.

use perldantic_core::{
    ErrorsOptions, InputType, JsonOptions, SchemaSerializer, SchemaValidator, SerializeOptions,
    ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn strict() -> ValidateOptions {
    ValidateOptions {
        strict: Some(true),
        ..ValidateOptions::default()
    }
}

/// `(type, msg)` of each error.
fn errors(v: &SchemaValidator, input: &Value, input_type: InputType) -> Vec<(String, String)> {
    errors_with(v, input, input_type, &ValidateOptions::default())
}

fn errors_with(
    v: &SchemaValidator,
    input: &Value,
    input_type: InputType,
    options: &ValidateOptions,
) -> Vec<(String, String)> {
    match v.validate_value_as(input, input_type, options) {
        Err(ValidateError::Validation(e)) => e
            .errors(&ErrorsOptions::default())
            .into_iter()
            .map(|d| (d.type_, d.msg))
            .collect(),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

fn one(type_: &str, msg: &str) -> Vec<(String, String)> {
    vec![(type_.to_owned(), msg.to_owned())]
}

#[test]
fn perl_input_uses_perl_words() {
    let cases = [
        (
            r#"{"type": "none"}"#,
            "1",
            "undef_required",
            "Input should be undef",
        ),
        (
            r#"{"type": "list"}"#,
            "1",
            "array_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "tuple", "items_schema": [{"type": "int"}]}"#,
            "1",
            "array_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "dict"}"#,
            "1",
            "hash_type",
            "Input should be a hash reference",
        ),
        (
            r#"{"type": "model", "cls": "My::Point", "schema": {"type": "model-fields", "model_name": "My::Point", "fields": {}}}"#,
            "1",
            "model_type",
            "Input should be a hash reference or an instance of My::Point",
        ),
        (
            r#"{"type": "list", "min_length": 2}"#,
            "[1]",
            "too_short",
            "Array should have at least 2 items after validation, not 1",
        ),
        (
            r#"{"type": "dict", "max_length": 1}"#,
            r#"{"a": 1, "b": 2}"#,
            "too_long",
            "Hash should have at most 1 item after validation, not 2",
        ),
        (
            r#"{"type": "int"}"#,
            r#""x""#,
            "int_parsing",
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            r#"{"type": "int"}"#,
            "1.5",
            "int_from_fraction",
            "Input should be a valid integer, got a number with a fractional part",
        ),
        (
            r#"{"type": "float"}"#,
            "[]",
            "number_type",
            "Input should be a valid number",
        ),
        (
            r#"{"type": "float"}"#,
            r#""x""#,
            "number_parsing",
            "Input should be a valid number, unable to parse string as a number",
        ),
        (
            r#"{"type": "set"}"#,
            "1",
            "array_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "frozenset"}"#,
            "1",
            "array_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "bytes"}"#,
            "1",
            "bytes_type",
            "Input should be a valid byte string",
        ),
        (
            r#"{"type": "timedelta"}"#,
            "[]",
            "duration_type",
            "Input should be a valid duration",
        ),
        (
            r#"{"type": "timedelta"}"#,
            r#""x""#,
            "duration_parsing",
            "Input should be a valid duration, invalid digit in duration",
        ),
        (
            r#"{"type": "decimal"}"#,
            "[]",
            "decimal_type",
            "Decimal input should be a number, a string or a Math::BigFloat object",
        ),
        (
            r#"{"type": "uuid"}"#,
            "[]",
            "uuid_type",
            "UUID input should be a string, a byte string or a Perldantic::Uuid object",
        ),
        (
            r#"{"type": "url"}"#,
            "[]",
            "url_type",
            "URL input should be a string or a Perldantic::Url object",
        ),
        (
            r#"{"type": "json"}"#,
            "[]",
            "json_type",
            "JSON input should be a string",
        ),
    ];
    for (schema, input, type_, msg) in cases {
        assert_eq!(
            errors(&validator(schema), &j(input), InputType::Perl),
            one(type_, msg),
            "{schema}"
        );
    }
}

#[test]
fn python_input_keeps_pydantic_words() {
    assert_eq!(
        errors(
            &validator(r#"{"type": "list"}"#),
            &j("1"),
            InputType::Python
        ),
        one("list_type", "Input should be a valid list")
    );
    assert_eq!(
        errors(
            &validator(r#"{"type": "none"}"#),
            &j("1"),
            InputType::Python
        ),
        one("none_required", "Input should be None")
    );
}

fn perl_error(
    schema: &str,
    input: &Value,
    options: &ValidateOptions,
) -> perldantic_core::ValidationError {
    match validator(schema).validate_value_as(input, InputType::Perl, options) {
        Err(ValidateError::Validation(e)) => e,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn contexts_use_perl_words() {
    let e = perl_error(
        r#"{"type": "list", "min_length": 2}"#,
        &j("[1]"),
        &ValidateOptions::default(),
    );
    let ctx = e.errors(&ErrorsOptions::default())[0].ctx.clone().unwrap();
    assert_eq!(
        ctx,
        j(r#"{"field_type": "Array", "min_length": 2, "actual_length": 1}"#)
    );
    let e = perl_error(r#"{"type": "decimal"}"#, &j(r#""1.5""#), &strict());
    let details = &e.errors(&ErrorsOptions::default())[0];
    assert_eq!(details.msg, "Input should be an instance of Math::BigFloat");
    assert_eq!(details.ctx, Some(j(r#"{"class": "Math::BigFloat"}"#)));
}

#[test]
fn perl_errors_have_no_pydantic_links() {
    let e = perl_error(r#"{"type": "none"}"#, &j("1"), &ValidateOptions::default());
    assert_eq!(e.errors(&ErrorsOptions::default())[0].url, None);
    assert!(!e.display(true, false).contains("errors.pydantic.dev"));
}

#[test]
fn perl_input_values_and_types() {
    let input = j(r#"[null, true, false, 1, 1.5, "it's", [1], {"a": 1, "b c": null}]"#);
    let e = perl_error(
        r#"{"type": "tuple", "items_schema": [{"type": "str"}, {"type": "str"}, {"type": "str"}, {"type": "str"}, {"type": "str"}, {"type": "int"}, {"type": "str"}, {"type": "str"}]}"#,
        &input,
        &ValidateOptions::default(),
    );
    let text = e.display(false, false);
    for expected in [
        "input_value=undef, input_type=Undef]",
        "input_value=!!1, input_type=Bool]",
        "input_value=!!0, input_type=Bool]",
        "input_value=1, input_type=Int]",
        "input_value=1.5, input_type=Num]",
        r"input_value='it\'s', input_type=Str]",
        "input_value=[1], input_type=ArrayRef]",
        "input_value={a => 1, 'b c' => undef}, input_type=HashRef]",
    ] {
        assert!(text.contains(expected), "{expected} in\n{text}");
    }
}

#[test]
fn perl_names_of_values() {
    let cases = [
        ("null", "undef", "Undef"),
        (
            "123456789012345678901234567890",
            "123456789012345678901234567890",
            "Int",
        ),
        (r#""a\nb""#, r#""a\nb""#, "Str"),
        (r#""caf\u00e9 \\ \"""#, r#"'café \\ "'"#, "Str"),
        ("[]", "[]", "ArrayRef"),
        ("{}", "{}", "HashRef"),
    ];
    for (json, repr, type_name) in cases {
        let value = j(json);
        assert_eq!(
            (value.perl_repr(), value.perl_type_name()),
            (repr.to_owned(), type_name),
            "{json}"
        );
    }
}

#[test]
fn error_types_take_perl_codes() {
    use perldantic_core::ErrorType;
    for (perl, pydantic) in [
        ("hash_type", "dict_type"),
        ("array_type", "list_type"),
        ("undef_required", "none_required"),
        ("number_type", "float_type"),
        ("number_parsing", "float_parsing"),
        ("int_from_fraction", "int_from_float"),
        ("duration_type", "time_delta_type"),
    ] {
        let error = ErrorType::new(perl, None).unwrap();
        assert_eq!(error.type_string(), pydantic);
        assert_eq!(error.type_string_for(InputType::Perl), perl);
    }
}

#[test]
fn perl_arrays_are_strict_tuples() {
    let v = validator(r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "str"}]}"#);
    let array = j(r#"[1, "a"]"#);
    assert_eq!(
        v.validate_value_as(&array, InputType::Perl, &strict())
            .unwrap(),
        Value::Tuple(vec![Value::Int(1), Value::from("a")])
    );
    assert_eq!(
        errors_with(&v, &array, InputType::Python, &strict()),
        one("tuple_type", "Input should be a valid tuple")
    );
    // Only arrays: strict still refuses other kinds of input.
    assert_eq!(
        errors_with(&v, &j(r#"{"a": 1}"#), InputType::Perl, &strict()),
        one("array_type", "Input should be an array reference")
    );
}

#[test]
fn validate_value_is_python_input() {
    let v = validator(r#"{"type": "none"}"#);
    let Err(ValidateError::Validation(e)) = v.validate_value(&j("1"), &ValidateOptions::default())
    else {
        panic!("expected an error");
    };
    assert_eq!(e.input_type(), InputType::Python);
    assert_eq!(InputType::try_from("perl"), Ok(InputType::Perl));
    assert_eq!(InputType::Perl.as_str(), "perl");
}

#[test]
fn perl_arrays_are_sets_and_tuples() {
    let set = validator(r#"{"type": "set", "items_schema": {"type": "int"}, "strict": true}"#);
    assert_eq!(
        set.validate_value_as(
            &j("[1, 1, 2]"),
            InputType::Perl,
            &ValidateOptions::default()
        )
        .unwrap(),
        Value::Set(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        errors_with(&set, &j("[1]"), InputType::Python, &strict()),
        one("set_type", "Input should be a valid set")
    );

    // serializers take Perl arrays for tuples and sets without warnings
    let perl = SerializeOptions {
        input_type: InputType::Perl,
        ..SerializeOptions::default()
    };
    for schema in [
        r#"{"type": "set"}"#,
        r#"{"type": "frozenset"}"#,
        r#"{"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 0}"#,
    ] {
        let s = SchemaSerializer::new(&j(schema), None).unwrap();
        let out = s
            .to_json(&j("[1, 2]"), &perl, &JsonOptions::default())
            .unwrap();
        assert_eq!(
            (out.output.as_str(), out.warning),
            ("[1,2]", None),
            "{schema}"
        );
        let python = s
            .to_json(
                &j("[1, 2]"),
                &SerializeOptions::default(),
                &JsonOptions::default(),
            )
            .unwrap();
        assert!(
            python.warning.is_some(),
            "{schema}: Python data warns as upstream"
        );
    }
}
