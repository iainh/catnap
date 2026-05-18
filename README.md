# catnap

A Rust REST client experiment inspired by the Eclipse MicroProfile REST Client 4.0 model: define a trait, annotate the HTTP shape, and let generated code handle request construction.

The goal is simple client definitions with little boilerplate:

```rust
use catnap::{get, path, post, produces, rest_client, Result};
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

## Current Scope

This first slice supports:

- Trait-based client definitions via `#[rest_client]`
- HTTP method annotations: `#[get]`, `#[post]`, `#[put]`, `#[patch]`, `#[delete]`, `#[options]`, `#[head]`
- Path parameters with `#[path("name")]`
- Query parameters with `#[query("name")]`
- Header parameters with `#[header("Name")]`
- Media types with `#[consumes("...")]` and `#[produces("...")]`, defaulting to `application/json`
- JSON request bodies from the first unannotated argument
- JSON response decoding, `text/plain` string responses, raw `Response`, and `Result<()>`
- Builder configuration for base URL, default headers, redirect handling, timeout, and query parameter style

The design intentionally mirrors MicroProfile's developer experience while staying idiomatic for Rust: explicit async traits, typed `Result`, and serde-powered JSON.
