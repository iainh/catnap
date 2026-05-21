//! Trait-first REST clients inspired by Eclipse MicroProfile REST Client.
//!
//! Catnap lets you describe an HTTP API as a Rust trait and derive a small
//! `reqwest`-backed client from that trait. It is intended for service clients
//! where the useful abstraction is the remote resource, not repeated manual
//! request construction.
//!
//! The main entry point is [`rest_client`]. It keeps the annotated trait in
//! place and generates a cloneable client named `<TraitName>Client`.
//!
//! # Making a GET request
//!
//! ```
//! use catnap::{rest_client, Result};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct User {
//!     id: String,
//! }
//!
//! #[rest_client(path = "/users")]
//! trait Users {
//!     #[get("/{id}")]
//!     async fn get_user(&self, #[path] id: &str) -> Result<User>;
//! }
//!
//! # async fn example() -> Result<()> {
//! let client = UsersClient::builder()
//!     .base_url("https://example.com")?
//!     .build()?;
//! let _user = client.get_user("42").await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Defining request bodies
//!
//! An unannotated method argument is treated as the request body. JSON is the
//! default media type when the default `json` feature is enabled.
//!
//! ```
//! use catnap::{rest_client, Result};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize)]
//! struct NewUser {
//!     name: String,
//! }
//!
//! #[derive(Deserialize)]
//! struct User {
//!     id: String,
//!     name: String,
//! }
//!
//! #[rest_client(path = "/users")]
//! trait Users {
//!     #[post("")]
//!     async fn create(&self, user: &NewUser) -> Result<User>;
//! }
//! ```
//!
//! # Paths, query parameters, and headers
//!
//! Use `#[path]`, `#[query]`, and `#[header("Name")]` on method arguments to
//! bind values to the generated request. Bare `#[path]` and
//! `#[query]` infer the HTTP parameter name from the Rust argument name;
//! use `#[path("name")]` or `#[query("name")]` when the remote API
//! name differs.
//!
//! ```
//! use catnap::{rest_client, Result};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct User {
//!     id: String,
//! }
//!
//! #[rest_client(path = "/tenants/{tenant}")]
//! trait Users {
//!     #[get("/users/{id}")]
//!     async fn get_user(
//!         &self,
//!         #[path] tenant: &str,
//!         #[path] id: &str,
//!         #[query] include: &str,
//!         #[header("X-Request-Id")] request_id: &str,
//!     ) -> Result<User>;
//! }
//! ```
//!
//! # Media types
//!
//! Catnap supports JSON by default, plain text responses as [`String`], raw
//! [`Response`] values, status-only `Result<()>` methods, and optional XML with
//! the `xml` feature.
//!
//! ```
//! use catnap::{rest_client, Result};
//!
//! #[rest_client]
//! trait Health {
//!     #[get("/health")]
//!     #[produces("text/plain")]
//!     async fn health(&self) -> Result<String>;
//! }
//! ```
//!
//! # Feature flags
//!
//! The default features are `json`, `basic-auth`, and `tls-rustls`.
//!
//! - `json` enables JSON request and response bodies.
//! - `basic-auth` enables [`RestClientBuilder::basic_auth`].
//! - `tls-rustls` enables HTTPS through reqwest's Rustls backend.
//! - `xml` enables XML request and response bodies.
//!
//! # Logging
//!
//! Catnap emits `tracing` debug events for outgoing requests and incoming
//! responses. Sensitive headers such as `Authorization`, `Proxy-Authorization`,
//! `Cookie`, and `Set-Cookie` are redacted.

pub use catnap_macros::rest_client;
pub use http;

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, PROXY_AUTHORIZATION, SET_COOKIE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::Proxy;
use reqwest::redirect::Policy;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::error::Error as StdError;
use std::fmt::{self, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use tracing::{Level, debug};
use url::Url;

/// Result type returned by generated clients and runtime helpers.
pub type Result<T> = std::result::Result<T, Error>;

#[doc(hidden)]
pub mod __private {
    use super::*;

    const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'/')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'`')
        .add(b'{')
        .add(b'}');

    pub fn encode_path_segment(value: impl Display) -> String {
        utf8_percent_encode(&value.to_string(), PATH_SEGMENT_ENCODE_SET).to_string()
    }
}

/// Error type returned while building or invoking a generated REST client.
#[derive(Debug)]
pub enum Error {
    /// A generated client was built without configuring a base URL.
    MissingBaseUrl,
    /// The configured base URL could not be parsed.
    InvalidBaseUrl(url::ParseError),
    /// A request path could not be joined to the configured base URL.
    InvalidRequestUrl {
        /// The path passed to the runtime request builder.
        path: String,
        /// The URL parse error produced while joining the path.
        source: url::ParseError,
    },
    /// A configured default header name was invalid.
    InvalidHeaderName(String),
    /// A configured default header value was invalid.
    InvalidHeaderValue {
        /// Header name associated with the invalid value.
        name: String,
        /// The invalid header value error.
        source: http::header::InvalidHeaderValue,
    },
    /// A config value could not be parsed.
    InvalidConfigValue {
        /// Config key that supplied the invalid value.
        key: String,
        /// Invalid config value.
        value: String,
        /// Human-readable parse failure.
        message: String,
    },
    /// The selected request or response media type is not supported.
    UnsupportedMediaType {
        /// The operation that required media type support.
        operation: MediaOperation,
        /// The unsupported media type.
        media_type: &'static str,
    },
    /// The underlying `reqwest` request failed.
    Request(reqwest::Error),
    /// JSON response deserialization failed.
    #[cfg(feature = "json")]
    JsonDeserialize(serde_json::Error),
    /// XML response deserialization failed.
    #[cfg(feature = "xml")]
    XmlDeserialize(quick_xml::de::DeError),
    /// XML request serialization failed.
    #[cfg(feature = "xml")]
    XmlSerialize(quick_xml::se::SeError),
    /// A typed request returned an HTTP status `>= 400`.
    ///
    /// Methods returning [`Response`] leave status handling to the caller.
    Http {
        /// The HTTP error status.
        status: StatusCode,
        /// The response body decoded as UTF-8 lossily.
        body: String,
    },
    /// A custom response exception mapper converted an HTTP response to an
    /// application-specific error.
    MappedResponse(Box<dyn StdError + Send + Sync>),
}

