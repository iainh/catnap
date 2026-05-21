use catnap::{Error, Result, rest_client};
use std::error::Error as StdError;
use std::fmt;

mod support;

#[derive(Debug)]
struct TeapotError {
    body: String,
}

impl fmt::Display for TeapotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "remote service refused coffee: {}", self.body)
    }
}

impl StdError for TeapotError {}

#[rest_client]
trait HttpBin {
    #[get("/status/{code}")]
    async fn status_text(&self, #[path] code: u16) -> Result<String>;
}

#[tokio::main]
async fn main() -> Result<()> {
    support::init_tracing();

    let client = HttpBinClient::builder()
        .base_url("https://httpbin.io")?
        .response_exception_mapper(100, |response| {
            if response.status() == catnap::http::StatusCode::IM_A_TEAPOT {
                Some(Error::mapped_response(TeapotError {
                    body: response.body_text_lossy(),
                }))
            } else {
                None
            }
        })
        .build()?;

    match client.status_text(418).await {
        Ok(body) => println!("unexpected success: {body}"),
        Err(Error::MappedResponse(source)) => {
            println!("custom error: {source}");
        }
        Err(error) => return Err(error),
    }

    Ok(())
}
