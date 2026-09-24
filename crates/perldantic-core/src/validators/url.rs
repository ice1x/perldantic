//! `url` and `multi-host-url` schemas. Port of upstream `validators/url.rs`; values are
//! `crate::url::Url` / `MultiHostUrl` rather than Python objects.

use std::cell::RefCell;
use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;
use std::sync::Arc;

use url::{ParseError, SyntaxViolation};

use crate::build_tools::{SchemaDict, is_strict, schema_err, schema_or_config};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValResult};
use crate::input::Input;
use crate::url::{MultiHostUrl, Url, scheme_is_special};
use crate::value::{Dict, Value};

use super::literal::{expected_name, expected_repr};
use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator};

type AllowedSchemes = Option<(HashSet<String>, String)>;

/// The constraints both URL validators take.
#[derive(Debug, Clone)]
struct UrlConstraints {
    strict: bool,
    max_length: Option<usize>,
    allowed_schemes: AllowedSchemes,
    host_required: bool,
    default_host: Option<String>,
    default_port: Option<u16>,
    default_path: Option<String>,
    name: String,
    preserve_empty_path: bool,
}

impl UrlConstraints {
    fn from_schema(schema: &Dict, config: Option<&Dict>, name: &'static str) -> CoreResult<Self> {
        let (allowed_schemes, name) = get_allowed_schemes(schema, name)?;
        let default_port: Option<usize> = schema.get_as("default_port")?;
        let default_port = match default_port.map(u16::try_from) {
            None => None,
            Some(Ok(port)) => Some(port),
            Some(Err(_)) => return schema_err!("'default_port' must be a valid port number"),
        };
        Ok(Self {
            strict: is_strict(schema, config)?,
            max_length: schema.get_as("max_length")?,
            allowed_schemes,
            host_required: schema.get_as("host_required")?.unwrap_or(false),
            default_host: schema.get_as("default_host")?,
            default_port,
            default_path: schema.get_as("default_path")?,
            name,
            preserve_empty_path: schema_or_config(
                schema,
                config,
                "preserve_empty_path",
                "url_preserve_empty_path",
            )?
            .unwrap_or(false),
        })
    }

    fn check_length(&self, input: &(impl Input + ?Sized), length: usize) -> ValResult<()> {
        if let Some(max_length) = self.max_length
            && length > max_length
        {
            return Err(ValError::new(
                ErrorType::UrlTooLong {
                    max_length,
                    context: None,
                },
                input,
            ));
        }
        Ok(())
    }

    fn check_scheme(&self, scheme: &str, input: &(impl Input + ?Sized)) -> ValResult<()> {
        if let Some((allowed_schemes, expected_schemes_repr)) = &self.allowed_schemes
            && !allowed_schemes.contains(scheme)
        {
            return Err(ValError::new(
                ErrorType::UrlScheme {
                    expected_schemes: expected_schemes_repr.clone(),
                    context: None,
                },
                input,
            ));
        }
        Ok(())
    }

    /// Check `host_required` and fill in `default_host`, `default_port` and `default_path`.
    fn check_sub_defaults(
        &self,
        url: &mut url::Url,
        input: &(impl Input + ?Sized),
    ) -> ValResult<()> {
        check_sub_defaults(
            url,
            self.host_required,
            self.default_host.as_ref(),
            self.default_port,
            self.default_path.as_ref(),
        )
        .map_err(|error_type| ValError::new(error_type, input))
    }
}

#[derive(Debug, Clone)]
pub struct UrlValidator {
    constraints: UrlConstraints,
}

impl BuildValidator for UrlValidator {
    const EXPECTED_TYPE: &'static str = "url";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let constraints = UrlConstraints::from_schema(schema, config, Self::EXPECTED_TYPE)?;
        Ok(Arc::new(CombinedValidator::Url(Box::new(Self {
            constraints,
        }))))
    }
}

impl Validator for UrlValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let c = &self.constraints;
        let mut url = self.get_url(input, state.strict_or(c.strict))?;
        c.check_scheme(url.scheme(), input)?;
        c.check_sub_defaults(url.url_mut(), input)?;
        // Lax rather than strict to preserve V2.4 semantic that str wins over url in union
        state.floor_exactness(Exactness::Lax);
        Ok(Value::Url(Box::new(url)))
    }

    fn get_name(&self) -> &str {
        &self.constraints.name
    }
}

