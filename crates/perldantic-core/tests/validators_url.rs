//! `url` and `multi-host-url` beyond the recorded cases: the accessors of `Url` and
//! `MultiHostUrl` (expected values taken from pydantic-core 2.49), their Python forms,
//! serialization and the JSON Schema.

use perldantic_core::{
    ErrorsOptions, JsonOptions, JsonSchemaOptions, MultiHostUrl, SchemaSerializer, SchemaValidator,
    SerMode, SerializeOptions, Url, UrlHost, ValidateError, ValidateOptions, Value,
    generate_json_schema,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn url(text: &str) -> Url {
    Url::parse(text, false).unwrap()
}

#[test]
fn url_accessors_match_pydantic() {
    let u = url("https://user:pa%20ss@xn--mnchen-3ya.de:8443/a/b?x=1&y=a%20b&x=2#frag");
    assert_eq!(u.scheme(), "https");
    assert_eq!(u.username(), Some("user"));
    assert_eq!(u.password(), Some("pa%20ss"));
    assert_eq!(u.host(), Some("xn--mnchen-3ya.de"));
    assert_eq!(u.unicode_host().as_deref(), Some("münchen.de"));
    assert_eq!(u.port(), Some(8443));
    assert_eq!(u.path(), Some("/a/b"));
    assert_eq!(u.query(), Some("x=1&y=a%20b&x=2"));
    assert_eq!(
        u.query_params(),
        [("x", "1"), ("y", "a b"), ("x", "2")].map(|(k, v)| (k.to_owned(), v.to_owned()))
    );
    assert_eq!(u.fragment(), Some("frag"));
    // upstream replaces the host at a fixed offset, ignoring credentials
    assert_eq!(
        u.unicode_string(),
        "https://münchen.demnchen-3ya.de:8443/a/b?x=1&y=a%20b&x=2#frag"
    );

    let ftp = url("ftp://example.com");
    assert_eq!(ftp.as_str(), "ftp://example.com/");
    assert_eq!((ftp.username(), ftp.password()), (None, None));
    assert_eq!(
        (ftp.port(), ftp.path(), ftp.query()),
        (Some(21), Some("/"), None)
    );
}

#[test]
fn empty_paths_can_be_preserved() {
    assert_eq!(
        url("https://example.com?q=1").as_str(),
        "https://example.com/?q=1"
    );
    let kept = Url::parse("https://example.com?q=1", true).unwrap();
    assert_eq!(kept.as_str(), "https://example.com?q=1");
    assert_eq!(kept.path(), None);
    assert_eq!(kept.repr(), "Url('https://example.com?q=1')");
    // compared by the parsed URL, as upstream does
    assert_eq!(kept, url("https://example.com?q=1"));
}

#[test]
fn multi_host_urls_list_their_hosts() {
    let m = MultiHostUrl::parse("postgres://u:p@h1:5432,h2,h3:1/db?x=1", false).unwrap();
    assert_eq!(m.as_str(), "postgres://u:p@h1:5432,h2,h3:1/db?x=1");
    assert_eq!(
        m.repr(),
        "MultiHostUrl('postgres://u:p@h1:5432,h2,h3:1/db?x=1')"
    );
    assert_eq!(m.scheme(), "postgres");
    let host = |username: Option<&str>, password: Option<&str>, host: &str, port| UrlHost {
        username: username.map(Into::into),
        password: password.map(Into::into),
        host: Some(host.into()),
        port,
    };
    assert_eq!(
        m.hosts(),
        [
            host(Some("u"), Some("p"), "h1", Some(5432)),
            host(None, None, "h2", None),
            host(None, None, "h3", Some(1)),
        ]
    );
    assert_eq!(
        (m.path(), m.query(), m.fragment()),
        (Some("/db"), Some("x=1"), None)
    );

    let idn = MultiHostUrl::parse("https://xn--mnchen-3ya.de,b.com/p", false).unwrap();
    assert_eq!(idn.unicode_string(), "https://münchen.de,b.com/p");
    assert_eq!(idn.hosts()[1].port, Some(443));
}

#[test]
fn urls_compare_like_pydantic() {
    assert!(url("https://a.com") < url("https://b.com"));
    assert_eq!(url("https://a.com"), url("https://a.com/"));
    let redis = |text| MultiHostUrl::parse(text, false).unwrap();
    assert_eq!(redis("redis://a,b"), redis("redis://a,b"));
    assert!(redis("redis://a,b") < redis("redis://b,a"));
}

#[test]
fn invalid_text_is_an_error() {
    let err = Url::parse("nope", false).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Input should be a valid URL, relative URL without a base"
    );
    assert!(MultiHostUrl::parse("redis://a,,b", false).is_err());
}

