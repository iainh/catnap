# Runtime configuration

Catnap generated clients can be configured directly in code or from a named
configuration key. The design follows the useful parts of MicroProfile Rest
Client configuration while keeping the Rust API explicit and easy to test.

Use this guide when you need to move endpoint URLs, timeouts, proxy settings or
query parameter style out of application code.

## Configuration styles

Catnap supports three configuration styles:

- Builder configuration in Rust code
- Config-keyed environment configuration
- Direct `RestClientConfig` construction

You can combine these styles. Environment loading updates the current builder
or config, so values you set before `load_env()` can be replaced by configured
values. Values you set after `load_env()` take final precedence.

```rust
let client = UsersClient::builder()
    .config_key("users-api")
    .load_env()?
    .request_timeout(std::time::Duration::from_secs(5))
    .build()?;
```

In this example, the environment supplies most values, but the request timeout
is always 5 seconds.

## Builder configuration

Every generated client has a `builder()` method. Use the builder when the
configuration is known in code, supplied by your own config library or assembled
from multiple sources.

```rust
let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .follow_redirects(true)
    .proxy("http://proxy.example.com:8080")
    .connect_timeout(std::time::Duration::from_millis(250))
    .read_timeout(std::time::Duration::from_secs(1))
    .request_timeout(std::time::Duration::from_secs(2))
    .query_param_style(catnap::QueryParamStyle::CommaSeparated)
    .header("User-Agent", "my-service")?
    .build()?;
```

The builder supports:

- `base_url(...)` for the service base URL
- `header(...)` for default headers sent on every request
- `basic_auth(...)` when the `basic-auth` feature is enabled
- `follow_redirects(...)` for HTTP redirect handling
- `proxy(...)` for an HTTP proxy URL
- `connect_timeout(...)` for connection establishment
- `read_timeout(...)` for reading response body chunks
- `request_timeout(...)` or `timeout(...)` for the whole request deadline
- `query_param_style(...)` for repeated query parameters
- `request_filter(...)` for outgoing request filters
- `response_filter(...)` for incoming response filters
- `response_exception_mapper(...)` for custom response mapping
- `disable_default_response_exception_mapper()` for typed response status handling

## Config keys

A config key names a generated client for external configuration. By default,
Catnap uses the trait name as the config key.

```rust
#[rest_client(path = "/users")]
trait Users {
    #[get("")]
    async fn list(&self) -> catnap::Result<()>;
}

assert_eq!(UsersClient::CONFIG_KEY, "Users");
```

Pass `config_key` to choose a stable deployment-facing name. This is useful
when Rust trait names might change but deployment configuration should not.

```rust
#[rest_client(path = "/users", config_key = "users-api")]
trait Users {
    #[get("")]
    async fn list(&self) -> catnap::Result<()>;
}

assert_eq!(UsersClient::CONFIG_KEY, "users-api");
```

The generated client exposes two config-key helpers:

- `UsersClient::from_env()` builds the client from environment-backed settings
- `UsersClient::builder().load_env()?` applies environment settings and lets you
  keep configuring the builder

```rust
let client = UsersClient::from_env()?;
```

```rust
let client = UsersClient::builder()
    .load_env()?
    .header("User-Agent", "my-service")?
    .build()?;
```

## Environment key styles

Catnap reads two environment key styles for each config key.

The MicroProfile-style form is:

```text
<config-key>/mp-rest/<property>
```

Example:

```text
users-api/mp-rest/url=https://api.example.com
users-api/mp-rest/followRedirects=true
users-api/mp-rest/queryParamStyle=COMMA_SEPARATED
```

This form mirrors the MicroProfile Rest Client specification. It is useful for
config systems that can store arbitrary key names. It is not shell-friendly
because common shells do not allow variable names that contain `/`.

The shell-friendly form is:

```text
CATNAP_<CONFIG_KEY>_<PROPERTY>
```

Catnap converts the config key to uppercase and replaces non-alphanumeric
characters with underscores. It converts camel-case property names to uppercase
snake case.

Examples:

```text
CATNAP_USERS_API_URL=https://api.example.com
CATNAP_USERS_API_FOLLOW_REDIRECTS=true
CATNAP_USERS_API_QUERY_PARAM_STYLE=COMMA_SEPARATED
```

For the config key `users-api`, Catnap uses the prefix `CATNAP_USERS_API_`.

## Supported environment properties

The following table lists the supported properties. MicroProfile-style keys use
the property name in the first column. Shell-friendly keys use the environment
name in the second column.

| Property | Shell-friendly name | Value | Effect |
| --- | --- | --- | --- |
| `url` | `CATNAP_<KEY>_URL` | URL | Sets the base URL |
| `uri` | `CATNAP_<KEY>_URI` | URI or URL | Sets the base URL and takes precedence over `url` |
| `followRedirects` | `CATNAP_<KEY>_FOLLOW_REDIRECTS` | `true`, `false`, `1` or `0` | Enables or disables redirect following |
| `proxyAddress` | `CATNAP_<KEY>_PROXY_ADDRESS` | `host:port` or URL | Sets the proxy |
| `proxy` | `CATNAP_<KEY>_PROXY` | `host:port` or URL | Sets the proxy |
| `connectTimeout` | `CATNAP_<KEY>_CONNECT_TIMEOUT` | Milliseconds | Sets the connection timeout |
| `readTimeout` | `CATNAP_<KEY>_READ_TIMEOUT` | Milliseconds | Sets the read timeout |
| `queryParamStyle` | `CATNAP_<KEY>_QUERY_PARAM_STYLE` | `MULTI_PAIRS`, `COMMA_SEPARATED` or `ARRAY_PAIRS` | Sets repeated query parameter encoding |