impl Error {
    /// Wraps an application error returned by a response exception mapper.
    pub fn mapped_response(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::MappedResponse(Box::new(source))
    }
}

/// Operation that required media type support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOperation {
    /// Serializing a request body.
    RequestSerialization,
    /// Deserializing a response body.
    ResponseDeserialization,
}

impl Display for MediaOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestSerialization => formatter.write_str("request body serialization"),
            Self::ResponseDeserialization => formatter.write_str("response deserialization"),
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBaseUrl => formatter.write_str("base URL is required"),
            Self::InvalidBaseUrl(source) => write!(formatter, "invalid base URL: {source}"),
            Self::InvalidRequestUrl { path, source } => {
                write!(formatter, "invalid request URL path `{path}`: {source}")
            }
            Self::InvalidHeaderName(name) => write!(formatter, "invalid HTTP header name `{name}`"),
            Self::InvalidHeaderValue { name, source } => {
                write!(
                    formatter,
                    "invalid HTTP header value for `{name}`: {source}"
                )
            }
            Self::InvalidConfigValue {
                key,
                value,
                message,
            } => {
                write!(
                    formatter,
                    "invalid config value `{value}` for `{key}`: {message}"
                )
            }
            Self::UnsupportedMediaType {
                operation,
                media_type,
            } => {
                write!(
                    formatter,
                    "unsupported media type `{media_type}` for {operation}"
                )
            }
            Self::Request(source) => write!(formatter, "request failed: {source}"),
            #[cfg(feature = "json")]
            Self::JsonDeserialize(source) => {
                write!(formatter, "JSON deserialization failed: {source}")
            }
            #[cfg(feature = "xml")]
            Self::XmlDeserialize(source) => {
                write!(formatter, "XML deserialization failed: {source}")
            }
            #[cfg(feature = "xml")]
            Self::XmlSerialize(source) => write!(formatter, "XML serialization failed: {source}"),
            Self::Http { status, body } => write!(formatter, "HTTP {status}: {body}"),
            Self::MappedResponse(source) => write!(formatter, "{source}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::InvalidBaseUrl(source) => Some(source),
            Self::InvalidRequestUrl { source, .. } => Some(source),
            Self::InvalidHeaderValue { source, .. } => Some(source),
            Self::Request(source) => Some(source),
            #[cfg(feature = "json")]
            Self::JsonDeserialize(source) => Some(source),
            #[cfg(feature = "xml")]
            Self::XmlDeserialize(source) => Some(source),
            #[cfg(feature = "xml")]
            Self::XmlSerialize(source) => Some(source),
            Self::MappedResponse(source) => Some(source.as_ref()),
            Self::MissingBaseUrl
            | Self::InvalidHeaderName(_)
            | Self::InvalidConfigValue { .. }
            | Self::UnsupportedMediaType { .. }
            | Self::Http { .. } => None,
        }
    }
}

impl From<url::ParseError> for Error {
    fn from(source: url::ParseError) -> Self {
        Self::InvalidBaseUrl(source)
    }
}

impl From<reqwest::Error> for Error {
    fn from(source: reqwest::Error) -> Self {
        Self::Request(source)
    }
}

#[cfg(feature = "json")]
impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Self::JsonDeserialize(source)
    }
}

#[cfg(feature = "xml")]
impl From<quick_xml::de::DeError> for Error {
    fn from(source: quick_xml::de::DeError) -> Self {
        Self::XmlDeserialize(source)
    }
}

#[cfg(feature = "xml")]
impl From<quick_xml::se::SeError> for Error {
    fn from(source: quick_xml::se::SeError) -> Self {
        Self::XmlSerialize(source)
    }
}

/// How repeated query parameters should be encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QueryParamStyle {
    /// `key=value1&key=value2`
    #[default]
    MultiPairs,
    /// `key=value1,value2`
    CommaSeparated,
    /// `key[]=value1&key[]=value2`
    ArrayPairs,
}

impl QueryParamStyle {
    fn parse_config(value: &str) -> Option<Self> {
        match value {
            "MULTI_PAIRS" | "multi_pairs" | "multi-pairs" => Some(Self::MultiPairs),
            "COMMA_SEPARATED" | "comma_separated" | "comma-separated" => Some(Self::CommaSeparated),
            "ARRAY_PAIRS" | "array_pairs" | "array-pairs" => Some(Self::ArrayPairs),
            _ => None,
        }
    }
}

/// Dynamic query parameters flattened into a request query string.
///
/// This is useful for APIs where callers provide arbitrary query names at
/// runtime, similar to `@RestQuery Map<String, List<String>>` in RESTEasy
/// Reactive.
pub trait QueryMap {
    /// Appends the query map to the request using the configured repeated query
    /// parameter style.
    fn append_query_map(&self, request: RequestBuilder) -> RequestBuilder;
}

