use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn macro_diagnostics_are_compile_time_errors() {
    let cases = [
        CompileCase::fail(
            "malformed_parameter_attribute",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list(&self, #[query(name = "page")] page: u32) -> Result<()>;
}

fn main() {}
"#,
            "expected string literal",
        ),
        CompileCase::fail(
            "multiple_parameter_bindings",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/{id}")]
    async fn get(&self, #[path("id")] #[query("id")] id: &str) -> Result<()>;
}

fn main() {}
"#,
            "may have only one of #[path()], #[query()], or #[header]",
        ),
        CompileCase::fail(
            "non_async_method",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    fn list(&self) -> Result<()>;
}

fn main() {}
"#,
            "REST client methods must be async",
        ),
        CompileCase::fail(
            "missing_receiver",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list() -> Result<()>;
}

fn main() {}
"#,
            "must take &self as the first parameter",
        ),
        CompileCase::fail(
            "get_body_argument",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list(&self, body: &str) -> Result<()>;
}

fn main() {}
"#,
            "GET and HEAD REST client methods cannot have request body arguments",
        ),
        CompileCase::fail(
            "unclosed_path_placeholder",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/{id")]
    async fn get(&self) -> Result<()>;
}

fn main() {}
"#,
            "unclosed `{` in path placeholder",
        ),
        CompileCase::fail(
            "duplicate_http_methods",
            r#"
use catnap::{get, post, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    #[post("/items")]
    async fn list(&self) -> Result<()>;
}

fn main() {}
"#,
            "may have only one HTTP method attribute",
        ),
        CompileCase::fail(
            "std_result_return",
            r#"
use catnap::{get, rest_client};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list(&self) -> std::result::Result<(), catnap::Error>;
}

fn main() {}
"#,
            "must return catnap::Result<T> with one type parameter",
        ),
        CompileCase::fail(
            "tuple_parameter_pattern",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list(&self, #[query("page")] (page): u32) -> Result<()>;
}

fn main() {}
"#,
            "must use simple identifier patterns",
        ),
        CompileCase::fail(
            "empty_path_parameter_name",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/{id}")]
    async fn get(&self, #[path("")] id: &str) -> Result<()>;
}

fn main() {}
"#,
            "#[path] parameter names must not be empty",
        ),
        CompileCase::fail(
            "invalid_header_name",
            r#"
use catnap::{get, header, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/items")]
    async fn list(&self, #[header("Bad Header")] header: &str) -> Result<()>;
}

fn main() {}
"#,
            "is not a valid HTTP header name",
        ),
        CompileCase::fail(
            "duplicate_resource_argument",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client(path = "/v1", path = "/v2")]
trait Api {
    #[get("/items")]
    async fn list(&self) -> Result<()>;
}

fn main() {}
"#,
            "duplicate `path` argument",
        ),
        CompileCase::fail(
            "missing_path_argument",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client(path = "/users/{id}")]
trait Api {
    #[get("")]
    async fn get(&self) -> Result<()>;
}

fn main() {}
"#,
            "missing #[path(\"id\")] argument for path placeholder",
        ),
        CompileCase::fail(
            "extra_path_argument",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client(path = "/users")]
trait Api {
    #[get("")]
    async fn get(&self, #[path("id")] id: &str) -> Result<()>;
}

fn main() {}
"#,
            "#[path(\"id\")] does not match a path placeholder",
        ),
        CompileCase::fail(
            "duplicate_path_placeholders",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client]
trait Api {
    #[get("/users/{id}/aliases/{id}")]
    async fn get(&self, #[path("id")] id: &str) -> Result<()>;
}

fn main() {}
"#,
            "REST client path placeholders must be unique",
        ),
        CompileCase::fail(
            "duplicate_body_arguments",
            r#"
use catnap::{post, rest_client, Result};

#[rest_client]
trait Api {
    #[post("/items")]
    async fn create(&self, first: &str, second: &str) -> Result<()>;
}

fn main() {}
"#,
            "REST client methods may have at most one unannotated body argument",
        ),
        CompileCase::fail(
            "invalid_media_type",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client(produces = "application json")]
trait Api {
    #[get("/items")]
    async fn list(&self) -> Result<()>;
}

fn main() {}
"#,
            "media types must use `type/subtype` syntax",
        ),
        CompileCase::pass(
            "qualified_catnap_attributes",
            r#"
use catnap::{rest_client, Result};

#[rest_client(path = "/items")]
trait Api {
    #[catnap::get("/{id}")]
    #[catnap::produces("text/plain")]
    async fn get(&self, #[path("id")] id: &str) -> Result<String>;

    #[catnap::post("/{id}")]
    #[catnap::consumes("text/plain")]
    async fn rename(&self, #[path("id")] id: &str, name: &str) -> Result<()>;
}

fn main() {}
"#,
        ),
        CompileCase::pass(
            "inferred_path_and_query_names",
            r#"
use catnap::{get, rest_client, Result};

#[rest_client(path = "/users/{id}")]
trait Api {
    #[get("/posts")]
    async fn list(&self, #[path()] id: &str, #[query()] page: u32) -> Result<()>;
}

fn main() {}
"#,
        ),
    ];

    let root = compile_test_root();
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove old compile-test root");
    }
    fs::create_dir_all(&root).expect("create compile-test root");

    for case in cases {
        run_case(&root, case);
    }
}

struct CompileCase {
    name: &'static str,
    source: &'static str,
    expected: Expected,
}

enum Expected {
    Pass,
    Fail(&'static str),
}

impl CompileCase {
    fn pass(name: &'static str, source: &'static str) -> Self {
        Self {
            name,
            source,
            expected: Expected::Pass,
        }
    }

    fn fail(name: &'static str, source: &'static str, stderr_contains: &'static str) -> Self {
        Self {
            name,
            source,
            expected: Expected::Fail(stderr_contains),
        }
    }
}

fn run_case(root: &Path, case: CompileCase) {
    let case_dir = root.join(case.name);
    fs::create_dir_all(case_dir.join("src")).expect("create compile-test case directory");
    fs::write(case_dir.join("Cargo.toml"), manifest()).expect("write compile-test manifest");
    fs::write(case_dir.join("src/main.rs"), case.source).expect("write compile-test source");

    let output = Command::new(cargo())
        .arg("check")
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(case_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .expect("run cargo check for compile-test case");

    let stderr = String::from_utf8_lossy(&output.stderr);
    match case.expected {
        Expected::Pass => {
            assert!(
                output.status.success(),
                "{} should compile successfully\nstderr:\n{}",
                case.name,
                stderr
            );
        }
        Expected::Fail(expected) => {
            assert!(
                !output.status.success(),
                "{} should fail to compile",
                case.name
            );
            assert!(
                stderr.contains(expected),
                "{} stderr should contain `{}`\nstderr:\n{}",
                case.name,
                expected,
                stderr
            );
        }
    }
}

fn compile_test_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join("macro-diagnostics")
}

fn manifest() -> String {
    let catnap_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    format!(
        r#"[workspace]

[package]
name = "macro-diagnostic-case"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
catnap = {{ path = "{}" }}
"#,
        toml_string(catnap_path.as_path())
    )
}

fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn toml_string(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
