#[allow(unused_imports)]
use catnap::get;
use catnap::{Response, Result, rest_client};
use serde::Deserialize;

mod support;

#[derive(Debug, Deserialize)]
struct Slideshow {
    slideshow: SlideshowDetails,
}

#[derive(Debug, Deserialize)]
struct SlideshowDetails {
    title: String,
    author: String,
    slides: Vec<Slide>,
}

#[derive(Debug, Deserialize)]
struct Slide {
    title: String,
}

#[rest_client]
trait HttpBin {
    #[get("/json")]
    async fn json(&self) -> Result<Slideshow>;

    #[get("/headers")]
    async fn headers(&self) -> Result<Response>;
}

#[tokio::main]
async fn main() -> Result<()> {
    support::init_tracing();

    let client = HttpBinClient::builder()
        .base_url("https://httpbin.io")?
        .header("User-Agent", "catnap-example")?
        .build()?;

    let slideshow = client.json().await?;
    println!(
        "{} by {} has {} slides",
        slideshow.slideshow.title,
        slideshow.slideshow.author,
        slideshow.slideshow.slides.len()
    );

    if let Some(first_slide) = slideshow.slideshow.slides.first() {
        println!("first slide: {}", first_slide.title);
    }

    let headers = client.headers().await?;
    println!("headers endpoint returned HTTP {}", headers.status());

    Ok(())
}