impl<K, V> QueryMap for BTreeMap<K, Vec<V>>
where
    K: Display,
    V: Display,
{
    fn append_query_map(&self, mut request: RequestBuilder) -> RequestBuilder {
        for (name, values) in self {
            request = request.query_params(&name.to_string(), values);
        }
        request
    }
}

impl<K, V> QueryMap for HashMap<K, Vec<V>>
where
    K: Display + Eq + Hash,
    V: Display,
{
    fn append_query_map(&self, mut request: RequestBuilder) -> RequestBuilder {
        for (name, values) in self {
            request = request.query_params(&name.to_string(), values);
        }
        request
    }
}

impl<T> QueryMap for &T
where
    T: QueryMap + ?Sized,
{
    fn append_query_map(&self, request: RequestBuilder) -> RequestBuilder {
        (*self).append_query_map(request)
    }
}

type ResponseExceptionMapperFn =
    dyn for<'a> Fn(&ResponseExceptionContext<'a>) -> Option<Error> + Send + Sync;

/// Ordered mapper that can turn an HTTP response into an application error.
///
/// Mappers with lower priority values run first. If every custom mapper
/// returns `None`, Catnap's default mapper converts statuses `>= 400` to
/// [`Error::Http`].
#[derive(Clone)]
pub struct ResponseExceptionMapper {
    priority: i32,
    mapper: Arc<ResponseExceptionMapperFn>,
}

impl ResponseExceptionMapper {
    /// Creates a mapper with the given priority.
    pub fn new(
        priority: i32,
        mapper: impl for<'a> Fn(&ResponseExceptionContext<'a>) -> Option<Error> + Send + Sync + 'static,
    ) -> Self {
        Self {
            priority,
            mapper: Arc::new(mapper),
        }
    }

    /// Returns the mapper priority. Lower values run first.
    pub fn priority(&self) -> i32 {
        self.priority
    }

    fn map_response(&self, response: &ResponseExceptionContext<'_>) -> Option<Error> {
        (self.mapper)(response)
    }
}

impl std::fmt::Debug for ResponseExceptionMapper {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseExceptionMapper")
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

/// Buffered HTTP response data available to response exception mappers.
pub struct ResponseExceptionContext<'a> {
    status: StatusCode,
    headers: &'a HeaderMap,
    body: &'a [u8],
}

impl ResponseExceptionContext<'_> {
    /// Returns the HTTP response status.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the HTTP response headers.
    pub fn headers(&self) -> &HeaderMap {
        self.headers
    }

    /// Returns the raw buffered response body.
    pub fn body(&self) -> &[u8] {
        self.body
    }

    /// Decodes the buffered body as UTF-8, replacing invalid bytes.
    pub fn body_text_lossy(&self) -> String {
        String::from_utf8_lossy(self.body).into_owned()
    }
}

/// Builder used by generated clients.
///
/// Every `#[rest_client]` trait generates a client with a `builder()` method
/// that returns `RestClientBuilder<GeneratedClient>`.
#[derive(Debug, Clone)]
pub struct RestClientBuilder<T> {
    config: RestClientConfig,
    marker: PhantomData<T>,
}

impl<T> Default for RestClientBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RestClientBuilder<T> {
    /// Creates an empty builder.
    ///
    /// Most callers should use the generated `YourClient::builder()` method so
    /// the target client type can be inferred.
    pub fn new() -> Self {
        Self {
            config: RestClientConfig::default(),
            marker: PhantomData,
        }
    }

    /// Sets the configuration key used by [`Self::load_env`].
    ///
    /// Generated clients default this to the trait name or to the explicit
    /// `config_key` macro argument.
    pub fn config_key(mut self, key: impl Into<String>) -> Self {
        self.config.config_key = Some(key.into());
        self
    }

    /// Applies environment configuration for the current config key.
    ///
    /// Catnap reads MicroProfile-style keys such as
    /// `<key>/mp-rest/url` and shell-friendly keys such as
    /// `CATNAP_<KEY>_URL`, where `<KEY>` is uppercased and non-alphanumeric
    /// characters are converted to underscores.
    pub fn load_env(mut self) -> Result<Self> {
        self.config.load_env()?;
        Ok(self)
    }