Catnap also reads a global default response mapper setting:

| MicroProfile-style name | Shell-friendly name | Value | Effect |
| --- | --- | --- | --- |
| `microprofile.rest.client.disable.default.mapper` | `CATNAP_DISABLE_DEFAULT_MAPPER` | `true`, `false`, `1` or `0` | Disables or enables default mapping for HTTP statuses `>= 400` |
| `microprofile.rest.client.disable.default.response.exception.mapper` | `CATNAP_DISABLE_DEFAULT_RESPONSE_EXCEPTION_MAPPER` | `true`, `false`, `1` or `0` | Same as above |

## Precedence rules

Catnap uses these precedence rules when `load_env()` runs:

1. For a single property, the MicroProfile-style key wins over the
   shell-friendly key.
2. `uri` wins over `url`.
3. `proxyAddress` wins over `proxy`.
4. `microprofile.rest.client.disable.default.mapper` wins over
   `microprofile.rest.client.disable.default.response.exception.mapper`.
5. Builder calls after `load_env()` replace loaded values.

These rules keep MicroProfile-compatible names stable while allowing
Rust-friendly aliases for common deployment environments.

## Macro attributes

The `#[rest_client]` macro accepts resource-level attributes:

```rust
#[rest_client(
    path = "/users",
    consumes = "application/json",
    produces = "application/json",
    config_key = "users-api",
)]
trait Users {
    #[get("")]
    async fn list(&self) -> catnap::Result<()>;
}
```

The attributes are:

- `path`: Sets the resource path prefix for all methods
- `consumes`: Sets the default request body media type
- `produces`: Sets the default response media type
- `config_key`: Sets the generated client's configuration key

Method-level `#[consumes(...)]` and `#[produces(...)]` attributes override
resource-level media type defaults for that method.

`config_key` must not be empty. If you omit it, Catnap uses the trait name.

Method parameter attributes such as `#[path]`, `#[query]`, `#[query_map]` and
`#[header("Name")]` describe how a method argument is bound to a request. They
are part of the generated client API and are not loaded from environment
configuration.

## Query parameter styles

Use `queryParamStyle` or `query_param_style(...)` when an API expects repeated
query values in a specific format.

`MULTI_PAIRS` sends repeated key-value pairs:

```text
tag=rust&tag=http
```

`COMMA_SEPARATED` sends one key with comma-separated values:

```text
tag=rust,http
```

`ARRAY_PAIRS` sends repeated array-style keys:

```text
tag[]=rust&tag[]=http
```

The selected style applies to collection-valued `#[query]` parameters and to
`#[query_map]` values.

## Proxy values

The `proxy(...)` builder method expects a proxy URL:

```rust
let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .proxy("http://proxy.example.com:8080")
    .build()?;
```

Environment proxy properties accept either a full URL or a MicroProfile-style
`host:port` value. If the value does not include a scheme, Catnap prefixes it
with `http://`.

```text
CATNAP_USERS_API_PROXY=proxy.example.com:8080
```

This is treated as:

```text
http://proxy.example.com:8080
```

Invalid proxy values fail while building the client and return
`Error::InvalidConfigValue`.

## Timeouts

Timeout environment values are durations in milliseconds.

`connectTimeout` controls how long Catnap waits to establish the connection.

`readTimeout` controls how long Catnap waits while reading response body
chunks.

Catnap also has a Rust-only `request_timeout(...)` builder method for setting a
total request deadline. This timeout covers the whole request, including
connection reuse, request upload, response headers and response body download.
It is not loaded from environment configuration because MicroProfile Rest
Client only defines `connectTimeout` and `readTimeout`.

Use the builder when you prefer `Duration` values in code:

```rust
let client = UsersClient::builder()
    .base_url("https://api.example.com")?
    .connect_timeout(std::time::Duration::from_millis(250))
    .read_timeout(std::time::Duration::from_secs(1))
    .request_timeout(std::time::Duration::from_secs(2))
    .build()?;
```

## Error handling

Environment parsing can fail before the client is built. Catnap returns
`Error::InvalidConfigValue` for malformed values.

Common causes include:

- An invalid `url` or `uri`
- An invalid boolean value for `followRedirects`
- A non-numeric timeout
- An unknown `queryParamStyle`
- An invalid proxy URL

`from_env()` still requires a base URL. If no `url`, `uri` or explicit
`base_url(...)` is supplied, building the client returns `Error::MissingBaseUrl`.

## What environment configuration does not cover

Environment loading configures serializable runtime settings only. Register
custom Rust values in code, including:

- Default headers that contain secrets or runtime tokens
- Request and response filter closures
- Response exception mapper closures
- Custom request setup that depends on application state

```rust
#[derive(Debug)]
struct RateLimited;

impl std::fmt::Display for RateLimited {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("rate limited")
    }
}

impl std::error::Error for RateLimited {}

let token = "token";

let client = UsersClient::builder()
    .load_env()?
    .header("Authorization", format!("Bearer {token}"))?
    .request_filter(100, |request| {
        request
            .headers_mut()
            .insert("X-Request-Id", "trace-123".parse().unwrap());
        Ok(())
    })
    .response_exception_mapper(100, |response| {
        if response.status() == catnap::http::StatusCode::TOO_MANY_REQUESTS {
            Some(catnap::Error::mapped_response(RateLimited))
        } else {
            None
        }
    })
    .build()?;
```

This split keeps deployment configuration external while keeping Rust-specific
behaviour type-checked.
