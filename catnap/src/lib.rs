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
    delete, get, head, header, options, patch, path, post, put, query, rest_client,
};
pub use http;

use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::marker::PhantomData;
use std::time::Duration;
use url::Url;

/// Crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while building or invoking a generated REST client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("invalid HTTP header name `{0}`")]
    InvalidHeaderName(String),
    #[error("invalid HTTP header value for `{name}`: {source}")]
    InvalidHeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
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
        let mut base_url = config
            .base_url
            .unwrap_or_else(|| Url::parse("http://localhost").expect("static URL is valid"));
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

    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let url = self.base_url.join(path.trim_start_matches('/'));
        RequestBuilder {
            builder: self
                .client
                .request(method, url.unwrap_or_else(|_| self.base_url.clone())),
            query_param_style: self.query_param_style,
        }
        .headers(self.default_headers.clone())
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

    pub fn header(mut self, name: &str, value: impl ToString) -> Self {
        self.builder = self.builder.header(name, value.to_string());
        self
    }

    pub fn query_param(mut self, name: &str, value: impl ToString) -> Self {
        self.builder = self.builder.query(&[(name, value.to_string())]);
        self
    }

    pub fn query_params<I, V>(mut self, name: &str, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: ToString,
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

    pub fn json<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        self.builder = self.builder.json(body);
        self
    }

    pub async fn send(self) -> Result<Response> {
        let response = self.builder.send().await?;
        Ok(Response { inner: response })
    }

    pub async fn send_json<T: DeserializeOwned>(self) -> Result<T> {
        let response = self.builder.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Http { status, body });
        }
        Ok(response.json::<T>().await?)
    }

    pub async fn send_empty(self) -> Result<()> {
        let response = self.builder.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Http { status, body });
        }
        Ok(())
    }
}

/// Raw response wrapper for callers that need headers, status, or custom body handling.
pub struct Response {
    inner: reqwest::Response,
}

impl Response {
    pub fn status(&self) -> StatusCode {
        self.inner.status()
    }

    pub fn headers(&self) -> &HeaderMap {
        self.inner.headers()
    }

    pub async fn text(self) -> Result<String> {
        Ok(self.inner.text().await?)
    }

    pub async fn json<T: DeserializeOwned>(self) -> Result<T> {
        Ok(self.inner.json::<T>().await?)
    }
}