    /// Sets the base URL used by all generated request paths.
    ///
    /// Request paths are joined against this URL. A trailing slash is added to
    /// the path component when needed so resource paths join predictably.
    pub fn base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.config.base_url = Some(Url::parse(base_url.as_ref())?);
        Ok(self)
    }

    /// Adds or replaces a default header sent with every request.
    ///
    /// Use this for stable headers such as `User-Agent`, bearer tokens or API
    /// keys. Use the [`header`] parameter attribute for per-request values.
    pub fn header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Result<Self> {
        let name_text = name.as_ref();
        let name = HeaderName::from_bytes(name_text.as_bytes())
            .map_err(|_| Error::InvalidHeaderName(name_text.to_owned()))?;
        let value =
            HeaderValue::from_str(value.as_ref()).map_err(|source| Error::InvalidHeaderValue {
                name: name.to_string(),
                source,
            })?;
        self.config.default_headers.insert(name, value);
        Ok(self)
    }

    /// Configures HTTP Basic authentication for every request.
    ///
    /// This method is available when the `basic-auth` feature is enabled, which
    /// is part of the default feature set.
    #[cfg(feature = "basic-auth")]
    pub fn basic_auth(self, username: impl AsRef<str>, password: impl AsRef<str>) -> Result<Self> {
        let credentials = format!("{}:{}", username.as_ref(), password.as_ref());
        let encoded = base64_standard(credentials.as_bytes());
        self.header("Authorization", format!("Basic {encoded}"))
    }

    /// Enables or disables redirect following.
    ///
    /// Redirects are disabled by default. When enabled, catnap uses
    /// `reqwest::redirect::Policy::limited(10)`.
    pub fn follow_redirects(mut self, enabled: bool) -> Self {
        self.config.follow_redirects = enabled;
        self
    }

    /// Sets an HTTP proxy URL used by the underlying `reqwest::Client`.
    pub fn proxy(mut self, url: impl Into<String>) -> Self {
        self.config.proxy = Some(url.into());
        self
    }

    /// Sets a total request timeout on the underlying `reqwest::Client`.
    ///
    /// This is an alias for [`Self::request_timeout`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Sets a total request timeout on the underlying `reqwest::Client`.
    ///
    /// The timeout covers the whole request, from connection through response
    /// body download.
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Sets the timeout for reading response body chunks.
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.config.read_timeout = Some(timeout);
        self
    }

    /// Sets the timeout for establishing a connection.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = Some(timeout);
        self
    }

    /// Sets how repeated query parameters are encoded.
    pub fn query_param_style(mut self, style: QueryParamStyle) -> Self {
        self.config.query_param_style = style;
        self
    }

    /// Registers a response exception mapper for typed response methods.
    ///
    /// Mappers with lower priority values run first. The first mapper that
    /// returns `Some(Error)` controls the request result.
    pub fn response_exception_mapper(
        mut self,
        priority: i32,
        mapper: impl for<'a> Fn(&ResponseExceptionContext<'a>) -> Option<Error> + Send + Sync + 'static,
    ) -> Self {
        self.config
            .response_exception_mappers
            .push(ResponseExceptionMapper::new(priority, mapper));
        self
    }

    /// Disables Catnap's default typed-response mapper for statuses `>= 400`.
    pub fn disable_default_response_exception_mapper(mut self) -> Self {
        self.config.default_response_exception_mapper = false;
        self
    }
}

impl<T> RestClientBuilder<T>
where
    T: BuildFromConfig,
{
    /// Builds the generated client.
    pub fn build(self) -> Result<T> {
        T::build_from_config(self.config)
    }
}

#[cfg(feature = "basic-auth")]
fn base64_standard(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);

        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }

        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

/// Implemented by generated client structs.
pub trait BuildFromConfig: Sized {
    /// Builds a generated client from runtime configuration.
    fn build_from_config(config: RestClientConfig) -> Result<Self>;
}

/// Runtime configuration consumed by generated clients.
#[derive(Debug, Clone)]
pub struct RestClientConfig {
    /// Configuration key used for environment-backed configuration.
    pub config_key: Option<String>,
    /// Base URL used to resolve generated request paths.
    pub base_url: Option<Url>,
    /// Headers applied to every request.
    pub default_headers: HeaderMap,
    /// Whether the underlying client follows redirects.
    pub follow_redirects: bool,
    /// Optional HTTP proxy URL.
    pub proxy: Option<String>,
    /// Optional total request timeout.
    pub timeout: Option<Duration>,
    /// Optional timeout for reading response body chunks.
    pub read_timeout: Option<Duration>,
    /// Optional connection-establishment timeout.
    pub connect_timeout: Option<Duration>,
    /// Encoding style for repeated query parameters.
    pub query_param_style: QueryParamStyle,
    /// Custom response exception mappers for typed response methods.
    pub response_exception_mappers: Vec<ResponseExceptionMapper>,
    /// Whether statuses `>= 400` are mapped to [`Error::Http`] by default.
    pub default_response_exception_mapper: bool,
}

impl Default for RestClientConfig {
    fn default() -> Self {
        Self {
            config_key: None,
            base_url: None,
            default_headers: HeaderMap::default(),
            follow_redirects: false,
            proxy: None,
            timeout: None,
            read_timeout: None,
            connect_timeout: None,
            query_param_style: QueryParamStyle::default(),
            response_exception_mappers: Vec::new(),
            default_response_exception_mapper: true,
        }
    }
}

impl RestClientConfig {
    /// Applies environment-backed configuration using this config's key.
    pub fn load_env(&mut self) -> Result<()> {
        let Some(config_key) = self.config_key.clone() else {
            return Ok(());
        };
        self.load_env_for_key(&config_key)
    }

    /// Applies environment-backed configuration for the supplied key.
    pub fn load_env_for_key(&mut self, config_key: &str) -> Result<()> {
        let source = EnvConfigSource::new(config_key);

        if let Some((key, value)) = source.get("uri").or_else(|| source.get("url")) {
            self.base_url =
                Some(
                    Url::parse(&value).map_err(|source| Error::InvalidConfigValue {
                        key,
                        value,
                        message: source.to_string(),
                    })?,
                );
        }

        if let Some((key, value)) = source.get("followRedirects") {
            self.follow_redirects = parse_config_bool(&key, &value)?;
        }

        if let Some((_, value)) = source.get_any(&["proxyAddress", "proxy"]) {
            self.proxy = Some(proxy_address_to_url(&value));
        }

        if let Some((key, value)) = source.get("connectTimeout") {
            self.connect_timeout = Some(parse_config_duration_ms(&key, &value)?);
        }

        if let Some((key, value)) = source.get("readTimeout") {
            self.read_timeout = Some(parse_config_duration_ms(&key, &value)?);
        }

        if let Some((key, value)) = source.get("queryParamStyle") {
            self.query_param_style =
                QueryParamStyle::parse_config(&value).ok_or_else(|| Error::InvalidConfigValue {
                    key,
                    value,
                    message: "expected MULTI_PAIRS, COMMA_SEPARATED, or ARRAY_PAIRS".to_owned(),
                })?;
        }

        if let Some((key, value)) = source.get_global_any(&[
            "disable.default.mapper",
            "disable.default.response.exception.mapper",
        ]) {
            self.default_response_exception_mapper = !parse_config_bool(&key, &value)?;
        }

        Ok(())
    }
}

