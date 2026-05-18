# catnap

A Rust REST client experiment inspired by the Eclipse MicroProfile REST Client 4.0 model: define a trait, annotate the HTTP shape, and let generated code handle request construction.

The goal is simple client definitions with little boilerplate:

```rust
use catnap::{consumes, get, path, post, produces, rest_client, Result};
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
    async fn get(&self, #[path("id")] id: &str) -> Result<User>;

    #[post("")]
    async fn create(&self, user: &NewUser) -> Result<User>;

    #[get("/{id}/name")]
    #[produces("text/plain")]
    async fn name(&self, #[path("id")] id: &str) -> Result<String>;

    #[post("/xml")]
    #[consumes("application/xml")]
    #[produces("application/xml")]
    async fn create_xml(&self, user: &User) -> Result<User>;
}

async fn example() -> Result<()> {
    let client = UsersClient::builder()
        .base_url("https://api.example.com")?
        .header("Authorization", "Bearer token")?
        .build()?;

    let user = client.get("42").await?;
    Ok(())
}
```

JSON support is enabled by default. Enable XML support with:

```toml
catnap = { version = "0.1", features = ["xml"] }
```

For builds without JSON support:

```toml
catnap = { version = "0.1", default-features = false }
```

## Current Scope

This first slice supports:

- Trait-based client definitions via `#[rest_client]`
- HTTP method annotations: `#[get]`, `#[post]`, `#[put]`, `#[patch]`, `#[delete]`, `#[options]`, `#[head]`
- Path parameters with `#[path("name")]`
- Query parameters with `#[query("name")]`
- Header parameters with `#[header("Name")]`
- Media types with `#[consumes("...")]` and `#[produces("...")]`, defaulting to `application/json`
- JSON request/response bodies enabled by the default `json` feature
- Optional XML request/response bodies with the `xml` feature
- `text/plain` string responses, raw `Response`, and `Result<()>`
- Builder configuration for base URL, default headers, redirect handling, timeout, and query parameter style

The design intentionally mirrors MicroProfile's developer experience while staying idiomatic for Rust: explicit async traits, typed `Result`, and feature-gated media support.

## Examples

The `catnap` crate includes examples using public `httpbin.io` endpoints:

```sh
cargo run -p catnap --example httpbin_json
cargo run -p catnap --example httpbin_status
cargo run -p catnap --example httpbin_basic_auth
cargo run -p catnap --features xml --example httpbin_xml
```
