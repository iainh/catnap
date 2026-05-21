# catnap

Catnap is a trait-first REST client for Rust. It is inspired by Eclipse
MicroProfile REST Client: describe the remote API as a typed interface, annotate
the HTTP details, and let generated code handle the repetitive request
construction.

Catnap uses `reqwest` at runtime. The value of the crate is the small declarative
layer around `reqwest`: fewer hand-written URLs, headers, query strings and
response conversions in application code.

## Quick start

Add catnap to your project:

```toml
[dependencies]
catnap = "0.7"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Define a trait for the remote resource and annotate each method with its HTTP
shape:

```rust
use catnap::{rest_client, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct User {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct NewUser {
    name: String,
}

#[rest_client(path = "/users")]
trait Users {
    #[get("")]
    async fn list(&self) -> Result<Vec<User>>;

    #[get("/{id}")]
    async fn get(&self, #[path] id: &str) -> Result<User>;

    #[post("")]
    async fn create(&self, user: &NewUser) -> Result<User>;
}

async fn example() -> Result<()> {
    let client = UsersClient::builder()
        .base_url("https://api.example.com")?
        .header("User-Agent", "my-service")?
        .build()?;

    let user = client.get("42").await?;
    println!("{user:?}");

    Ok(())
}
```

The `#[rest_client]` macro keeps the original trait and generates a client named
`<TraitName>Client`. In the example above, `UsersClient` implements `Users`.

## Design model

MicroProfile REST Client uses annotated Java interfaces such as `@GET`,
`@Path`, `@QueryParam` and `@HeaderParam`. Catnap follows the same shape in
Rust:

- `#[rest_client]` defines a typed remote resource.
- `#[get]`, `#[post]`, `#[put]`, `#[patch]`, `#[delete]`, `#[options]` and
  `#[head]` define HTTP methods.
- `#[path]` or `#[path("name")]` replaces `{name}` placeholders in the request path.
- `#[query]` or `#[query("name")]` adds query parameters.
- `#[query_map]` flattens dynamic map values into query parameters.
- `#[header("Name")]` adds per-request headers.
- `#[consumes("type/subtype")]` selects request body serialization.
- `#[produces("type/subtype")]` selects response deserialization.

Catnap stays idiomatic for Rust by using explicit async trait methods, a typed
`catnap::Result<T>`, feature-gated serialization support and normal `serde`
models.

## Paths and parameters

Resource-level paths are declared on the trait. Method-level paths are declared
on HTTP method attributes. Catnap joins them and percent-encodes path parameter
values as single path segments.

```rust
use catnap::{rest_client, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Issue {
    id: String,
    title: String,
}

#[rest_client(path = "/repos/{owner}/{repo}")]
trait Issues {
    #[get("/issues")]
    async fn list(
        &self,
        #[path] owner: &str,
        #[path] repo: &str,
        #[query] state: &str,
    ) -> Result<Vec<Issue>>;
}
```

If a path contains `{owner}`, the method must have a matching `#[path]`
argument named `owner` or an explicit `#[path("owner")]` argument. Extra
`#[path]` arguments that do not match a placeholder are compile errors.

## Query parameter style

Repeated query parameters can be encoded in three common formats:

```rust
use catnap::{QueryParamStyle, Result};

async fn example() -> Result<()> {
    let client = SearchClient::builder()
        .base_url("https://api.example.com")?
        .query_param_style(QueryParamStyle::MultiPairs)
        .build()?;

    Ok(())
}
# use catnap::rest_client;
# #[rest_client]
# trait Search {
#     #[get("/search")]
#     async fn search(&self, #[query("tag")] tag: &str) -> Result<()>;
# }
```

The supported styles are:

- `MultiPairs`: `tag=rust&tag=http`
- `CommaSeparated`: `tag=rust,http`
- `ArrayPairs`: `tag[]=rust&tag[]=http`

For APIs where query parameter names are dynamic, use `#[query_map]` with a
`BTreeMap` or `HashMap` of repeated values:

```rust
use catnap::{rest_client, Result};
use std::collections::BTreeMap;

#[rest_client]
trait Search {
    #[get("/search")]
    async fn search(
        &self,
        #[query_map] parameters: &BTreeMap<String, Vec<String>>,
    ) -> Result<()>;
}
```

`BTreeMap` gives deterministic query ordering, which is useful in tests and
logs. `#[query_map]` uses the same repeated query parameter style as regular
collection-valued `#[query]` parameters.

## Headers and authentication

Use builder headers for values that apply to every request:

```rust
let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .header("Authorization", "Bearer token")?
    .build()?;
# use catnap::{rest_client, Result};
# #[rest_client]
# trait Users {
#     #[get("/users")]
#     async fn list(&self) -> Result<()>;
# }
# Ok::<_, catnap::Error>(())
```

Use method parameters for values that change per request:

```rust
use catnap::{rest_client, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct User {
    id: String,
}

#[rest_client(path = "/users")]
trait Users {
    #[get("/{id}")]
    async fn get(
        &self,
        #[path] id: &str,
        #[header("X-Request-Id")] request_id: &str,
    ) -> Result<User>;
}
```

Basic authentication is available through the default `basic-auth` feature:

```rust
let client = HttpBinClient::builder()
    .base_url("https://httpbin.io")?
    .basic_auth("catnap", "secret")?
    .build()?;
# use catnap::{rest_client, Result};
# #[rest_client]
# trait HttpBin {
#     #[get("/get")]
#     async fn get(&self) -> Result<()>;
# }
# Ok::<_, catnap::Error>(())
```

For bearer tokens, API keys or custom authentication schemes, set the required
header directly with `.header(...)` or a `#[header(...)]` parameter.

## Request and response bodies

Catnap defaults to JSON. An unannotated method parameter is treated as the
request body, and the `Result<T>` response type controls deserialization.

```rust
use catnap::{rest_client, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct CreateTodo {
    title: String,
}

#[derive(Debug, Deserialize)]
struct Todo {
    id: String,
    title: String,
}

#[rest_client(path = "/todos")]
trait Todos {
    #[post("")]
    async fn create(&self, todo: &CreateTodo) -> Result<Todo>;
}
```

For plain text responses, mark the method with `#[produces("text/plain")]` and
return `String`:

```rust
use catnap::{rest_client, Result};

#[rest_client]
trait Health {
    #[get("/health")]
    #[produces("text/plain")]
    async fn health(&self) -> Result<String>;
}
```

For status-only calls, return `Result<()>`. Catnap treats statuses `>= 400` as
`Error::Http` by default.

For custom handling, return `Result<catnap::Response>` and read the raw response
yourself:

```rust
use catnap::{rest_client, Response, Result};

#[rest_client]
trait Downloads {
    #[get("/archive")]
    async fn archive(&self) -> Result<Response>;
}
```

## XML support

XML is optional. Enable the `xml` feature and use `#[consumes]` or `#[produces]`
with `application/xml` or `text/xml`:

```toml
catnap = { version = "0.7", features = ["xml"] }
```

```rust
use catnap::{rest_client, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Feed {
    title: String,
}

#[rest_client]
trait FeedApi {
    #[get("/feed.xml")]
    #[produces("application/xml")]
    async fn feed(&self) -> Result<Feed>;
}
```

XML serialization and deserialization use `quick-xml` with `serde`.

## Runtime configuration

Every generated client has a builder:

```rust
use std::time::Duration;

let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .follow_redirects(true)
    .proxy("http://proxy.example.com:8080")
    .connect_timeout(Duration::from_secs(2))
    .read_timeout(Duration::from_secs(5))
    .request_timeout(Duration::from_secs(10))
    .header("User-Agent", "my-service")?
    .build()?;
# use catnap::{rest_client, Result};
# #[rest_client]
# trait Users {
#     #[get("/users")]
#     async fn list(&self) -> Result<()>;
# }
# Ok::<_, catnap::Error>(())
```

The builder configures:

- Base URL
- Default headers
- Basic authentication
- Redirect handling
- HTTP proxy
- Connect timeout
- Read timeout
- Total request timeout
- Repeated query parameter encoding
- Request and response filters
- Response exception mapping

Generated clients are cloneable. Clones share the underlying `reqwest::Client`.

Clients can also load runtime settings from a config key. By default the key is
the trait name; pass `config_key` to choose a stable deployment-facing name:

```rust
# use catnap::{rest_client, Result};
#[rest_client(path = "/users", config_key = "users-api")]
trait Users {
    #[get("")]
    async fn list(&self) -> Result<()>;
}

let client = UsersClient::from_env()?;
# Ok::<_, catnap::Error>(())
```

`from_env()` and `builder().load_env()?` read MicroProfile-style keys such as
`users-api/mp-rest/url` and shell-friendly keys such as:

- `CATNAP_USERS_API_URL`
- `CATNAP_USERS_API_FOLLOW_REDIRECTS`
- `CATNAP_USERS_API_PROXY` or `CATNAP_USERS_API_PROXY_ADDRESS`
- `CATNAP_USERS_API_CONNECT_TIMEOUT`
- `CATNAP_USERS_API_READ_TIMEOUT`
- `CATNAP_USERS_API_QUERY_PARAM_STYLE`
- `CATNAP_DISABLE_DEFAULT_MAPPER`

See [Runtime configuration](docs/configuration.md) for the full set of keys,
precedence rules and examples.

## Error handling

All generated methods return `catnap::Result<T>`, an alias for
`Result<T, catnap::Error>`.

Common error variants include:

- `MissingBaseUrl` when `.base_url(...)` was not configured
- `InvalidBaseUrl` and `InvalidRequestUrl` for URL construction failures
- `InvalidHeaderName` and `InvalidHeaderValue` for invalid headers
- `InvalidConfigValue` for malformed environment configuration
- `UnsupportedMediaType` when a feature or media type is not supported
- `Request` for `reqwest` transport errors
- `JsonDeserialize` and `XmlDeserialize` for response decoding errors
- `Http` for HTTP statuses `>= 400` in typed response methods
- `MappedResponse` for application errors returned by response exception mappers

Raw `Response` methods do not apply response exception mapping. Inspect the
status yourself when returning `Result<Response>`.

Custom response exception mappers can inspect the buffered status, headers, and
body before typed deserialization:

```rust
use catnap::{Error, ResponseExceptionContext};
use std::{error::Error as StdError, fmt};

#[derive(Debug)]
struct RateLimited;

impl fmt::Display for RateLimited {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("rate limited")
    }
}

impl StdError for RateLimited {}

let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .request_filter(100, |request| {
        request
            .headers_mut()
            .insert("X-Request-Id", "trace-123".parse().unwrap());
        Ok(())
    })
    .response_filter(100, |response| {
        tracing::debug!(status = %response.status());
        Ok(())
    })
    .response_exception_mapper(100, |response: &ResponseExceptionContext<'_>| {
        if response.status() == catnap::http::StatusCode::TOO_MANY_REQUESTS {
            Some(Error::mapped_response(RateLimited))
        } else {
            None
        }
    })
    .build()?;
# use catnap::{rest_client, Result};
# #[rest_client]
# trait Users {
#     #[get("/users")]
#     async fn list(&self) -> Result<()>;
# }
# Ok::<_, catnap::Error>(())
```

## Logging

Catnap emits `tracing` debug events for outgoing requests and incoming
responses. It logs method, URL, headers and body where the body can be cloned or
has already been buffered for typed decoding.

Logging runs after request and response filters, so logs reflect the request and
response state Catnap will use for execution, error mapping and decoding.

Sensitive headers are redacted:

- `Authorization`
- `Proxy-Authorization`
- `Cookie`
- `Set-Cookie`

Install a subscriber in applications or examples to see logs:

```rust
tracing_subscriber::fmt()
    .with_env_filter("catnap=debug")
    .init();
```

Raw `Response` calls preserve streaming response behaviour and log status and
headers only. Typed methods buffer response bodies because they need the bytes
for decoding and error reporting.

## Feature flags

Default features:

- `json`: JSON request and response bodies with `serde_json`
- `basic-auth`: `RestClientBuilder::basic_auth`
- `tls-rustls`: HTTPS support through reqwest's Rustls backend

Optional features:

- `xml`: XML request and response bodies with `quick-xml`

Minimal HTTP-only install without default features:

```toml
catnap = { version = "0.7", default-features = false }
```

JSON without basic auth or TLS:

```toml
catnap = { version = "0.7", default-features = false, features = ["json"] }
```

JSON with HTTPS but without basic auth:

```toml
catnap = { version = "0.7", default-features = false, features = ["json", "tls-rustls"] }
```

JSON and XML:

```toml
catnap = { version = "0.7", features = ["xml"] }
```

## Examples

The crate includes examples using public `httpbin.io` endpoints:

```sh
cargo run -p catnap --example httpbin_json
cargo run -p catnap --example httpbin_status
cargo run -p catnap --example httpbin_basic_auth
cargo run -p catnap --features xml --example httpbin_xml
```

Examples initialize a small `tracing-subscriber` logger with debug output
enabled.

## Current scope

Catnap currently focuses on the core MicroProfile-style client definition model:
typed interfaces, method annotations, path/query/header parameters, media type
selection, builder configuration, basic authentication and tracing.

Provider registration, exception mappers, CDI-style injection, externalized
configuration and advanced multipart/server-sent-event support are not part of
the current release.