struct EnvConfigSource {
    mp_prefix: String,
    env_prefix: String,
}

impl EnvConfigSource {
    fn new(config_key: &str) -> Self {
        Self {
            mp_prefix: format!("{config_key}/mp-rest/"),
            env_prefix: format!("CATNAP_{}_", normalize_config_key(config_key)),
        }
    }

    fn get(&self, property: &str) -> Option<(String, String)> {
        let mp_key = format!("{}{property}", self.mp_prefix);
        if let Ok(value) = env::var(&mp_key) {
            return Some((mp_key, value));
        }

        let env_key = format!("{}{}", self.env_prefix, env_property_name(property));
        env::var(&env_key).ok().map(|value| (env_key, value))
    }

    fn get_any(&self, properties: &[&str]) -> Option<(String, String)> {
        properties.iter().find_map(|property| self.get(property))
    }

    fn get_global(&self, property: &str) -> Option<(String, String)> {
        let mp_key = format!("microprofile.rest.client.{property}");
        if let Ok(value) = env::var(&mp_key) {
            return Some((mp_key, value));
        }

        let env_key = format!("CATNAP_{}", env_property_name(property));
        env::var(&env_key).ok().map(|value| (env_key, value))
    }

    fn get_global_any(&self, properties: &[&str]) -> Option<(String, String)> {
        properties
            .iter()
            .find_map(|property| self.get_global(property))
    }
}

fn normalize_config_key(value: &str) -> String {
    value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn env_property_name(value: &str) -> String {
    value
        .chars()
        .flat_map(|char| {
            if char.is_ascii_uppercase() {
                ['_', char].into_iter()
            } else if char.is_ascii_alphanumeric() {
                ['\0', char.to_ascii_uppercase()].into_iter()
            } else {
                ['\0', '_'].into_iter()
            }
        })
        .filter(|char| *char != '\0')
        .collect()
}

fn parse_config_bool(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" | "TRUE" | "True" | "1" => Ok(true),
        "false" | "FALSE" | "False" | "0" => Ok(false),
        _ => Err(Error::InvalidConfigValue {
            key: key.to_owned(),
            value: value.to_owned(),
            message: "expected true or false".to_owned(),
        }),
    }
}

fn parse_config_duration_ms(key: &str, value: &str) -> Result<Duration> {
    let millis = value
        .parse::<u64>()
        .map_err(|source| Error::InvalidConfigValue {
            key: key.to_owned(),
            value: value.to_owned(),
            message: source.to_string(),
        })?;
    Ok(Duration::from_millis(millis))
}

fn proxy_address_to_url(value: &str) -> String {
    if value.contains("://") {
        value.to_owned()
    } else {
        format!("http://{value}")
    }
}

/// Runtime client used by generated implementations.
///
/// Most applications interact with the generated `<TraitName>Client` rather
/// than this type directly.
#[derive(Debug, Clone)]
pub struct RestClient {
    base_url: Url,
    client: reqwest::Client,
    default_headers: HeaderMap,
    query_param_style: QueryParamStyle,
    response_exception_mappers: Vec<ResponseExceptionMapper>,
    default_response_exception_mapper: bool,
}

impl RestClient {
    /// Creates a runtime client from generated-client configuration.
    pub fn from_config(config: RestClientConfig) -> Result<Self> {
        let mut base_url = config.base_url.ok_or(Error::MissingBaseUrl)?;
        if !base_url.path().ends_with('/') {
            let next_path = format!("{}/", base_url.path());
            base_url.set_path(&next_path);
        }
        let mut client = reqwest::Client::builder().redirect(if config.follow_redirects {
            Policy::limited(10)
        } else {
            Policy::none()
        });

        if let Some(timeout) = config.timeout {
            client = client.timeout(timeout);
        }
        if let Some(timeout) = config.read_timeout {
            client = client.read_timeout(timeout);
        }
        if let Some(timeout) = config.connect_timeout {
            client = client.connect_timeout(timeout);
        }
        if let Some(proxy) = config.proxy {
            client =
                client.proxy(
                    Proxy::all(&proxy).map_err(|source| Error::InvalidConfigValue {
                        key: "proxyAddress".to_owned(),
                        value: proxy,
                        message: source.to_string(),
                    })?,
                );
        }

        let mut response_exception_mappers = config.response_exception_mappers;
        response_exception_mappers.sort_by_key(ResponseExceptionMapper::priority);

        Ok(Self {
            base_url,
            client: client.build()?,
            default_headers: config.default_headers,
            query_param_style: config.query_param_style,
            response_exception_mappers,
            default_response_exception_mapper: config.default_response_exception_mapper,
        })
    }

