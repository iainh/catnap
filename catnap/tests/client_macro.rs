#![allow(dead_code)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use catnap::{
    Error, MediaOperation, QueryParamStyle, Response, RestClientBuilder, Result, rest_client,
};
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
    #[produces("text/plain")]
    async fn name(&self, #[path("id")] id: &str) -> Result<String>;

    #[post("/{id}/name")]
    #[consumes("text/plain")]
    async fn rename(&self, #[path("id")] id: &str, name: &str) -> Result<()>;

    #[post("/xml")]
    #[consumes("application/xml")]
    #[produces("application/xml")]
    async fn create_xml(&self, user: &XmlUser) -> Result<XmlUser>;

    #[get("/{id}/inferred")]
    async fn inferred(&self, #[path] id: &str, #[query] include: &str) -> Result<Response>;

    #[get("/{path}/shadow")]
    async fn shadowed_names(&self, #[path] path: &str, #[query] request: &str) -> Result<Response>;
}

#[rest_client]
trait Search {
    #[get("/search")]
    async fn search(&self, #[query("tag")] tags: Vec<&str>) -> Result<()>;

    #[get("/search")]
    async fn search_slice(&self, #[query("tag")] tags: &[&str]) -> Result<()>;

    #[get("/search")]
    async fn search_map(
        &self,
        #[query_map] parameters: &BTreeMap<String, Vec<String>>,
    ) -> Result<()>;
}

#[rest_client]
trait UnsupportedMedia {
    #[get("/download")]
    #[produces("application/octet-stream")]
    async fn download(&self) -> Result<Vec<u8>>;
}

#[rest_client(config_key = "users-api")]
trait Configured {
    #[get("/ping")]
    async fn ping(&self) -> Result<Response>;
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn generated_client_can_be_built() -> Result<()> {
    let _client = UsersClient::builder()
        .base_url("https://api.example.com")?
        .header("User-Agent", "catnap-test")?
        .build()?;

    let _builder: RestClientBuilder<UsersClient> = UsersClient::builder();
    assert_eq!(UsersClient::CONFIG_KEY, "Users");
    Ok(())
}

#[tokio::test]
async fn generated_client_loads_config_key_from_env() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
    let client = {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        unsafe {
            std::env::set_var("CATNAP_USERS_API_URL", server.base_url());
        }
        let client = ConfiguredClient::from_env()?;
        unsafe {
            std::env::remove_var("CATNAP_USERS_API_URL");
        }
        client
    };

    assert_eq!(ConfiguredClient::CONFIG_KEY, "users-api");

    let response = client.ping().await?;
    assert_eq!(response.status(), catnap::http::StatusCode::NO_CONTENT);

    let request = server.request();
    assert_eq!(request.line, "GET /ping HTTP/1.1");

    Ok(())
}

#[tokio::test]
async fn generated_client_uses_query_param_style_for_collections() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = SearchClient::builder()
        .base_url(server.base_url())?
        .query_param_style(QueryParamStyle::ArrayPairs)
        .build()?;

    client.search(vec!["rust", "http"]).await?;

    let request = server.request();
    assert_eq!(
        request.line,
        "GET /search?tag%5B%5D=rust&tag%5B%5D=http HTTP/1.1"
    );

    Ok(())
}

#[tokio::test]
async fn generated_client_uses_comma_separated_query_param_style_for_slices() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = SearchClient::builder()
        .base_url(server.base_url())?
        .query_param_style(QueryParamStyle::CommaSeparated)
        .build()?;

    client.search_slice(&["rust", "http"]).await?;

    let request = server.request();
    assert_eq!(request.line, "GET /search?tag=rust%2Chttp HTTP/1.1");

    Ok(())
}

#[tokio::test]
async fn generated_client_defaults_to_multi_pair_query_param_style() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = SearchClient::builder()
        .base_url(server.base_url())?
        .build()?;

    client.search(vec!["rust", "http"]).await?;

    let request = server.request();
    assert_eq!(request.line, "GET /search?tag=rust&tag=http HTTP/1.1");

    Ok(())
}

#[tokio::test]
async fn generated_client_flattens_dynamic_query_maps() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = SearchClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let parameters = BTreeMap::from([
        ("page".to_owned(), vec!["1".to_owned()]),
        ("tag".to_owned(), vec!["rust".to_owned(), "http".to_owned()]),
    ]);
    client.search_map(&parameters).await?;

    let request = server.request();
    assert_eq!(
        request.line,
        "GET /search?page=1&tag=rust&tag=http HTTP/1.1"
    );

    Ok(())
}

#[tokio::test]
async fn generated_client_applies_query_param_style_to_dynamic_query_maps() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = SearchClient::builder()
        .base_url(server.base_url())?
        .query_param_style(QueryParamStyle::CommaSeparated)
        .build()?;

    let parameters =
        BTreeMap::from([("tag".to_owned(), vec!["rust".to_owned(), "http".to_owned()])]);
    client.search_map(&parameters).await?;

    let request = server.request();
    assert_eq!(request.line, "GET /search?tag=rust%2Chttp HTTP/1.1");

    Ok(())
}

