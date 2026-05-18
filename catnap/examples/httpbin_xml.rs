use catnap::{Result, rest_client};
#[allow(unused_imports)]
use catnap::{get, produces};
use serde::Deserialize;

mod support;

#[derive(Debug, Deserialize)]
struct Slideshow {
    #[serde(rename = "@title")]
    title: String,
    #[serde(rename = "@author")]
    author: String,
    slide: Vec<Slide>,
}

#[derive(Debug, Deserialize)]
struct Slide {
    title: String,
}

#[rest_client]
trait HttpBin {
    #[get("/xml")]
    #[produces("application/xml")]
    async fn xml(&self) -> Result<Slideshow>;
}

#[tokio::main]
async fn main() -> Result<()> {
    support::init_tracing();

    let client = HttpBinClient::builder()
        .base_url("https://httpbin.io")?
        .build()?;

    let slideshow = client.xml().await?;
    println!(
        "{} by {} has {} slides",
        slideshow.title,
        slideshow.author,
        slideshow.slide.len()
    );

    if let Some(first_slide) = slideshow.slide.first() {
        println!("first slide: {}", first_slide.title);
    }

    Ok(())
}
