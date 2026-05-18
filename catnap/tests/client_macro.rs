#![allow(dead_code)]

use catnap::{Response, RestClientBuilder, Result, rest_client};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct User {
    id: String,
}

#[derive(Debug, Serialize)]
struct NewUser {
    name: String,
}

#[rest_client(path = "/users", produces = "application/json")]
trait Users {
    #[get("")]
    async fn list(&self, #[query("page")] page: u32) -> Result<Vec<User>>;

    #[get("/{id}")]
    async fn get(&self, #[path("id")] id: &str) -> Result<User>;

    #[post("")]
    async fn create(
        &self,
        #[header("Authorization")] auth: &str,
        user: &NewUser,
    ) -> Result<Response>;

    #[get("/{id}/name")]
    #[catnap::produces("text/plain")]
    async fn name(&self, #[path("id")] id: &str) -> Result<String>;

    #[post("/{id}/name")]
    #[catnap::consumes("text/plain")]
    async fn rename(&self, #[path("id")] id: &str, name: &str) -> Result<()>;
}

#[test]
fn generated_client_can_be_built() -> Result<()> {
    let _client = UsersClient::builder()
        .base_url("https://api.example.com")?
        .header("User-Agent", "catnap-test")?
        .build()?;

    let _builder: RestClientBuilder<UsersClient> = UsersClient::builder();
    Ok(())
}