#[tokio::test]
async fn generated_get_joins_paths_query_params_and_headers() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n[]",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .header("User-Agent", "catnap-test")?
        .build()?;

    let users = client.list(3).await?;

    let request = server.request();
    assert!(users.is_empty());
    assert_eq!(request.line, "GET /users?page=3 HTTP/1.1");
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(request.header("user-agent"), Some("catnap-test"));
    assert!(request.body.is_empty());

    Ok(())
}

#[tokio::test]
async fn generated_path_params_are_percent_encoded_segments() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nAlice",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let name = client.name("a/b c").await?;

    let request = server.request();
    assert_eq!(name, "Alice");
    assert_eq!(request.line, "GET /users/a%2Fb%20c/name HTTP/1.1");
    assert_eq!(request.header("accept"), Some("text/plain"));

    Ok(())
}

#[tokio::test]
async fn generated_post_sends_headers_and_json_body() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n");

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let response = client
        .create("Bearer token", &NewUser { name: "Ada".into() })
        .await?;

    let request = server.request();
    assert_eq!(response.status(), catnap::http::StatusCode::CREATED);
    assert_eq!(request.line, "POST /users HTTP/1.1");
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(request.header("authorization"), Some("Bearer token"));
    assert_eq!(request.header("content-type"), Some("application/json"));

    let body: serde_json::Value = serde_json::from_str(&request.body).expect("valid JSON body");
    assert_eq!(body, serde_json::json!({ "name": "Ada" }));

    Ok(())
}

#[tokio::test]
async fn generated_client_infers_path_and_query_names() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let response = client.inferred("42", "roles").await?;

    let request = server.request();
    assert_eq!(response.status(), catnap::http::StatusCode::NO_CONTENT);
    assert_eq!(
        request.line,
        "GET /users/42/inferred?include=roles HTTP/1.1"
    );

    Ok(())
}

#[tokio::test]
async fn generated_client_handles_parameters_named_like_generated_locals() -> Result<()> {
    let server = TestServer::spawn("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let response = client.shadowed_names("a/b", "trace").await?;

    let request = server.request();
    assert_eq!(response.status(), catnap::http::StatusCode::NO_CONTENT);
    assert_eq!(
        request.line,
        "GET /users/a%2Fb/shadow?request=trace HTTP/1.1"
    );

    Ok(())
}

#[tokio::test]
async fn typed_responses_return_http_errors_with_bodies() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nmissing user",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let error = client.get("missing").await.expect_err("404 should fail");

    let request = server.request();
    assert_eq!(request.line, "GET /users/missing HTTP/1.1");
    assert!(matches!(
        error,
        Error::Http {
            status: catnap::http::StatusCode::NOT_FOUND,
            ref body,
        } if body == "missing user"
    ));

    Ok(())
}

#[tokio::test]
async fn default_response_exception_mapper_allows_3xx_typed_responses() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 302 Found\r\nContent-Type: application/json\r\nContent-Length: 14\r\n\r\n{\"id\":\"moved\"}",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .build()?;

    let user = client.get("moved").await?;

    let request = server.request();
    assert_eq!(request.line, "GET /users/moved HTTP/1.1");
    assert_eq!(user.id, "moved");

    Ok(())
}

