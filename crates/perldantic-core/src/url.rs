//! URLs as pydantic-core has them. Port of upstream `url.rs` without the Python classes: `Url`
//! and `MultiHostUrl` keep the parsed `url::Url`s and give the same accessors, text and
//! comparisons as `pydantic_core.Url` and `pydantic_core.MultiHostUrl`.

use std::borrow::Cow;
use std::cmp::Ordering;

use idna::punycode::decode_to_string;

use crate::core_error::{CoreError, CoreResult};
use crate::errors::ValError;

/// Upstream `PyUrl`.
#[derive(Debug, Clone)]
pub struct Url {
    lib_url: url::Url,
    /// Treat the path as empty when it is `/`: the `url` crate always normalises an empty path
    /// of a special scheme to `/`, but `preserve_empty_path` keeps it empty.
    path_is_empty: bool,
}

impl Url {
    pub fn new(lib_url: url::Url, path_is_empty: bool) -> Self {
        Self {
            lib_url,
            path_is_empty,
        }
    }

    /// Parse URL text as `Url(text, preserve_empty_path=...)` does.
    pub fn parse(text: &str, preserve_empty_path: bool) -> CoreResult<Self> {
        crate::validators::url::parse_simple_url(text, preserve_empty_path).map_err(invalid_url)
    }

    pub fn url(&self) -> &url::Url {
        &self.lib_url
    }

    pub(crate) fn url_mut(&mut self) -> &mut url::Url {
        &mut self.lib_url
    }

    /// The URL text: `str(url)`.
    pub fn as_str(&self) -> Cow<'_, str> {
        if self.path_is_empty {
            Cow::Owned(serialize_url_without_path_slash(&self.lib_url))
        } else {
            Cow::Borrowed(self.lib_url.as_str())
        }
    }

    /// `repr(url)`.
    pub fn repr(&self) -> String {
        format!("Url('{}')", self.as_str())
    }

    pub fn scheme(&self) -> &str {
        self.lib_url.scheme()
    }

    pub fn username(&self) -> Option<&str> {
        match self.lib_url.username() {
            "" => None,
            user => Some(user),
        }
    }

    pub fn password(&self) -> Option<&str> {
        self.lib_url.password()
    }

    pub fn host(&self) -> Option<&str> {
        self.lib_url.host_str()
    }

    /// The host with punycode decoded where appropriate.
    pub fn unicode_host(&self) -> Option<String> {
        match self.lib_url.host() {
            Some(url::Host::Domain(domain)) if is_punycode_domain(&self.lib_url, domain) => {
                decode_punycode(domain)
            }
            _ => self.lib_url.host_str().map(ToString::to_string),
        }
    }

    /// The port, or the scheme's default port.
    pub fn port(&self) -> Option<u16> {
        self.lib_url.port_or_known_default()
    }

    pub fn path(&self) -> Option<&str> {
        match self.lib_url.path() {
            "" => None,
            "/" if self.path_is_empty => None,
            path => Some(path),
        }
    }

    pub fn query(&self) -> Option<&str> {
        self.lib_url.query()
    }

    /// The query as decoded `(key, value)` pairs.
    pub fn query_params(&self) -> Vec<(String, String)> {
        self.lib_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.lib_url.fragment()
    }

    /// The URL text with punycode decoded where appropriate.
    pub fn unicode_string(&self) -> String {
        unicode_url(&self.as_str(), &self.lib_url).into_owned()
    }
}

impl PartialEq for Url {
    /// Upstream compares the parsed URLs, ignoring `path_is_empty`.
    fn eq(&self, other: &Self) -> bool {
        self.lib_url == other.lib_url
    }
}

impl PartialOrd for Url {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.lib_url.cmp(&other.lib_url))
    }
}

/// One host of a multi-host URL, as `MultiHostUrl.hosts()` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlHost {
    pub username: Option<String>,
    pub password: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Upstream `PyMultiHostUrl`: the last host is in `ref_url`, the others in `extra_urls`.
#[derive(Debug, Clone)]
pub struct MultiHostUrl {
    ref_url: Url,
    extra_urls: Option<Vec<url::Url>>,
}

impl MultiHostUrl {
    pub fn new(ref_url: Url, extra_urls: Option<Vec<url::Url>>) -> Self {
        Self {
            ref_url,
            extra_urls,
        }
    }

    /// Parse URL text as `MultiHostUrl(text, preserve_empty_path=...)` does.
    pub fn parse(text: &str, preserve_empty_path: bool) -> CoreResult<Self> {
        crate::validators::url::parse_simple_multi_host_url(text, preserve_empty_path)
            .map_err(invalid_url)
    }

    pub(crate) fn mut_lib_url(&mut self) -> &mut url::Url {
        &mut self.ref_url.lib_url
    }

    pub fn scheme(&self) -> &str {
        self.ref_url.scheme()
    }