    /// Starts a request for the given HTTP method and resource path.
    pub fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|source| Error::InvalidRequestUrl {
                path: path.to_owned(),
                source,
            })?;
        Ok(RequestBuilder {
            builder: self.client.request(method, url),
            query_param_style: self.query_param_style,
            query_params: Vec::new(),
            response_exception_mappers: self.response_exception_mappers.clone(),
            default_response_exception_mapper: self.default_response_exception_mapper,
        }
        .headers(self.default_headers.clone()))
    }
}

/// A small wrapper around reqwest's request builder with MicroProfile-like helpers.
pub struct RequestBuilder {
    builder: reqwest::RequestBuilder,
    query_param_style: QueryParamStyle,
    query_params: Vec<(String, String)>,
    response_exception_mappers: Vec<ResponseExceptionMapper>,
    default_response_exception_mapper: bool,
}

impl RequestBuilder {
    /// Applies a complete set of headers to the request.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.builder = self.builder.headers(headers);
        self
    }

    /// Adds a single request header.
    pub fn header(mut self, name: &str, value: impl Display) -> Self {
        self.builder = self.builder.header(name, value.to_string());
        self
    }

    /// Sets the `Accept` header.
    pub fn accept(mut self, media_type: &'static str) -> Self {
        self.builder = self.builder.header(ACCEPT, media_type);
        self
    }

    /// Sets the `Content-Type` header.
    pub fn content_type(mut self, media_type: &'static str) -> Self {
        self.builder = self.builder.header(CONTENT_TYPE, media_type);
        self
    }

    /// Adds a query parameter.
    pub fn query_param(mut self, name: &str, value: impl Display) -> Self {
        self.query_params.push((name.to_owned(), value.to_string()));
        self
    }

    /// Adds repeated query parameter values according to the configured style.
    pub fn query_params<I, V>(mut self, name: &str, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Display,
    {
        let values: Vec<String> = values.into_iter().map(|value| value.to_string()).collect();
        match self.query_param_style {
            QueryParamStyle::MultiPairs => {
                for value in values {
                    self.query_params.push((name.to_owned(), value));
                }
            }
            QueryParamStyle::CommaSeparated => {
                self.query_params.push((name.to_owned(), values.join(",")));
            }
            QueryParamStyle::ArrayPairs => {
                let array_name = format!("{name}[]");
                for value in values {
                    self.query_params.push((array_name.clone(), value));
                }
            }
        }
        self
    }

    /// Adds dynamic query parameters from a map-like value.
    pub fn query_map(self, values: impl QueryMap) -> Self {
        values.append_query_map(self)
    }

    /// Serializes a JSON request body.
    ///
    /// This method requires the `json` feature.
    pub fn json<T: Serialize + ?Sized>(self, body: &T) -> Result<Self> {
        #[cfg(feature = "json")]
        {
            let mut this = self;
            this.builder = this.builder.json(body);
            Ok(this)
        }

        #[cfg(not(feature = "json"))]
        {
            let _ = body;
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::RequestSerialization,
                media_type: "application/json",
            })
        }
    }

    /// Sets a plain text request body.
    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.builder = self.builder.body(body.into());
        self
    }

    /// Serializes an XML request body.
    ///
    /// This method requires the `xml` feature.
    pub fn xml<T: Serialize + ?Sized>(self, body: &T) -> Result<Self> {
        #[cfg(feature = "xml")]
        {
            let mut this = self;
            this.builder = this.builder.body(quick_xml::se::to_string(body)?);
            Ok(this)
        }

        #[cfg(not(feature = "xml"))]
        {
            let _ = body;
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::RequestSerialization,
                media_type: "application/xml",
            })
        }
    }

    fn build(self) -> Result<(reqwest::Client, reqwest::Request)> {
        let (client, request) = self.builder.build_split();
        let mut request = request?;
        if !self.query_params.is_empty() {
            request
                .url_mut()
                .query_pairs_mut()
                .extend_pairs(self.query_params);
        }
        Ok((client, request))
    }

    async fn send_streaming(self) -> Result<reqwest::Response> {
        let (client, request) = self.build()?;
        log_request(&request);
        let response = client.execute(request).await?;
        debug!(
            status = %response.status(),
            url = %response.url(),
            headers = ?LoggedHeaders(response.headers()),
            "received HTTP response"
        );
        Ok(response)
    }

    async fn send_buffered(self) -> Result<BufferedResponse> {
        let (client, request) = self.build()?;
        log_request(&request);
        let response = client.execute(request).await?;
        let status = response.status();
        let url = response.url().clone();
        let headers = response.headers().clone();
        let body = response.bytes().await?.to_vec();
        if tracing::enabled!(Level::DEBUG) {
            debug!(
                status = %status,
                url = %url,
                headers = ?LoggedHeaders(&headers),
                body = ?LoggedBody(&body),
                "received HTTP response"
            );
        }
        Ok(BufferedResponse {
            status,
            headers,
            body,
        })
    }

    /// Sends the request and returns a raw streaming response.
    pub async fn send(self) -> Result<Response> {
        let response = self.send_streaming().await?;
        Ok(Response {
            inner: ResponseInner::Streaming(response),
        })
    }

    /// Sends the request and deserializes a JSON response body.
    ///
    /// Statuses `>= 400` return [`Error::Http`] by default. This method requires the
    /// `json` feature.
    pub async fn send_json<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "json")]
        {
            let response_exception_mappers = self.response_exception_mappers.clone();
            let default_response_exception_mapper = self.default_response_exception_mapper;
            let response = self.send_buffered().await?;
            if let Some(error) = map_response_exception(
                &response,
                &response_exception_mappers,
                default_response_exception_mapper,
            ) {
                return Err(error);
            }
            Ok(serde_json::from_slice(&response.body)?)
        }

        #[cfg(not(feature = "json"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::ResponseDeserialization,
                media_type: "application/json",
            })
        }
    }

    /// Sends the request and decodes the response body as text.
    ///
    /// Statuses `>= 400` return [`Error::Http`] by default.
    pub async fn send_text(self) -> Result<String> {
        let response_exception_mappers = self.response_exception_mappers.clone();
        let default_response_exception_mapper = self.default_response_exception_mapper;
        let response = self.send_buffered().await?;
        if let Some(error) = map_response_exception(
            &response,
            &response_exception_mappers,
            default_response_exception_mapper,
        ) {
            return Err(error);
        }
        Ok(response.into_body_text())
    }

    /// Sends the request and deserializes an XML response body.
    ///
    /// Statuses `>= 400` return [`Error::Http`] by default. This method requires the
    /// `xml` feature.
    pub async fn send_xml<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "xml")]
        {
            let response_exception_mappers = self.response_exception_mappers.clone();
            let default_response_exception_mapper = self.default_response_exception_mapper;
            let response = self.send_buffered().await?;
            if let Some(error) = map_response_exception(
                &response,
                &response_exception_mappers,
                default_response_exception_mapper,
            ) {
                return Err(error);
            }
            let body = response.into_body_text();
            Ok(quick_xml::de::from_str(&body)?)
        }

        #[cfg(not(feature = "xml"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::ResponseDeserialization,
                media_type: "application/xml",
            })
        }
    }

    /// Sends the request and expects only a successful status.
    ///
    /// Statuses `>= 400` return [`Error::Http`] by default.
    pub async fn send_empty(self) -> Result<()> {
        let response_exception_mappers = self.response_exception_mappers.clone();
        let default_response_exception_mapper = self.default_response_exception_mapper;
        let response = self.send_buffered().await?;
        if let Some(error) = map_response_exception(
            &response,
            &response_exception_mappers,
            default_response_exception_mapper,
        ) {
            return Err(error);
        }
        Ok(())
    }
}