#[tokio::test]
async fn default_response_exception_mapper_can_be_disabled() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\n{\"id\":\"missing\"}",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .disable_default_response_exception_mapper()
        .build()?;

    let user = client.get("missing").await?;

    let request = server.request();
    assert_eq!(request.line, "GET /users/missing HTTP/1.1");
    assert_eq!(user.id, "missing");

    Ok(())
}

#[tokio::test]
async fn custom_response_exception_mappers_run_by_priority() -> Result<()> {
    let server = TestServer::spawn(
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 30\r\nContent-Type: text/plain\r\nContent-Length: 9\r\n\r\nslow down",
    );

    let client = UsersClient::builder()
        .base_url(server.base_url())?
        .response_exception_mapper(100, |response| {
            if response.status() == catnap::http::StatusCode::TOO_MANY_REQUESTS {
                Some(Error::mapped_response(RateLimited {
                    retry_after: response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    body: response.body_text_lossy(),
                }))
            } else {
                None
            }
        })
        .response_exception_mapper(500, |response| {
            if response.status().as_u16() >= 400 {
                Some(Error::mapped_response(FallbackMappedResponse))
            } else {
                None
            }
        })
        .build()?;

    let error = client.get("limited").await.expect_err("429 should map");

    let request = server.request();
    assert_eq!(request.line, "GET /users/limited HTTP/1.1");
    assert!(matches!(
        error,
        Error::MappedResponse(ref source)
            if source.to_string() == "rate limited; retry after 30: slow down"
    ));

    Ok(())
}

#[tokio::test]
async fn unsupported_response_media_type_fails_before_sending() -> Result<()> {
    let client = UnsupportedMediaClient::builder()
        .base_url("http://127.0.0.1:9")?
        .build()?;

    let error = client
        .download()
        .await
        .expect_err("unsupported response media type should fail");

    assert!(matches!(
        error,
        Error::UnsupportedMediaType {
            operation: MediaOperation::ResponseDeserialization,
            media_type: "application/octet-stream",
        }
    ));

    Ok(())
}

#[derive(Debug)]
struct RateLimited {
    retry_after: String,
    body: String,
}

impl fmt::Display for RateLimited {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "rate limited; retry after {}: {}",
            self.retry_after, self.body
        )
    }
}

impl StdError for RateLimited {}

#[derive(Debug)]
struct FallbackMappedResponse;

impl fmt::Display for FallbackMappedResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("fallback response mapper")
    }
}

impl StdError for FallbackMappedResponse {}

struct TestServer {
    base_url: String,
    request_receiver: mpsc::Receiver<CapturedRequest>,
    handle: thread::JoinHandle<()>,
}

impl TestServer {
    fn spawn(response: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
        let address = listener.local_addr().expect("read local server address");
        let (request_sender, request_receiver) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let request = read_request(&mut stream);
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
            request_sender.send(request).expect("send request");
        });

        Self {
            base_url: format!("http://{address}"),
            request_receiver,
            handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn request(self) -> CapturedRequest {
        let request = self
            .request_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("receive request");
        self.handle.join().expect("join test server");
        request
    }
}

struct CapturedRequest {
    line: String,
    headers: BTreeMap<String, String>,
    body: String,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
    let mut request_line = String::new();
    let mut headers = BTreeMap::new();
    let mut body = Vec::new();

    {
        let mut reader = BufReader::new(stream);
        reader
            .read_line(&mut request_line)
            .expect("read request line");

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request header");
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }

            let (name, value) = trimmed.split_once(':').expect("valid request header");
            headers.insert(name.to_ascii_lowercase(), value.trim_start().to_owned());
        }

        if let Some(content_length) = headers
            .get("content-length")
            .map(|value| value.parse::<usize>().expect("valid content length"))
        {
            body.resize(content_length, 0);
            reader.read_exact(&mut body).expect("read request body");
        }
    }

    CapturedRequest {
        line: request_line.trim_end().to_owned(),
        headers,
        body: String::from_utf8(body).expect("UTF-8 request body"),
    }
}
