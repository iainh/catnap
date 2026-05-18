//! Trait-first REST clients inspired by Eclipse MicroProfile REST Client.
//!
//! The main entry point is [`rest_client`], which turns a Rust trait into a
//! generated reqwest-backed client implementation.
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
use std::fmt::Display;
use std::marker::PhantomData;
use std::time::Duration;
use tracing::{Level, debug};
use url::Url;

#[cfg(feature = "basic-auth")]
use base64::Engine;

/// Crate-wide result type.
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

/// Errors produced while building or invoking a generated REST client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base URL is required")]
    MissingBaseUrl,
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("invalid request URL path `{path}`: {source}")]
    InvalidRequestUrl {
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("invalid HTTP header name `{0}`")]
    InvalidHeaderName(String),
    #[error("invalid HTTP header value for `{name}`: {source}")]
    InvalidHeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("unsupported media type `{media_type}` for {operation}")]
    UnsupportedMediaType {
        operation: &'static str,
        media_type: &'static str,
    },
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[cfg(feature = "json")]
    #[error("JSON deserialization failed: {0}")]
    JsonDeserialize(#[from] serde_json::Error),
    #[cfg(feature = "xml")]
    #[error("XML deserialization failed: {0}")]
    XmlDeserialize(#[from] quick_xml::de::DeError),
    #[cfg(feature = "xml")]
    #[error("XML serialization failed: {0}")]
    XmlSerialize(#[from] quick_xml::se::SeError),
    #[error("HTTP {status}: {body}")]
    Http { status: StatusCode, body: String },
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

/// A typed REST client builder.
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
    pub fn new() -> Self {
        Self {
            config: RestClientConfig::default(),
            marker: PhantomData,
        }
    }

    pub fn base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.config.base_url = Some(Url::parse(base_url.as_ref())?);
        Ok(self)
    }

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

    #[cfg(feature = "basic-auth")]
    pub fn basic_auth(self, username: impl AsRef<str>, password: impl AsRef<str>) -> Result<Self> {
        let credentials = format!("{}:{}", username.as_ref(), password.as_ref());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        self.header("Authorization", format!("Basic {encoded}"))
    }

    pub fn follow_redirects(mut self, enabled: bool) -> Self {
        self.config.follow_redirects = enabled;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    pub fn query_param_style(mut self, style: QueryParamStyle) -> Self {
        self.config.query_param_style = style;
        self
    }
}

impl<T> RestClientBuilder<T>
where
    T: BuildFromConfig,
{
    pub fn build(self) -> Result<T> {
        T::build_from_config(self.config)
    }
}

/// Implemented by generated client structs.
pub trait BuildFromConfig: Sized {
    fn build_from_config(config: RestClientConfig) -> Result<Self>;
}

/// Runtime configuration consumed by generated clients.
#[derive(Debug, Clone, Default)]
pub struct RestClientConfig {
    pub base_url: Option<Url>,
    pub default_headers: HeaderMap,
    pub follow_redirects: bool,
    pub timeout: Option<Duration>,
    pub query_param_style: QueryParamStyle,
}

/// A reqwest-backed runtime client used by generated implementations.
#[derive(Debug, Clone)]
pub struct RestClient {
    base_url: Url,
    client: reqwest::Client,
    default_headers: HeaderMap,
    query_param_style: QueryParamStyle,
}

impl RestClient {
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
        }
        .headers(self.default_headers.clone()))
    }
}

/// A small wrapper around reqwest's request builder with MicroProfile-like helpers.
pub struct RequestBuilder {
    builder: reqwest::RequestBuilder,
    query_param_style: QueryParamStyle,
}

impl RequestBuilder {
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.builder = self.builder.headers(headers);
        self
    }

    pub fn header(mut self, name: &str, value: impl Display) -> Self {
        self.builder = self.builder.header(name, value.to_string());
        self
    }

    pub fn accept(mut self, media_type: &'static str) -> Self {
        self.builder = self.builder.header(ACCEPT, media_type);
        self
    }

    pub fn content_type(mut self, media_type: &'static str) -> Self {
        self.builder = self.builder.header(CONTENT_TYPE, media_type);
        self
    }

    pub fn query_param(mut self, name: &str, value: impl Display) -> Self {
        self.builder = self.builder.query(&[(name, value.to_string())]);
        self
    }

    pub fn query_params<I, V>(mut self, name: &str, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Display,
    {
        let values: Vec<String> = values.into_iter().map(|value| value.to_string()).collect();
        match self.query_param_style {
            QueryParamStyle::MultiPairs => {
                for value in values {
                    self.builder = self.builder.query(&[(name, value)]);
                }
            }
            QueryParamStyle::CommaSeparated => {
                self.builder = self.builder.query(&[(name, values.join(","))]);
            }
            QueryParamStyle::ArrayPairs => {
                let array_name = format!("{name}[]");
                for value in values {
                    self.builder = self.builder.query(&[(array_name.as_str(), value)]);
                }
            }
        }
        self
    }

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

    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.builder = self.builder.body(body.into());
        self
    }

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

    async fn send_streaming(self) -> Result<reqwest::Response> {
        log_request(&self.builder);
        let response = self.builder.send().await?;
        debug!(
            status = %response.status(),
            url = %response.url(),
            headers = ?LoggedHeaders(response.headers()),
            "received HTTP response"
        );
        Ok(response)
    }

    async fn send_buffered(self) -> Result<BufferedResponse> {
        log_request(&self.builder);
        let response = self.builder.send().await?;
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

    pub async fn send(self) -> Result<Response> {
        let response = self.send_streaming().await?;
        Ok(Response {
            inner: ResponseInner::Streaming(response),
        })
    }

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

    pub async fn send_text(self) -> Result<String> {
        let response = self.send_buffered().await?;
        let status = response.status;
        if !status.is_success() {
            let body = response.body_text();
            return Err(Error::Http { status, body });
        }
        Ok(response.body_text())
    }

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

fn log_request(builder: &reqwest::RequestBuilder) {
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }

    if let Some(builder) = builder.try_clone() {
        match builder.build() {
            Ok(request) => {
                debug!(
                    method = %request.method(),
                    url = %request.url(),
                    headers = ?LoggedHeaders(request.headers()),
                    body = ?request.body().and_then(reqwest::Body::as_bytes).map(LoggedBody),
                    "sending HTTP request"
                );
            }
            Err(error) => {
                debug!(error = %error, "failed to build request for logging");
            }
        }
    } else {
        debug!("request body is streaming and cannot be cloned for logging");
    }
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
pub struct Response {
    inner: ResponseInner,
}

enum ResponseInner {
    Streaming(reqwest::Response),
}

impl Response {
    pub fn status(&self) -> StatusCode {
        match &self.inner {
            ResponseInner::Streaming(response) => response.status(),
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        match &self.inner {
            ResponseInner::Streaming(response) => response.headers(),
        }
    }

    pub async fn text(self) -> Result<String> {
        match self.inner {
            ResponseInner::Streaming(response) => Ok(response.text().await?),
        }
    }

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