impl UrlValidator {
    fn get_url(&self, input: &(impl Input + ?Sized), strict: bool) -> ValResult<Url> {
        let c = &self.constraints;
        match input.as_value() {
            // any syntax errors were fixed by the first validation, strict or not
            Some(Value::Url(url)) => {
                c.check_length(input, url.as_str().len())?;
                return Ok((**url).clone());
            }
            Some(Value::MultiHostUrl(url)) => return self.parse(input, &url.as_str(), strict),
            _ => {}
        }
        match input.validate_str(strict, false) {
            Ok(either_str) => self.parse(input, &either_str.into_inner().as_cow(), strict),
            Err(_) => Err(ValError::new(ErrorTypeDefaults::UrlType, input)),
        }
    }

    fn parse(&self, input: &(impl Input + ?Sized), url_str: &str, strict: bool) -> ValResult<Url> {
        let c = &self.constraints;
        c.check_length(input, url_str.len())?;
        let url = parse_url(url_str, input, strict)?;
        let path_is_empty = need_to_preserve_empty_path(&url, url_str, c.preserve_empty_path);
        Ok(Url::new(url, path_is_empty))
    }
}

#[derive(Debug, Clone)]
pub struct MultiHostUrlValidator {
    constraints: UrlConstraints,
}

impl BuildValidator for MultiHostUrlValidator {
    const EXPECTED_TYPE: &'static str = "multi-host-url";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let constraints = UrlConstraints::from_schema(schema, config, Self::EXPECTED_TYPE)?;
        if let Some(default_host) = &constraints.default_host
            && default_host.contains(',')
        {
            return schema_err!("default_host cannot contain a comma, see pydantic-core#326");
        }
        Ok(Arc::new(CombinedValidator::MultiHostUrl(Box::new(Self {
            constraints,
        }))))
    }
}

impl Validator for MultiHostUrlValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let c = &self.constraints;
        let mut url = self.get_url(input, state.strict_or(c.strict))?;
        c.check_scheme(url.scheme(), input)?;
        c.check_sub_defaults(url.mut_lib_url(), input)?;
        // Lax rather than strict to preserve V2.4 semantic that str wins over url in union
        state.floor_exactness(Exactness::Lax);
        Ok(Value::MultiHostUrl(Box::new(url)))
    }

    fn get_name(&self) -> &str {
        &self.constraints.name
    }
}

impl MultiHostUrlValidator {
    fn get_url(&self, input: &(impl Input + ?Sized), strict: bool) -> ValResult<MultiHostUrl> {
        let c = &self.constraints;
        // any syntax errors were fixed by the first validation, strict or not
        match input.as_value() {
            Some(Value::MultiHostUrl(url)) => {
                c.check_length(input, url.as_str().len())?;
                Ok((**url).clone())
            }
            Some(Value::Url(url)) => {
                c.check_length(input, url.as_str().len())?;
                Ok(MultiHostUrl::new((**url).clone(), None))
            }
            _ => match input.validate_str(strict, false) {
                Ok(either_str) => {
                    let either_str = either_str.into_inner();
                    let url_str = either_str.as_cow();
                    c.check_length(input, url_str.len())?;
                    parse_multihost_url(&url_str, input, strict, c.preserve_empty_path)
                }
                Err(_) => Err(ValError::new(ErrorTypeDefaults::UrlType, input)),
            },
        }
    }
}

/// Parse URL text as the simple `url` validator does (upstream `PyUrl::py_new`).
pub(crate) fn parse_simple_url(text: &str, preserve_empty_path: bool) -> ValResult<Url> {
    let url = parse_url(text, text, false)?;
    let path_is_empty = need_to_preserve_empty_path(&url, text, preserve_empty_path);
    Ok(Url::new(url, path_is_empty))
}

/// Parse URL text as the simple `multi-host-url` validator does (upstream
/// `PyMultiHostUrl::py_new`).
pub(crate) fn parse_simple_multi_host_url(
    text: &str,
    preserve_empty_path: bool,
) -> ValResult<MultiHostUrl> {
    parse_multihost_url(text, text, false, preserve_empty_path)
}