    pub fn hosts(&self) -> Vec<UrlHost> {
        if let Some(extra_urls) = &self.extra_urls {
            let mut hosts: Vec<UrlHost> = extra_urls.iter().map(host_parts).collect();
            hosts.push(host_parts(&self.ref_url.lib_url));
            hosts
        } else if self.ref_url.lib_url.has_host() {
            vec![host_parts(&self.ref_url.lib_url)]
        } else {
            vec![]
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.ref_url.path()
    }

    pub fn query(&self) -> Option<&str> {
        self.ref_url.query()
    }

    pub fn query_params(&self) -> Vec<(String, String)> {
        self.ref_url.query_params()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.ref_url.fragment()
    }

    /// The URL text with the extra hosts put back, and punycode decoded where appropriate.
    pub fn unicode_string(&self) -> String {
        self.with_extra_hosts(self.ref_url.unicode_string(), |url| {
            unicode_url(url.as_str(), url).into_owned()
        })
    }

    /// The URL text: `str(url)`.
    pub fn as_str(&self) -> Cow<'_, str> {
        match &self.extra_urls {
            Some(_) => Cow::Owned(
                self.with_extra_hosts(self.ref_url.as_str().into_owned(), |url| {
                    url.as_str().to_owned()
                }),
            ),
            None => self.ref_url.as_str(),
        }
    }

    /// `repr(url)`.
    pub fn repr(&self) -> String {
        format!("MultiHostUrl('{}')", self.as_str())
    }

    /// Insert the hosts of the extra URLs (as `text_of` writes them) before the host of the
    /// reference URL text `full_url`.
    fn with_extra_hosts(
        &self,
        mut full_url: String,
        text_of: impl Fn(&url::Url) -> String,
    ) -> String {
        let Some(extra_urls) = &self.extra_urls else {
            return full_url;
        };
        let scheme = self.ref_url.lib_url.scheme();
        let host_offset = scheme.len() + 3;
        let mut extra_hosts = String::new();
        // special URLs have had a trailing slash added, non-special ones have not
        let sub = usize::from(scheme_is_special(scheme));
        for url in extra_urls {
            let text = text_of(url);
            extra_hosts.push_str(&text[host_offset..text.len() - sub]);
            extra_hosts.push(',');
        }
        full_url.insert_str(host_offset, &extra_hosts);
        full_url
    }
}

impl PartialEq for MultiHostUrl {
    /// Upstream compares the unicode strings.
    fn eq(&self, other: &Self) -> bool {
        self.unicode_string() == other.unicode_string()
    }
}

impl PartialOrd for MultiHostUrl {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.unicode_string().cmp(&other.unicode_string()))
    }
}

fn invalid_url(err: ValError) -> CoreError {
    let detail = match err {
        ValError::LineErrors(errors) => errors
            .first()
            .and_then(|e| {
                e.error_type
                    .render_message(crate::input::InputType::Python)
                    .ok()
            })
            .unwrap_or_default(),
        other => format!("{other:?}"),
    };
    CoreError::Value(detail)
}

fn host_parts(lib_url: &url::Url) -> UrlHost {
    UrlHost {
        username: Some(lib_url.username())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned),
        password: lib_url.password().map(ToOwned::to_owned),
        host: lib_url.host_str().map(ToOwned::to_owned),
        port: lib_url.port_or_known_default(),
    }
}

fn unicode_url<'s>(serialized: &'s str, lib_url: &url::Url) -> Cow<'s, str> {
    match lib_url.host() {
        Some(url::Host::Domain(domain)) if is_punycode_domain(lib_url, domain) => {
            let mut s = serialized.to_string();
            if let Some(decoded) = decode_punycode(domain) {
                // replace the range containing the punycode domain with the decoded domain
                let start = lib_url.scheme().len() + 3;
                s.replace_range(start..start + domain.len(), &decoded);
            }
            Cow::Owned(s)
        }
        _ => Cow::Borrowed(serialized),
    }
}

fn decode_punycode(domain: &str) -> Option<String> {
    let mut result = String::with_capacity(domain.len());
    for chunk in domain.split('.') {
        if let Some(stripped) = chunk.strip_prefix(PUNYCODE_PREFIX) {
            result.push_str(&decode_to_string(stripped)?);
        } else {
            result.push_str(chunk);
        }
        result.push('.');
    }
    result.pop();
    Some(result)
}

const PUNYCODE_PREFIX: &str = "xn--";

fn is_punycode_domain(lib_url: &url::Url, domain: &str) -> bool {
    scheme_is_special(lib_url.scheme())
        && domain
            .split('.')
            .any(|part| part.starts_with(PUNYCODE_PREFIX))
}

/// Based on the `url` crate's parser: schemes with special parsing rules.
pub fn scheme_is_special(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

fn serialize_url_without_path_slash(url: &url::Url) -> String {
    // The path is `/` (checked when `path_is_empty` is set); the text before it ends at the
    // authority and the text after it is the query and fragment.
    let s = url.as_str();
    let path_start = url::Position::BeforePath;
    let after_path = url::Position::AfterPath;
    debug_assert_eq!(url.path(), "/");
    format!("{}{}", &url[..path_start], &s[url[..after_path].len()..])
}
