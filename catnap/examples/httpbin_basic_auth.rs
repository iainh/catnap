use catnap::{Result, rest_client};
#[allow(unused_imports)]
use catnap::{get, path};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BasicAuthResponse {
    authorized: bool,
    user: String,
}

#[rest_client]
trait HttpBin {
    #[get("/basic-auth/{user}/{password}")]
    async fn check_credentials(
        &self,
        #[path("user")] user: &str,
        #[path("password")] password: &str,
    ) -> Result<BasicAuthResponse>;
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = HttpBinClient::builder()
        .base_url("https://httpbin.io")?
        .basic_auth("catnap", "secret")?
        .build()?;

    let response = client.check_credentials("catnap", "secret").await?;
    println!("authorized={} user={}", response.authorized, response.user);

    Ok(())
}