fn parsing_error(error: impl ToString, input: &(impl Input + ?Sized)) -> ValError {
    ValError::new(
        ErrorType::UrlParsing {
            error: error.to_string(),
            context: None,
        },
        input,
    )
}

fn parse_multihost_url(
    url_str: &str,
    input: &(impl Input + ?Sized),
    strict: bool,
    preserve_empty_path: bool,
) -> ValResult<MultiHostUrl> {
    if url_str.is_empty() {
        return Err(parsing_error(EMPTY_INPUT, input));
    }
    let mut chars = PositionedPeekable::new(url_str);

    // consume whitespace, taken from `with_log`
    // https://github.com/servo/rust-url/blob/v2.3.1/url/src/parser.rs#L213-L226
    while let Some(&c) = chars.peek() {
        if c > ' ' {
            break;
        }
        chars.next();
    }

    // consume the url scheme, some logic from `parse_scheme`
    // https://github.com/servo/rust-url/blob/v2.3.1/url/src/parser.rs#L387-L411
    let scheme_start = chars.position;
    let scheme_end = loop {
        match chars.next() {
            Some('a'..='z' | 'A'..='Z' | '0'..='9' | '+' | '-' | '.') => continue,
            Some(':') => {
                // require the scheme to be non-empty
                let scheme_end = chars.position - ':'.len_utf8();
                if scheme_end > scheme_start {
                    break scheme_end;
                }
            }
            _ => {}
        }
        return Err(parsing_error(ParseError::RelativeUrlWithoutBase, input));
    };
    let scheme = url_str[scheme_start..scheme_end].to_ascii_lowercase();

    // consume the double slash, or any number of slashes, including backslashes, taken from
    // `parse_with_scheme`
    // https://github.com/servo/rust-url/blob/v2.3.1/url/src/parser.rs#L413-L456
    while let Some(&('/' | '\\')) = chars.peek() {
        chars.next();
    }
    let prefix = &url_str[..chars.position];

    // process host and port, splitting based on `,`, some logic taken from `parse_host`
    // https://github.com/servo/rust-url/blob/v2.3.1/url/src/parser.rs#L971-L1026
    let mut hosts: Vec<&str> = Vec::with_capacity(3);
    let mut start = chars.position;
    while let Some(c) = chars.next() {
        match c {
            '\\' if scheme_is_special(&scheme) => break,
            '/' | '?' | '#' => break,
            ',' => {
                // minus 1 because we know that the last char was a `,` with length 1
                let end = chars.position - ','.len_utf8();
                if start == end {
                    return Err(parsing_error(ParseError::EmptyHost, input));
                }
                hosts.push(&url_str[start..end]);
                start = chars.position;
            }
            _ => (),
        }
    }
    // with just one host, for consistent behaviour, we parse the URL the same as with multiple
    // hosts
    let reconstructed_url = format!("{prefix}{}", &url_str[start..]);
    let ref_url = parse_url(&reconstructed_url, input, strict)?;
    let path_is_empty =
        need_to_preserve_empty_path(&ref_url, &reconstructed_url, preserve_empty_path);
    let ref_url = Url::new(ref_url, path_is_empty);

    if hosts.is_empty() {
        // if there's no one host (e.g. no `,`), we allow it to be empty to allow for default
        // hosts
        Ok(MultiHostUrl::new(ref_url, None))
    } else {
        // with more than one host, none of them can be empty
        if !ref_url.url().has_host() {
            return Err(parsing_error(ParseError::EmptyHost, input));
        }
        let extra_urls: Vec<url::Url> = hosts
            .iter()
            .map(|host| parse_url(&format!("{prefix}{host}"), input, strict))
            .collect::<ValResult<_>>()?;
        if extra_urls.iter().any(|url| !url.has_host()) {
            return Err(parsing_error(ParseError::EmptyHost, input));
        }
        Ok(MultiHostUrl::new(ref_url, Some(extra_urls)))
    }
}

