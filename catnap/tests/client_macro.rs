#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use catnap::{QueryParamStyle, Response, RestClientBuilder, Result, rest_client};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct User {
    id: String,
}

#[derive(Debug, Serialize)]
struct NewUser {
    name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename = "user")]
struct XmlUser {
    id: String,
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

    #[post("/xml")]
    #[catnap::consumes("application/xml")]
    #[catnap::produces("application/xml")]
    async fn create_xml(&self, user: &XmlUser) -> Result<XmlUser>;
}

#[rest_client]
trait Search {
    #[get("/search")]
    async fn search(&self, #[query("tag")] tags: Vec<&str>) -> Result<()>;
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

#[tokio::test]
async fn generated_client_uses_query_param_style_for_collections() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
    let address = listener.local_addr().expect("read local server address");
    let (line_sender, line_receiver) = mpsc::channel();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept test request");
        let mut request_line = String::new();
        {
            let mut reader = BufReader::new(&mut stream);
            reader
                .read_line(&mut request_line)
                .expect("read request line");
        }
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .expect("write test response");
        line_sender.send(request_line).expect("send request line");
    });

    let client = SearchClient::builder()
        .base_url(format!("http://{address}"))?
        .query_param_style(QueryParamStyle::ArrayPairs)
        .build()?;

    client.search(vec!["rust", "http"]).await?;

    let request_line = line_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("receive request line");
    server.join().expect("join test server");

    assert_eq!(
        request_line.trim_end(),
        "GET /search?tag%5B%5D=rust&tag%5B%5D=http HTTP/1.1"
    );

    Ok(())
}