fn map_response_exception(
    response: &BufferedResponse,
    response_exception_mappers: &[ResponseExceptionMapper],
    default_response_exception_mapper: bool,
) -> Option<Error> {
    let context = response.exception_context();
    for mapper in response_exception_mappers {
        if let Some(error) = mapper.map_response(&context) {
            return Some(error);
        }
    }

    if default_response_exception_mapper && response.status.as_u16() >= 400 {
        return Some(Error::Http {
            status: response.status,
            body: response.body_text_lossy(),
        });
    }

    None
}

struct LoggedHeaders<'a>(&'a HeaderMap);

impl std::fmt::Debug for LoggedHeaders<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = formatter.debug_map();
        for (name, value) in self.0 {
            if is_sensitive_header(name) {
                map.entry(&name.as_str(), &"<redacted>");
            } else {
                map.entry(&name.as_str(), &value.to_str().unwrap_or("<non-UTF-8>"));
            }
        }
        map.finish()
    }
}

fn is_sensitive_header(name: &HeaderName) -> bool {
    name == AUTHORIZATION || name == PROXY_AUTHORIZATION || name == COOKIE || name == SET_COOKIE
}

fn log_request(request: &reqwest::Request) {
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }

    debug!(
        method = %request.method(),
        url = %request.url(),
        headers = ?LoggedHeaders(request.headers()),
        body = ?request.body().and_then(reqwest::Body::as_bytes).map(LoggedBody),
        "sending HTTP request"
    );
}

struct LoggedBody<'a>(&'a [u8]);

impl std::fmt::Debug for LoggedBody<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(self.0) {
            Ok(body) => formatter.write_str(body),
            Err(_) => write!(formatter, "<{} non-UTF-8 bytes>", self.0.len()),
        }
    }
}

