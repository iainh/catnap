use catnap::{Response, Result, rest_client};
#[allow(unused_imports)]
use catnap::{get, path};

#[rest_client]
trait HttpBin {
    #[get("/status/{code}")]
    async fn status(&self, #[path("code")] code: u16) -> Result<Response>;
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = HttpBinClient::builder()
        .base_url("https://httpbin.io")?
        .build()?;

    let response = client.status(204).await?;
    println!("status endpoint returned HTTP {}", response.status());

    Ok(())
}