#[test]
fn values_carry_python_forms() {
    let value = Value::Url(Box::new(url("https://example.com")));
    assert_eq!(value.repr(), "Url('https://example.com/')");
    assert_eq!(value.py_str(), "https://example.com/");
    assert_eq!(value.type_name(), "Url");
    let v = SchemaValidator::new(&j(r#"{"type": "int"}"#), None).unwrap();
    let Err(ValidateError::Validation(e)) = v.validate_value(&value, &ValidateOptions::default())
    else {
        panic!("expected a validation error");
    };
    assert_eq!(e.errors(&ErrorsOptions::default())[0].type_, "int_type");
    assert!(
        e.to_string()
            .contains("input_value=Url('https://example.com/'), input_type=Url"),
        "{e}"
    );
}

#[test]
fn a_url_is_accepted_by_multi_host_url_and_back() {
    let multi = SchemaValidator::new(&j(r#"{"type": "multi-host-url"}"#), None).unwrap();
    let out = multi
        .validate_value(
            &Value::Url(Box::new(url("https://example.com"))),
            &ValidateOptions::default(),
        )
        .unwrap();
    assert_eq!(out.py_str(), "https://example.com/");
    assert_eq!(out.type_name(), "MultiHostUrl");

    let single = SchemaValidator::new(&j(r#"{"type": "url"}"#), None).unwrap();
    let out = single
        .validate_value(&out, &ValidateOptions::default())
        .unwrap();
    assert_eq!(out.type_name(), "Url");
}

#[test]
fn urls_serialize_as_text_in_json_mode() {
    let s = SchemaSerializer::new(&j(r#"{"type": "url"}"#), None).unwrap();
    let value = Value::Url(Box::new(Url::parse("https://example.com", true).unwrap()));
    let json = SerializeOptions {
        mode: SerMode::Json,
        ..SerializeOptions::default()
    };
    assert_eq!(
        s.to_python(&value, &SerializeOptions::default())
            .unwrap()
            .output,
        value
    );
    assert_eq!(
        s.to_python(&value, &json).unwrap().output,
        Value::Str("https://example.com".into())
    );
    let any = SchemaSerializer::new(&j(r#"{"type": "any"}"#), None).unwrap();
    let dict = Value::Dict([(value.clone(), value)].into_iter().collect());
    assert_eq!(
        any.to_json(&dict, &SerializeOptions::default(), &JsonOptions::default())
            .unwrap()
            .output,
        r#"{"https://example.com":"https://example.com"}"#
    );
}

#[test]
fn json_schema_is_a_uri_string() {
    let schema = |core: &str| {
        generate_json_schema(&j(core), None, &JsonSchemaOptions::default())
            .unwrap()
            .schema
    };
    assert_eq!(
        schema(r#"{"type": "url", "max_length": 99}"#),
        j(r#"{"type": "string", "format": "uri", "minLength": 1, "maxLength": 99}"#)
    );
    assert_eq!(
        schema(r#"{"type": "multi-host-url"}"#),
        j(r#"{"type": "string", "format": "multi-host-uri", "minLength": 1}"#)
    );
}