struct BufferedResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl BufferedResponse {
    fn exception_context(&self) -> ResponseExceptionContext<'_> {
        ResponseExceptionContext {
            status: self.status,
            headers: &self.headers,
            body: &self.body,
        }
    }

    fn body_text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn into_body_text(self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Raw response wrapper for callers that need headers, status, or custom body handling.
///
/// Returning `Result<Response>` from a generated trait method leaves status and
/// body handling to the caller. Unlike typed response methods, raw responses do
/// do not apply response exception mapping.
pub struct Response {
    inner: ResponseInner,
}

enum ResponseInner {
    Streaming(reqwest::Response),
}

impl Response {
    /// Returns the HTTP status code.
    pub fn status(&self) -> StatusCode {
        match &self.inner {
            ResponseInner::Streaming(response) => response.status(),
        }
    }

    /// Returns the HTTP response headers.
    pub fn headers(&self) -> &HeaderMap {
        match &self.inner {
            ResponseInner::Streaming(response) => response.headers(),
        }
    }

    /// Reads the full response body as text.
    pub async fn text(self) -> Result<String> {
        match self.inner {
            ResponseInner::Streaming(response) => Ok(response.text().await?),
        }
    }

    /// Reads the full response body and deserializes it as JSON.
    ///
    /// This method requires the `json` feature.
    pub async fn json<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "json")]
        {
            match self.inner {
                ResponseInner::Streaming(response) => Ok(response.json::<T>().await?),
            }
        }

        #[cfg(not(feature = "json"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::ResponseDeserialization,
                media_type: "application/json",
            })
        }
    }

    /// Reads the full response body and deserializes it as XML.
    ///
    /// This method requires the `xml` feature.
    pub async fn xml<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "xml")]
        {
            match self.inner {
                ResponseInner::Streaming(response) => {
                    Ok(quick_xml::de::from_str(&response.text().await?)?)
                }
            }
        }

        #[cfg(not(feature = "xml"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: MediaOperation::ResponseDeserialization,
                media_type: "application/xml",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_base_url() {
        let err = RestClient::from_config(RestClientConfig::default())
            .expect_err("missing base URL should fail");

        assert!(matches!(err, Error::MissingBaseUrl));
    }

    #[test]
    fn request_reports_invalid_url_paths() {
        let client = RestClient::from_config(RestClientConfig {
            base_url: Some(Url::parse("https://example.com").expect("valid base URL")),
            ..RestClientConfig::default()
        })
        .expect("client should build");

        let err = match client.request(Method::GET, "https://[") {
            Ok(_) => panic!("invalid request path should fail"),
            Err(err) => err,
        };

        assert!(matches!(err, Error::InvalidRequestUrl { .. }));
    }

    #[test]
    fn path_segment_encoding_preserves_single_segment() {
        assert_eq!(__private::encode_path_segment("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn media_operations_have_stable_display_text() {
        assert_eq!(
            MediaOperation::RequestSerialization.to_string(),
            "request body serialization"
        );
        assert_eq!(
            MediaOperation::ResponseDeserialization.to_string(),
            "response deserialization"
        );
    }

    #[test]
    fn builder_sets_split_timeouts() {
        let builder = RestClientBuilder::<RestClient>::new()
            .request_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2));

        assert_eq!(builder.config.timeout, Some(Duration::from_secs(10)));
        assert_eq!(builder.config.read_timeout, Some(Duration::from_secs(5)));
        assert_eq!(builder.config.connect_timeout, Some(Duration::from_secs(2)));
    }

    #[test]
    fn builder_sets_config_key_and_proxy() {
        let builder = RestClientBuilder::<RestClient>::new()
            .config_key("users-api")
            .proxy("http://127.0.0.1:8080");

        assert_eq!(builder.config.config_key.as_deref(), Some("users-api"));
        assert_eq!(
            builder.config.proxy.as_deref(),
            Some("http://127.0.0.1:8080")
        );
    }

    #[test]
    fn timeout_sets_total_request_timeout() {
        let builder = RestClientBuilder::<RestClient>::new().timeout(Duration::from_secs(10));

        assert_eq!(builder.config.timeout, Some(Duration::from_secs(10)));
    }

    #[test]
    fn config_key_normalizes_to_environment_prefix() {
        assert_eq!(normalize_config_key("users-api"), "USERS_API");
        assert_eq!(env_property_name("followRedirects"), "FOLLOW_REDIRECTS");
        assert_eq!(env_property_name("connectTimeout"), "CONNECT_TIMEOUT");
    }

    #[test]
    fn config_parser_applies_shell_friendly_env_keys() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        unsafe {
            env::set_var("CATNAP_USERS_API_URL", "https://api.example.com");
            env::set_var("CATNAP_USERS_API_FOLLOW_REDIRECTS", "true");
            env::set_var("CATNAP_USERS_API_PROXY", "proxy.example.com:8080");
            env::set_var("CATNAP_USERS_API_CONNECT_TIMEOUT", "250");
            env::set_var("CATNAP_USERS_API_READ_TIMEOUT", "1000");
            env::set_var("CATNAP_USERS_API_QUERY_PARAM_STYLE", "COMMA_SEPARATED");
            env::set_var("CATNAP_DISABLE_DEFAULT_MAPPER", "true");
        }

        let mut config = RestClientConfig {
            config_key: Some("users-api".to_owned()),
            ..RestClientConfig::default()
        };
        let result = config.load_env();

        unsafe {
            env::remove_var("CATNAP_USERS_API_URL");
            env::remove_var("CATNAP_USERS_API_FOLLOW_REDIRECTS");
            env::remove_var("CATNAP_USERS_API_PROXY");
            env::remove_var("CATNAP_USERS_API_CONNECT_TIMEOUT");
            env::remove_var("CATNAP_USERS_API_READ_TIMEOUT");
            env::remove_var("CATNAP_USERS_API_QUERY_PARAM_STYLE");
            env::remove_var("CATNAP_DISABLE_DEFAULT_MAPPER");
        }

        result.expect("env config should parse");
        assert_eq!(
            config.base_url.as_ref().map(Url::as_str),
            Some("https://api.example.com/")
        );
        assert!(config.follow_redirects);
        assert_eq!(
            config.proxy.as_deref(),
            Some("http://proxy.example.com:8080")
        );
        assert_eq!(config.connect_timeout, Some(Duration::from_millis(250)));
        assert_eq!(config.read_timeout, Some(Duration::from_millis(1000)));
        assert_eq!(config.timeout, None);
        assert_eq!(config.query_param_style, QueryParamStyle::CommaSeparated);
        assert!(!config.default_response_exception_mapper);
    }

    #[cfg(feature = "basic-auth")]
    #[test]
    fn basic_auth_sets_authorization_header() {
        let builder = RestClientBuilder::<RestClient>::new()
            .basic_auth("catnap", "secret")
            .expect("basic auth header should be valid");
        let value = builder
            .config
            .default_headers
            .get("Authorization")
            .expect("authorization header should be set");

        assert_eq!(value, "Basic Y2F0bmFwOnNlY3JldA==");
    }
}