fn parse_url(url_str: &str, input: &(impl Input + ?Sized), strict: bool) -> ValResult<url::Url> {
    if url_str.is_empty() {
        return Err(parsing_error(EMPTY_INPUT, input));
    }

    // we could build a vec of syntax violations and return them all, but that seems like
    // overkill and unlike other parser style validators
    let vios = RefCell::new(None);
    let record = |v| match v {
        // telling users about credentials in URLs doesn't really make sense in this context
        SyntaxViolation::EmbeddedCredentials => (),
        _ => *vios.borrow_mut() = Some(v),
    };
    let url = url::Url::options()
        // in strict mode a syntax violation is an error
        .syntax_violation_callback(strict.then_some(&record))
        .parse(url_str)
        .map_err(|e| parsing_error(e, input))?;

    if let Some(vio) = vios.into_inner() {
        return Err(ValError::new(
            ErrorType::UrlSyntaxViolation {
                error: vio.description().into(),
                context: None,
            },
            input,
        ));
    }
    Ok(url)
}

/// Whether the path got normalised to `/` while the original string had an empty path.
fn need_to_preserve_empty_path(url: &url::Url, url_str: &str, preserve_empty_path: bool) -> bool {
    if !preserve_empty_path || url.path() != "/" || !scheme_is_special(url.scheme()) {
        // non-special schemes don't normalise the path
        return false;
    }
    // find the scheme marker in the original input
    let (_, input_without_scheme) = url_str.split_once(':').expect("url has a scheme");
    // strip any leading / (part of the authority marker); URL normalises any number of them
    let input_without_scheme = input_without_scheme.trim_start_matches('/');
    // the path starts at the first /, or the query or fragment follow an empty path
    for c in input_without_scheme.chars() {
        match c {
            '/' => return false,
            '?' | '#' => return true,
            _ => (),
        }
    }
    // reached the end of the string without finding a path, so it's empty
    true
}

/// Check `host_required` and substitute `default_host`, `default_port` & `default_path` if they
/// aren't set.
fn check_sub_defaults(
    url: &mut url::Url,
    host_required: bool,
    default_host: Option<&String>,
    default_port: Option<u16>,
    default_path: Option<&String>,
) -> Result<(), ErrorType> {
    let map_parse_err = |e: ParseError| ErrorType::UrlParsing {
        error: e.to_string(),
        context: None,
    };
    if !url.has_host() {
        if let Some(default_host) = default_host {
            url.set_host(Some(default_host)).map_err(map_parse_err)?;
        } else if host_required {
            return Err(map_parse_err(ParseError::EmptyHost));
        }
    }
    if let Some(default_port) = default_port
        && url.port().is_none()
    {
        url.set_port(Some(default_port))
            .map_err(|()| map_parse_err(ParseError::EmptyHost))?;
    }
    if let Some(default_path) = default_path {
        let path = url.path();
        if path.is_empty() || path == "/" {
            url.set_path(default_path);
        }
    }
    Ok(())
}

fn get_allowed_schemes(schema: &Dict, name: &'static str) -> CoreResult<(AllowedSchemes, String)> {
    let Some(list) = schema.get_as::<Vec<Value>>("allowed_schemes")? else {
        return Ok((None, name.to_owned()));
    };
    if list.is_empty() {
        return schema_err!("`allowed_schemes` should have length > 0");
    }
    let mut expected = HashSet::new();
    let mut repr_args = Vec::new();
    for item in &list {
        let Value::Str(scheme) = item else {
            return schema_err!("`allowed_schemes` must be a list of strings");
        };
        repr_args.push(item.repr());
        expected.insert(scheme.clone());
    }
    let repr = expected_repr(&repr_args);
    Ok((Some((expected, repr)), expected_name(&repr_args, name)))
}

struct PositionedPeekable<'a> {
    peekable: Peekable<Chars<'a>>,
    position: usize,
}

impl<'a> PositionedPeekable<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            peekable: input.chars().peekable(),
            position: 0,
        }
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peekable.next();
        if let Some(c) = c {
            self.position += c.len_utf8();
        }
        c
    }

    fn peek(&mut self) -> Option<&char> {
        self.peekable.peek()
    }
}

const EMPTY_INPUT: &str = "input is empty";
