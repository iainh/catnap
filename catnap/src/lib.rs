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
//! use catnap::{get, path, rest_client, RestClientBuilder, Result};
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
//!     async fn get_user(&self, #[path("id")] id: &str) -> Result<User>;
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
//! use catnap::{post, rest_client, Result};
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
//! Use [`path`], [`query`], and [`header`] on method arguments to bind values to
//! the generated request.
//!
//! ```
//! use catnap::{get, header, path, query, rest_client, Result};
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
//!         #[path("tenant")] tenant: &str,
//!         #[path("id")] id: &str,
//!         #[query("include")] include: &str,
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
//! use catnap::{get, produces, rest_client, Result};
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
//! The default features are `json` and `basic-auth`.
//!
//! - `json` enables JSON request and response bodies.
//! - `basic-auth` enables [`RestClientBuilder::basic_auth`].
//! - `xml` enables XML request and response bodies.
//!
//! # Logging
//!
//! Catnap emits `tracing` debug events for outgoing requests and incoming
//! responses. Sensitive headers such as `Authorization`, `Proxy-Authorization`,
//! `Cookie`, and `Set-Cookie` are redacted.

pub use catnap_macros::{
    consumes, delete, get, head, header, options, patch, path, post, produces, put, query,
    rest_client,
};
pub use http;

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, PROXY_AUTHORIZATION, SET_COOKIE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::redirect::Policy;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::error::Error as StdError;
use std::fmt::{self, Display};
use std::marker::PhantomData;
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
    /// The selected request or response media type is not supported.
    UnsupportedMediaType {
        /// The operation that required media type support.
        operation: &'static str,
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
    /// A typed request returned a non-success HTTP status.
    ///
    /// Methods returning [`Response`] leave status handling to the caller.
    Http {
        /// The non-success HTTP status.
        status: StatusCode,
        /// The response body decoded as UTF-8 lossily.
        body: String,
    },
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
            Self::MissingBaseUrl
            | Self::InvalidHeaderName(_)
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

    /// Sets a request timeout on the underlying `reqwest::Client`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Sets how repeated query parameters are encoded.
    pub fn query_param_style(mut self, style: QueryParamStyle) -> Self {
        self.config.query_param_style = style;
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
#[derive(Debug, Clone, Default)]
pub struct RestClientConfig {
    /// Base URL used to resolve generated request paths.
    pub base_url: Option<Url>,
    /// Headers applied to every request.
    pub default_headers: HeaderMap,
    /// Whether the underlying client follows redirects.
    pub follow_redirects: bool,
    /// Optional request timeout.
    pub timeout: Option<Duration>,
    /// Encoding style for repeated query parameters.
    pub query_param_style: QueryParamStyle,
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

        Ok(Self {
            base_url,
            client: client.build()?,
            default_headers: config.default_headers,
            query_param_style: config.query_param_style,
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
        }
        .headers(self.default_headers.clone()))
    }
}

/// A small wrapper around reqwest's request builder with MicroProfile-like helpers.
pub struct RequestBuilder {
    builder: reqwest::RequestBuilder,
    query_param_style: QueryParamStyle,
    query_params: Vec<(String, String)>,
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
                operation: "request body serialization",
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
                operation: "request body serialization",
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
        Ok(BufferedResponse { status, body })
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
    /// Non-success statuses return [`Error::Http`]. This method requires the
    /// `json` feature.
    pub async fn send_json<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "json")]
        {
            let response = self.send_buffered().await?;
            let status = response.status;
            if !status.is_success() {
                let body = response.body_text();
                return Err(Error::Http { status, body });
            }
            Ok(serde_json::from_slice(&response.body)?)
        }

        #[cfg(not(feature = "json"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: "response deserialization",
                media_type: "application/json",
            })
        }
    }

    /// Sends the request and decodes the response body as text.
    ///
    /// Non-success statuses return [`Error::Http`].
    pub async fn send_text(self) -> Result<String> {
        let response = self.send_buffered().await?;
        let status = response.status;
        if !status.is_success() {
            let body = response.body_text();
            return Err(Error::Http { status, body });
        }
        Ok(response.body_text())
    }

    /// Sends the request and deserializes an XML response body.
    ///
    /// Non-success statuses return [`Error::Http`]. This method requires the
    /// `xml` feature.
    pub async fn send_xml<T: DeserializeOwned>(self) -> Result<T> {
        #[cfg(feature = "xml")]
        {
            let response = self.send_buffered().await?;
            let status = response.status;
            let body = response.body_text();
            if !status.is_success() {
                return Err(Error::Http { status, body });
            }
            Ok(quick_xml::de::from_str(&body)?)
        }

        #[cfg(not(feature = "xml"))]
        {
            let _ = self;
            Err(Error::UnsupportedMediaType {
                operation: "response deserialization",
                media_type: "application/xml",
            })
        }
    }

    /// Sends the request and expects only a successful status.
    ///
    /// Non-success statuses return [`Error::Http`].
    pub async fn send_empty(self) -> Result<()> {
        let response = self.send_buffered().await?;
        let status = response.status;
        if !status.is_success() {
            let body = response.body_text();
            return Err(Error::Http { status, body });
        }
        Ok(())
    }
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
    body: Vec<u8>,
}

impl BufferedResponse {
    fn body_text(self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Raw response wrapper for callers that need headers, status, or custom body handling.
///
/// Returning `Result<Response>` from a generated trait method leaves status and
/// body handling to the caller. Unlike typed response methods, raw responses do
/// not turn non-2xx statuses into [`Error::Http`].
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
                operation: "response deserialization",
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
                operation: "response deserialization",
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
