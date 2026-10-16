//! End-to-end tests: run the built `parcel` binary against a wiremock server
//! that speaks the `/api/v1` contract.
//!
//! wiremock serves from its own thread, so the blocking `Command::output()`
//! calls below don't starve it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use wiremock::matchers::{
    body_json, body_partial_json, header, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const ID: &str = "abcdefghijkm";
const TOKEN: &str = "pcl_test_token";

// ---------------------------------------------------------------- harness

struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("home")).unwrap();
        Sandbox { _dir: dir, root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    fn write(&self, rel: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// A `parcel` command isolated from the developer's environment.
    fn parcel(&self, server: &str) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_parcel"));
        for var in [
            "PARCEL_SERVER",
            "PARCEL_PROFILE",
            "PARCEL_TOKEN",
            "XDG_CONFIG_HOME",
            "SSH_CONNECTION",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("HOME", self.home())
            .env("PARCEL_CONFIG_DIR", self.config_dir())
            .env("PARCEL_SERVER", server)
            .current_dir(&self.root)
            .stdin(Stdio::null());
        cmd
    }

    /// Store a token for `server` in the default profile, as `login` would.
    fn save_login(&self, server: &str, token: &str) {
        fs::create_dir_all(self.config_dir()).unwrap();
        fs::write(
            self.config_path(),
            format!("[profiles.default]\nserver = \"{server}\"\ntoken = \"{token}\"\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.config_path(), fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("failed to run the parcel binary")
}

/// Run with `input` on stdin (the sandbox default is no stdin at all).
fn run_with_stdin(cmd: &mut Command, input: &str) -> Output {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run the parcel binary");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[track_caller]
fn assert_code(out: &Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "unexpected exit status\n--- stdout\n{}\n--- stderr\n{}",
        stdout(out),
        stderr(out)
    );
}

// ---------------------------------------------------------------- fixtures

fn summary(server: &str, version: i32) -> Value {
    json!({
        "id": ID,
        "title": "Q3 report",
        "description": "",
        "visibility": "private",
        "current_version": version,
        "entry": "index.html",
        "file_count": 2,
        "total_bytes": 2048,
        "created_at": "2026-10-01T12:00:00Z",
        "updated_at": "2026-10-15T12:00:00Z",
        "url": format!("{server}/p/{ID}"),
        "open_comments": 2,
    })
}

fn version_detail(version: i32, files: Value) -> Value {
    json!({
        "version": version,
        "entry": "index.html",
        "label": null,
        "file_count": files.as_array().unwrap().len(),
        "total_bytes": 2048,
        "created_at": "2026-10-15T12:00:00Z",
        "files": files,
    })
}

fn publish_result(server: &str, version: i32) -> Value {
    json!({
        "parcel": summary(server, version),
        "version": version_detail(version, json!([])),
    })
}

fn limits() -> Value {
    json!({
        "max_entry_bytes": 16 * 1024 * 1024,
        "max_file_bytes": 15 * 1024 * 1024,
        "max_upload_bytes": 64 * 1024 * 1024,
        "max_files_per_upload": 256,
        "max_files_per_version": 512,
        "max_version_bytes": 256 * 1024 * 1024,
        "user_quota_bytes": 2_i64 * 1024 * 1024 * 1024,
    })
}

fn server_info(server: &str, limits: Value) -> Value {
    json!({
        "name": "parcel",
        "version": "0.1.0",
        "app_origin": server,
        "content_origin": "http://content.invalid",
        "auth": {"mode": "dev", "issuer": "", "client_id": "parcel", "audience": "parcel"},
        "limits": limits,
        "allowed_extensions": ["html", "js", "css"],
    })
}

fn error_body(message: &str, kind: &str) -> Value {
    json!({"error": {"message": message, "type": kind}})
}

async fn mount_info(server: &MockServer, limits: Value) {
    Mock::given(method("GET"))
        .and(path("/api/v1/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(server_info(&server.uri(), limits)))
        .mount(server)
        .await;
}

async fn requests_to(server: &MockServer, verb: &str, url_path: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == url_path)
        .collect()
}

// ---------------------------------------------------------------- multipart

#[derive(Debug)]
struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

fn disposition_param(disposition: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = disposition.find(&marker)? + marker.len();
    let rest = &disposition[start..];
    Some(rest[..rest.find('"')?].to_string())
}

/// A small multipart/form-data parser, enough to assert what the CLI sent.
fn parse_multipart(request: &Request) -> Vec<Part> {
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .unwrap()
        .trim_matches('"');
    let delimiter = format!("--{boundary}").into_bytes();
    let body = &request.body;

    let mut parts = Vec::new();
    let mut pos = find(body, &delimiter, 0).expect("no opening boundary") + delimiter.len();
    loop {
        if body[pos..].starts_with(b"--") {
            break; // closing delimiter
        }
        pos += 2; // CRLF after the delimiter
        let headers_end = find(body, b"\r\n\r\n", pos).unwrap();
        let headers = String::from_utf8(body[pos..headers_end].to_vec()).unwrap();
        let next = find(body, &delimiter, headers_end).unwrap();
        let content = body[headers_end + 4..next - 2].to_vec(); // strip CRLF before delimiter

        let mut disposition = String::new();
        let mut part_type = None;
        for line in headers.split("\r\n") {
            let (key, value) = line.split_once(':').unwrap();
            match key.to_ascii_lowercase().as_str() {
                "content-disposition" => disposition = value.trim().to_string(),
                "content-type" => part_type = Some(value.trim().to_string()),
                _ => {}
            }
        }
        parts.push(Part {
            name: disposition_param(&disposition, "name").unwrap(),
            filename: disposition_param(&disposition, "filename"),
            content_type: part_type,
            body: content,
        });
        pos = next + delimiter.len();
    }
    parts
}

// ---------------------------------------------------------------- publish

#[tokio::test]
async fn publish_a_single_file_creates_a_parcel() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let html = b"<!doctype html><title>Q3</title><h1>Q3</h1>";
    let page = sandbox.write("work/report.html", html);

    mount_info(&server, limits()).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/parcels"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 1)))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .args(["--title", "Q3 report", "--label", "first draft", "--public"]));
    assert_code(&out, 0);
    let text = stdout(&out);
    assert!(text.contains(&format!("{}/p/{ID}", server.uri())), "{text}");
    assert!(text.contains(&format!("id:      {ID}")), "{text}");
    assert!(
        stderr(&out).contains("Uploading 1 file"),
        "{}",
        stderr(&out)
    );

    let posts = requests_to(&server, "POST", "/api/v1/parcels").await;
    let parts = parse_multipart(&posts[0]);
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert_eq!(parts[0].name, "metadata");
    assert_eq!(parts[0].filename, None);
    let metadata: Value = serde_json::from_slice(&parts[0].body).unwrap();
    assert_eq!(
        metadata,
        json!({
            "title": "Q3 report",
            "visibility": "public",
            "entry": "report.html",
            "label": "first draft",
        })
    );
    assert_eq!(parts[1].name, "file");
    assert_eq!(parts[1].filename.as_deref(), Some("report.html"));
    assert_eq!(
        parts[1].content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(parts[1].body, html);
}

#[tokio::test]
async fn publish_a_directory_as_a_new_version() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    sandbox.write("site/index.html", b"<h1>v2</h1>");
    sandbox.write("site/assets/app.js", b"console.log(1)");
    sandbox.write("site/assets/img/logo.svg", b"<svg/>");
    sandbox.write("site/.env", b"SECRET=hunter2");
    sandbox.write("site/.git/HEAD", b"ref: main");
    sandbox.write("site/node_modules/lib/index.js", b"x");
    sandbox.save_login(&server.uri(), TOKEN);

    mount_info(&server, limits()).await;
    let response = publish_result(&server.uri(), 2);
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/parcels/{ID}/versions")))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(201).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;

    // A viewer URL works as the parcel reference; the token comes from config.
    let out = run(sandbox.parcel(&server.uri()).args([
        "publish",
        "site",
        "--update",
        &format!("{}/p/{ID}/v/1", server.uri()),
        "--base-version",
        "1",
        "--json",
    ]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, response);

    let posts = requests_to(&server, "POST", &format!("/api/v1/parcels/{ID}/versions")).await;
    let parts = parse_multipart(&posts[0]);
    let metadata: Value = serde_json::from_slice(&parts[0].body).unwrap();
    assert_eq!(metadata, json!({"mode": "replace", "base_version": 1}));
    let files: Vec<(&str, &str)> = parts[1..]
        .iter()
        .map(|p| {
            assert_eq!(p.name, "file");
            (
                p.filename.as_deref().unwrap(),
                p.content_type.as_deref().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        files,
        vec![
            ("assets/app.js", "text/javascript; charset=utf-8"),
            ("assets/img/logo.svg", "image/svg+xml"),
            ("index.html", "text/html; charset=utf-8"),
        ]
    );
}

#[tokio::test]
async fn publish_rejects_disallowed_files_before_any_request() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    sandbox.write("site/index.html", b"<h1>x</h1>");
    sandbox.write("site/tool.exe", b"MZ");

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["publish", "site"]));
    assert_code(&out, 1);
    let err = stderr(&out);
    assert!(err.starts_with("error: cannot publish 1 file:"), "{err}");
    assert!(err.contains("tool.exe: file type not allowed"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn publish_checks_the_server_limits() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    sandbox.write("site/index.html", b"<h1>x</h1>");
    sandbox.write("site/video.mp4", &[0u8; 64]);

    let mut small = limits();
    small["max_file_bytes"] = json!(32);
    mount_info(&server, small).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/parcels"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["publish", "site"]));
    assert_code(&out, 1);
    assert!(
        stderr(&out).contains("video.mp4 is 64 B; the per-file limit is 32 B"),
        "{}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------- list / info / cat / pull

#[tokio::test]
async fn list_prints_a_table_or_the_raw_json() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    // Hand-written so the key order ("total" first) differs from the sorted
    // order a serde_json::Value would print; --json must keep the server's.
    let raw = format!(
        r#"{{"total":7,"parcels":[{}]}}"#,
        serde_json::to_string(&summary(&server.uri(), 3)).unwrap()
    );
    let body: Value = serde_json::from_str(&raw).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .and(query_param("q", "q3 report"))
        .and(query_param("limit", "1"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw, "application/json"))
        .expect(2)
        .mount(&server)
        .await;

    let args = ["list", "--query", "q3 report", "--limit", "1"];
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(args));
    assert_code(&out, 0);
    let text = stdout(&out);
    let mut lines = text.lines();
    let header_line = lines.next().unwrap();
    assert!(header_line.starts_with("ID"), "{text}");
    assert!(header_line.contains("VISIBILITY"), "{text}");
    assert!(header_line.ends_with("COMMENTS"), "{text}");
    let row = lines.next().unwrap();
    for cell in [ID, "Q3 report", "private", "v3", "2.0 KiB", "2 open"] {
        assert!(row.contains(cell), "missing {cell:?} in {row:?}");
    }
    assert!(text.contains("Showing 1–1 of 7"), "{text}");

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(args)
        .arg("--json"));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, body);
    // Pretty-printed, keys in the server's order.
    assert!(
        stdout(&out).starts_with("{\n  \"total\": 7,\n  \"parcels\": [\n"),
        "{}",
        stdout(&out)
    );
}

#[tokio::test]
async fn info_reports_metadata_versions_and_files() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let detail = json!({
        "parcel": summary(&server.uri(), 2),
        "is_owner": true,
        "versions": [
            {"version": 2, "entry": "index.html", "label": "final", "file_count": 2,
             "total_bytes": 2048, "created_at": "2026-10-15T12:00:00Z"},
            {"version": 1, "entry": "index.html", "label": null, "file_count": 1,
             "total_bytes": 1024, "created_at": "2026-10-01T12:00:00Z"},
        ],
        "files": [
            {"path": "index.html", "size_bytes": 1024, "content_type": "text/html; charset=utf-8", "sha256": "aa"},
            {"path": "assets/app.js", "size_bytes": 1024, "content_type": "text/javascript; charset=utf-8", "sha256": "bb"},
        ],
    });
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["info", ID]));
    assert_code(&out, 0);
    let text = stdout(&out);
    for needle in [
        "Q3 report",
        &format!("url:         {}/p/{ID}", server.uri()),
        "visibility:  private",
        "final (current)",
        &format!("comments:    2 open; see `parcel comments list {ID}`"),
        "assets/app.js",
        "text/javascript",
    ] {
        assert!(text.contains(needle), "missing {needle:?}\n{text}");
    }
}

#[tokio::test]
async fn cat_streams_raw_bytes() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let data: Vec<u8> = vec![0x00, 0xff, 0xfe, b'{', b'}', 0x80];
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/parcels/{ID}/versions/latest/files/assets/my%20data.json"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .expect(1)
        .mount(&server)
        .await;
    // Default path: the entry of the current version, pinned by number.
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "parcel": summary(&server.uri(), 3),
            "is_owner": false,
            "versions": [],
            "files": [],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/parcels/{ID}/versions/3/files/index.html"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_string("<h1>hi</h1>"))
        .expect(1)
        .mount(&server)
        .await;

    // Signed out: reads of public parcels need no token.
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["cat", ID, "assets/my data.json"]));
    assert_code(&out, 0);
    assert_eq!(out.stdout, data);

    let out = run(sandbox
        .parcel(&server.uri())
        .args(["cat", &format!("{}/p/{ID}", server.uri())]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "<h1>hi</h1>");

    for request in server.received_requests().await.unwrap() {
        assert!(request.headers.get("authorization").is_none());
    }
}

#[tokio::test]
async fn pull_downloads_a_version_and_refuses_to_overwrite() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let files = json!([
        {"path": "index.html", "size_bytes": 11, "content_type": "text/html; charset=utf-8", "sha256": "aa"},
        {"path": "assets/app.js", "size_bytes": 2, "content_type": "text/javascript; charset=utf-8", "sha256": "bb"},
    ]);
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}/versions/2")))
        .respond_with(ResponseTemplate::new(200).set_body_json(version_detail(2, files)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/parcels/{ID}/versions/2/files/index.html"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_string("<h1>hi</h1>"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/parcels/{ID}/versions/2/files/assets/app.js"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_string("1;"))
        .mount(&server)
        .await;

    let pull = |extra: &[&str]| {
        let mut cmd = sandbox.parcel(&server.uri());
        cmd.args(["pull", ID, "out", "--version", "2"]).args(extra);
        run(&mut cmd)
    };
    let out = pull(&[]);
    assert_code(&out, 0);
    assert!(stdout(&out).contains("Pulled 2 files"), "{}", stdout(&out));
    assert_eq!(
        fs::read_to_string(sandbox.root.join("out/index.html")).unwrap(),
        "<h1>hi</h1>"
    );
    assert_eq!(
        fs::read_to_string(sandbox.root.join("out/assets/app.js")).unwrap(),
        "1;"
    );

    fs::write(sandbox.root.join("out/index.html"), "local edit").unwrap();
    let out = pull(&[]);
    assert_code(&out, 1);
    assert!(
        stderr(&out).contains("refusing to overwrite"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        fs::read_to_string(sandbox.root.join("out/index.html")).unwrap(),
        "local edit"
    );

    let out = pull(&["--force"]);
    assert_code(&out, 0);
    assert_eq!(
        fs::read_to_string(sandbox.root.join("out/index.html")).unwrap(),
        "<h1>hi</h1>"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pull_never_writes_through_a_symlinked_directory() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let files = json!([
        {"path": "assets/app.js", "size_bytes": 2, "content_type": "text/javascript; charset=utf-8", "sha256": "bb"},
    ]);
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}/versions/2")))
        .respond_with(ResponseTemplate::new(200).set_body_json(version_detail(2, files)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/parcels/{ID}/versions/2/files/assets/app.js"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_string("1;"))
        .mount(&server)
        .await;

    // A checkout where `assets` points somewhere outside the destination.
    let outside = sandbox.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(sandbox.root.join("out")).unwrap();
    std::os::unix::fs::symlink(&outside, sandbox.root.join("out/assets")).unwrap();

    let mut cmd = sandbox.parcel(&server.uri());
    cmd.args(["pull", ID, "out", "--version", "2", "--force"]);
    let out = run(&mut cmd);
    assert_code(&out, 1);
    assert!(stderr(&out).contains("symlink"), "{}", stderr(&out));
    assert_eq!(
        fs::read_dir(&outside).unwrap().count(),
        0,
        "nothing escaped"
    );
}

// ---------------------------------------------------------------- auth

#[tokio::test]
async fn login_device_flow_saves_the_token() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let secret = "pcl_new_device_secret";
    Mock::given(method("POST"))
        .and(path("/api/v1/cli/device"))
        .and(body_partial_json(json!({})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dc_super_secret",
            "user_code": "BCDF-GHJK",
            "verification_uri": format!("{}/device", server.uri()),
            "verification_uri_complete": format!("{}/device?code=BCDF-GHJK", server.uri()),
            "expires_in": 60,
            "interval": 1,
        })))
        .expect(1)
        .mount(&server)
        .await;
    // First poll: pending. Every poll after that: approved.
    Mock::given(method("POST"))
        .and(path("/api/v1/cli/device/token"))
        .and(body_partial_json(json!({"device_code": "dc_super_secret"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "pending"})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/cli/device/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "approved",
            "token": secret,
            "email": "ada@example.com",
        })))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .args(["login", "--no-browser"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!("Logged in to {} as ada@example.com\n", server.uri())
    );
    let err = stderr(&out);
    assert!(err.contains("BCDF-GHJK"), "{err}");
    assert!(err.contains("/device?code=BCDF-GHJK"), "{err}");
    for leaked in [secret, "dc_super_secret"] {
        assert!(
            !stdout(&out).contains(leaked) && !err.contains(leaked),
            "{leaked} printed"
        );
    }

    let start = requests_to(&server, "POST", "/api/v1/cli/device").await;
    let body: Value = serde_json::from_slice(&start[0].body).unwrap();
    assert!(
        body["client_name"]
            .as_str()
            .unwrap()
            .starts_with("parcel CLI"),
        "{body}"
    );
    assert_eq!(
        requests_to(&server, "POST", "/api/v1/cli/device/token")
            .await
            .len(),
        2
    );

    let config = fs::read_to_string(sandbox.config_path()).unwrap();
    assert!(
        config.contains(&format!("server = \"{}\"", server.uri())),
        "{config}"
    );
    assert!(
        config.contains(&format!("token = \"{secret}\"")),
        "{config}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(sandbox.config_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The saved token is used from then on, and `config show` never prints it.
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .and(header("authorization", format!("Bearer {secret}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "u1",
            "email": "ada@example.com",
            "name": "Ada",
            "usage": {"parcel_count": 2, "version_count": 5, "stored_bytes": 3_145_728},
            "limits": limits(),
        })))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(sandbox.parcel(&server.uri()).arg("whoami"));
    assert_code(&out, 0);
    assert!(
        stdout(&out).starts_with("Ada <ada@example.com>\n"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("storage: 3.0 MiB of 2.0 GiB (2 parcels, 5 versions)"));

    let out = run(sandbox.parcel(&server.uri()).args(["config", "show"]));
    assert_code(&out, 0);
    assert!(stdout(&out).contains("token:    set (saved in the profile)"));
    assert!(!stdout(&out).contains(secret));
}

#[tokio::test]
async fn login_reports_a_denied_request() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    Mock::given(method("POST"))
        .and(path("/api/v1/cli/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dc", "user_code": "AAAA-BBBB",
            "verification_uri": "http://x.invalid/device",
            "verification_uri_complete": "http://x.invalid/device?code=AAAA-BBBB",
            "expires_in": 60, "interval": 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/cli/device/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"status": "denied"})))
        .mount(&server)
        .await;
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["login", "--no-browser"]));
    assert_code(&out, 3);
    assert!(
        stderr(&out).contains("error: the login was denied"),
        "{}",
        stderr(&out)
    );
    assert!(!sandbox.config_path().exists());
}

#[tokio::test]
async fn logout_revokes_and_forgets_the_token() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    sandbox.save_login(&server.uri(), TOKEN);
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tokens/current"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(sandbox.parcel(&server.uri()).arg("logout"));
    assert_code(&out, 0);
    let config = fs::read_to_string(sandbox.config_path()).unwrap();
    assert!(!config.contains("token"), "{config}");
    assert!(config.contains("server"), "{config}");

    let out = run(sandbox.parcel(&server.uri()).arg("list"));
    assert_code(&out, 3);
    assert!(
        stderr(&out).starts_with("error: not logged in to"),
        "{}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------- errors / delete

#[tokio::test]
async fn api_errors_map_to_messages_and_exit_codes() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(error_body("parcel not found", "Not Found")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(error_body("token revoked", "Unauthorized")),
        )
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["info", ID]));
    assert_code(&out, 4);
    assert_eq!(stderr(&out), "error: parcel not found\n");

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("list"));
    assert_code(&out, 3);
    assert_eq!(
        stderr(&out),
        "error: token revoked (run `parcel login` to sign in)\n"
    );

    let out = run(sandbox.parcel(&server.uri()).args(["info", "not/a/ref"]));
    assert_code(&out, 2);
}

#[tokio::test]
async fn delete_needs_yes_without_a_terminal() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    Mock::given(method("DELETE"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["delete", ID]));
    assert_code(&out, 2);
    assert!(stderr(&out).contains("pass --yes"), "{}", stderr(&out));
    assert!(
        requests_to(&server, "DELETE", &format!("/api/v1/parcels/{ID}"))
            .await
            .is_empty()
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["delete", ID, "--yes"]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Deleted {ID}\n"));
}

// ---------------------------------------------------------------- comments

fn person(name: &str, is_owner: bool, is_you: bool) -> Value {
    json!({"name": name, "is_owner": is_owner, "is_you": is_you})
}

fn thread(number: i32, version: i32, author: Value, body: &str, anchor: Option<Value>) -> Value {
    let mut thread = json!({
        "number": number,
        "version": version,
        "author": author,
        "body": body,
        "via": "web",
        "resolved": false,
        "resolved_at": null,
        "created_at": "2026-10-15T12:00:00Z",
        "updated_at": "2026-10-15T12:00:00Z",
        "replies": [],
    });
    if let Some(anchor) = anchor {
        thread["anchor"] = anchor;
    }
    thread
}

fn reply(number: i32, author: Value, body: &str, via: &str) -> Value {
    json!({
        "number": number,
        "author": author,
        "body": body,
        "via": via,
        "created_at": "2026-10-15T13:00:00Z",
        "updated_at": "2026-10-15T13:00:00Z",
    })
}

/// A thread whose replies include one by the caller with `number`.
fn replied_thread(number: i32, reply_number: i32) -> Value {
    let mut t = thread(number, 2, person("Sam", false, false), "Add a legend", None);
    t["replies"] = json!([reply(
        reply_number,
        person("Ada", true, true),
        "Fixed",
        "cli"
    )]);
    t
}

fn comments_path() -> String {
    format!("/api/v1/parcels/{ID}/comments")
}

/// Method and path of every request except reads, in the order received.
async fn writes(server: &MockServer) -> Vec<(String, String, Value)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() != "GET")
        .map(|r| {
            let body = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            (r.method.to_string(), r.url.path().to_string(), body)
        })
        .collect()
}

#[tokio::test]
async fn comments_list_shows_where_who_and_what() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let mut first = thread(
        1,
        2,
        person("Sam", false, false),
        "Make the bars blue.\nAnd label the axis.",
        Some(
            json!({"path": "index.html", "selector": "main > h2", "text": "Revenue by region",
                    "x": 0.5, "y": 0.2}),
        ),
    );
    first["replies"] = json!([reply(3, person("Ada", true, true), "Done in v3.", "cli")]);
    let second = thread(
        2,
        2,
        person("Sam", false, false),
        "Typo here",
        Some(json!({"path": "about.html", "selector": "main > p:nth-of-type(2)"})),
    );
    let general = thread(4, 3, person("Ada", true, true), "Ship it?", None);
    let body = json!({"comments": [first, second, general], "open": 3});
    Mock::given(method("GET"))
        .and(path(comments_path()))
        .and(query_param("status", "open"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    let all = json!({"comments": [], "open": 0});
    Mock::given(method("GET"))
        .and(path(comments_path()))
        .and(query_param("status", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(all.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["comments", "list", &format!("{}/p/{ID}/v/2", server.uri())]));
    assert_code(&out, 0);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("3 open threads on {ID}"), "{text}");
    assert!(
        lines[2].starts_with("#1 · v2 · index.html › \"Revenue by region\" · Sam · "),
        "{text}"
    );
    assert_eq!(lines[3], "    Make the bars blue.", "{text}");
    assert_eq!(lines[4], "    And label the axis.", "{text}");
    assert!(
        lines[5].starts_with("    #3 · you (owner) via cli · "),
        "{text}"
    );
    assert_eq!(lines[6], "        Done in v3.", "{text}");
    assert!(
        lines[8].starts_with("#2 · v2 · about.html › main > p:nth-of-type(2) · Sam · "),
        "{text}"
    );
    assert!(
        lines[11].starts_with("#4 · v3 · general · you (owner) · "),
        "{text}"
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["comments", "list", ID, "--status", "all", "--json"]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, all);
}

#[tokio::test]
async fn comments_add_and_reply_send_text_from_args_or_stdin() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let added = thread(5, 2, person("Ada", true, true), "Please add a legend", None);
    Mock::given(method("POST"))
        .and(path(comments_path()))
        .and(body_json(
            json!({"body": "Please add a legend", "version": 2}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(added.clone()))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(comments_path()))
        .and(body_json(
            json!({"body": "Fixed: legend added", "reply_to": 1}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(replied_thread(1, 6)))
        .expect(1)
        .mount(&server)
        .await;

    // The version comes from a /v/<n> URL, or from --version.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "comments",
            "add",
            &format!("{}/p/{ID}/v/2", server.uri()),
            "  Please add a legend ",
        ]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Added comment #5 on {ID} (v2)\n"));
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "comments",
            "add",
            ID,
            "Please add a legend",
            "--version",
            "2",
            "--json",
        ]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, added);

    let out = run_with_stdin(
        sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(["comments", "reply", ID, "1", "-"]),
        "Fixed: legend added\n",
    );
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "Replied to #1 as #6\n");

    // Blank text is refused before any request.
    let before = server.received_requests().await.unwrap().len();
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["comments", "add", ID, "   "]));
    assert_code(&out, 2);
    assert!(stderr(&out).contains("comment must be"), "{}", stderr(&out));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn comments_resolve_replies_to_each_thread_then_resolves_it() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let text = "Fixed in v4: legend added";
    for (number, reply_number) in [(1, 7), (3, 8)] {
        Mock::given(method("POST"))
            .and(path(comments_path()))
            .and(body_json(json!({"body": text, "reply_to": number})))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(replied_thread(number, reply_number)),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path(format!("{}/{number}", comments_path())))
            .and(body_json(json!({"resolved": true})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(replied_thread(number, reply_number)),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["comments", "resolve", ID, "1", "3", "1", "--reply", text]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        "Resolved #1 (replied as #7)\nResolved #3 (replied as #8)\n"
    );
    // Reply first, then resolve, one thread at a time; repeats are dropped.
    let base = comments_path();
    assert_eq!(
        writes(&server).await,
        vec![
            (
                "POST".into(),
                base.clone(),
                json!({"body": text, "reply_to": 1})
            ),
            (
                "PATCH".into(),
                format!("{base}/1"),
                json!({"resolved": true})
            ),
            (
                "POST".into(),
                base.clone(),
                json!({"body": text, "reply_to": 3})
            ),
            (
                "PATCH".into(),
                format!("{base}/3"),
                json!({"resolved": true})
            ),
        ]
    );
}

#[tokio::test]
async fn comments_reopen_delete_and_failures() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let base = comments_path();
    Mock::given(method("PATCH"))
        .and(path(format!("{base}/3")))
        .and(body_json(json!({"resolved": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(replied_thread(3, 8)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{base}/2")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{base}/1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(replied_thread(1, 7)))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{base}/2")))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(
            "only the parcel's owner or the thread's author can resolve it",
            "Forbidden",
        )))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{base}/9")))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(error_body("comment #9 not found", "Not Found")),
        )
        .mount(&server)
        .await;
    let comments = |args: &[&str]| {
        let mut cmd = sandbox.parcel(&server.uri());
        cmd.env("PARCEL_TOKEN", TOKEN).arg("comments").args(args);
        run(&mut cmd)
    };

    let out = comments(&["reopen", ID, "3"]);
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "Reopened #3\n");

    let out = comments(&["delete", ID, "2"]);
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Deleted comment #2 from {ID}\n"));

    let out = comments(&["resolve", ID, "9"]);
    assert_code(&out, 4);
    assert_eq!(stderr(&out), "error: resolving #9: comment #9 not found\n");

    // Threads before the failure stay resolved and are reported; the rest
    // are left alone.
    let out = comments(&["resolve", ID, "1", "2", "3"]);
    assert_code(&out, 1);
    assert_eq!(stdout(&out), "Resolved #1\n");
    assert!(
        stderr(&out).contains("resolving #2: only the parcel's owner"),
        "{}",
        stderr(&out)
    );
    assert!(
        requests_to(&server, "PATCH", &format!("{base}/3"))
            .await
            .iter()
            .all(
                |r| serde_json::from_slice::<Value>(&r.body).unwrap() == json!({"resolved": false})
            ),
        "#3 was not resolved after #2 failed"
    );

    // Comments need a login: exit 3 before any request.
    let before = server.received_requests().await.unwrap().len();
    let out = run(sandbox.parcel(&server.uri()).args(["comments", "list", ID]));
    assert_code(&out, 3);
    assert!(stderr(&out).contains("not logged in"), "{}", stderr(&out));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

// ---------------------------------------------------------------- data

/// A DataItem as raw JSON, so the value's key order is the server's.
fn data_item(collection: &str, key: &str, value: &str) -> String {
    format!(
        r#"{{"collection":"{collection}","key":"{key}","value":{value},"size_bytes":{},"updated_at":"2026-10-15T12:00:00Z"}}"#,
        value.len()
    )
}

fn data_path(rest: &str) -> String {
    format!("/api/v1/parcels/{ID}/data{rest}")
}

#[tokio::test]
async fn data_list_shows_collections_and_reads_every_page() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    Mock::given(method("GET"))
        .and(path(data_path("")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "revision": 7,
            "usage_bytes": 3200,
            "limit_bytes": 10 * 1024 * 1024,
            "writable": false,
            "collections": [
                {"name": "votes", "count": 3, "usage_bytes": 3200, "updated_at": "2026-10-15T12:00:00Z"},
            ],
        })))
        .mount(&server)
        .await;
    let (a, b, c) = (
        data_item("votes", "alice", r#"{"choice":"red","n":1}"#),
        data_item("votes", "bob", r#""blue""#),
        data_item("votes", "carol", "3"),
    );
    Mock::given(method("GET"))
        .and(path(data_path("/votes")))
        .and(query_param("limit", "1000"))
        .and(query_param_is_missing("after"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(r#"{{"items":[{a},{b}],"next_after":"bob"}}"#),
            "application/json",
        ))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(data_path("/votes")))
        .and(query_param("limit", "1000"))
        .and(query_param("after", "bob"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(format!(r#"{{"items":[{c}]}}"#), "application/json"),
        )
        .expect(2)
        .mount(&server)
        .await;

    // Reads need no login on a public parcel.
    let out = run(sandbox.parcel(&server.uri()).args(["data", "list", ID]));
    assert_code(&out, 0);
    let text = stdout(&out);
    assert!(
        text.starts_with("COLLECTION  ITEMS  SIZE     UPDATED\nvotes       3      3.1 KiB  "),
        "{text}"
    );
    assert!(
        text.ends_with(
            "\nUsing 3.1 KiB of the parcel's 10.0 MiB data limit. \
             Read-only: only the parcel's owner can change it.\n"
        ),
        "{text}"
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .args(["data", "list", ID, "votes"]));
    assert_code(&out, 0);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines[0].starts_with("KEY") && lines[0].ends_with("VALUE"),
        "{text}"
    );
    assert!(
        lines[1].starts_with("alice") && lines[1].ends_with(r#"{"choice":"red","n":1}"#),
        "{text}"
    );
    assert!(
        lines[2].starts_with("bob") && lines[2].ends_with(r#""blue""#),
        "{text}"
    );
    assert!(
        lines[3].starts_with("carol") && lines[3].ends_with('3'),
        "{text}"
    );

    // --json: every page in one list, items exactly as sent.
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["data", "list", ID, "votes", "--json"]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let expected: Value = serde_json::from_str(&format!(r#"{{"items":[{a},{b},{c}]}}"#)).unwrap();
    assert_eq!(printed, expected);
    assert!(
        stdout(&out).contains("\"choice\": \"red\",\n"),
        "{}",
        stdout(&out)
    );
}

#[tokio::test]
async fn data_get_prints_the_value_or_exits_4() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    // "mode" before "accent": printed in the server's order, not sorted.
    let item = data_item(
        "settings",
        "theme",
        r##"{"mode":"dark","accent":"#2563eb"}"##,
    );
    Mock::given(method("GET"))
        .and(path(data_path("/settings/theme")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(item.clone(), "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(data_path("/settings/missing")))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(error_body("no item settings/missing", "Not Found")),
        )
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["data", "get", ID, "settings", "theme"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        "{\n  \"mode\": \"dark\",\n  \"accent\": \"#2563eb\"\n}\n"
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .args(["data", "get", ID, "settings", "theme", "--json"]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, serde_json::from_str::<Value>(&item).unwrap());

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["data", "get", ID, "settings", "missing"]));
    assert_code(&out, 4);
    assert_eq!(stderr(&out), "error: no item settings/missing\n");

    let before = server.received_requests().await.unwrap().len();
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["data", "get", ID, "settings", "a b"]));
    assert_code(&out, 2);
    assert!(
        stderr(&out).contains("\"a b\": key must be"),
        "{}",
        stderr(&out)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn data_set_sends_inline_file_and_stdin_values() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let item_path = data_path("/votes/alice");
    Mock::given(method("PUT"))
        .and(path(item_path.as_str()))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            data_item("votes", "alice", r#"{"b":1,"a":2}"#),
            "application/json",
        ))
        .expect(4)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(data_path("/votes/bob")))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(
            "only the owner can change this parcel",
            "Forbidden",
        )))
        .mount(&server)
        .await;
    let file = sandbox.write("value.json", b"{\n  \"b\": 1,\n  \"a\": 2\n}\n");
    let set = |extra: &[&str]| {
        let mut cmd = sandbox.parcel(&server.uri());
        cmd.env("PARCEL_TOKEN", TOKEN)
            .args(["data", "set", ID])
            .args(extra);
        cmd
    };

    let out = run(&mut set(&["votes", "alice", r#"{"b": 1, "a": 2}"#]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Set votes/alice on {ID} (13 B)\n"));
    let out = run(&mut set(&[
        "votes",
        "alice",
        &format!("@{}", file.display()),
    ]));
    assert_code(&out, 0);
    let out = run_with_stdin(&mut set(&["votes", "alice", "-"]), "[1, 2]\n");
    assert_code(&out, 0);
    let out = run_with_stdin(&mut set(&["votes", "alice", "-", "--string"]), "hello\n");
    assert_code(&out, 0);
    let bodies: Vec<String> = requests_to(&server, "PUT", &item_path)
        .await
        .iter()
        .map(|r| String::from_utf8(r.body.clone()).unwrap())
        .collect();
    // Minified, keys in the order written.
    assert_eq!(
        bodies,
        vec![
            r#"{"b":1,"a":2}"#,
            r#"{"b":1,"a":2}"#,
            "[1,2]",
            r#""hello""#
        ]
    );

    // Bad input fails before any request.
    let before = server.received_requests().await.unwrap().len();
    let out = run(&mut set(&["votes", "alice", "hello"]));
    assert_code(&out, 2);
    assert!(stderr(&out).contains("pass --string"), "{}", stderr(&out));
    let out = run(&mut set(&["votes", "alice", "@missing.json"]));
    assert_code(&out, 1);
    assert!(
        stderr(&out).contains("reading missing.json"),
        "{}",
        stderr(&out)
    );
    let out = run(&mut set(&["no/pe", "alice", "1"]));
    assert_code(&out, 2);
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["data", "set", ID, "votes", "alice", "1"]));
    assert_code(&out, 3);
    assert_eq!(server.received_requests().await.unwrap().len(), before);

    // Only the owner writes.
    let out = run(&mut set(&["votes", "bob", "1"]));
    assert_code(&out, 1);
    assert_eq!(
        stderr(&out),
        "error: only the owner can change this parcel\n"
    );
}

#[tokio::test]
async fn data_update_sends_a_merge_patch() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let item = data_item("settings", "theme", r#"{"mode":"light"}"#);
    Mock::given(method("PATCH"))
        .and(path(data_path("/settings/theme")))
        .and(header("content-type", "application/merge-patch+json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(item.clone(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "data",
            "update",
            ID,
            "settings",
            "theme",
            r#"{"mode": "light", "accent": null}"#,
            "--json",
        ]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, serde_json::from_str::<Value>(&item).unwrap());
    let patches = requests_to(&server, "PATCH", &data_path("/settings/theme")).await;
    assert_eq!(patches[0].body, br#"{"mode":"light","accent":null}"#);

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["data", "update", ID, "settings", "theme", "[1]"]));
    assert_code(&out, 2);
    assert!(
        stderr(&out).contains("must be a JSON object"),
        "{}",
        stderr(&out)
    );
}

#[tokio::test]
async fn data_delete_and_clear() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    for p in [data_path("/votes/alice"), data_path("/votes")] {
        Mock::given(method("DELETE"))
            .and(path(p))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["data", "delete", ID, "votes", "alice"]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Deleted votes/alice from {ID}\n"));
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["data", "clear", ID, "votes"]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("Cleared votes on {ID}\n"));
}

// ---------------------------------------------------------------- skill

#[test]
fn skill_install_writes_under_home() {
    let sandbox = Sandbox::new();
    let unused = "http://127.0.0.1:9";
    let out = run(sandbox
        .parcel(unused)
        .args(["skill", "install", "--agents"]));
    assert_code(&out, 0);
    let skill = sandbox.home().join(".claude/skills/parcel/SKILL.md");
    let agents = sandbox.home().join(".agents/skills/parcel/SKILL.md");
    let shown = run(sandbox.parcel(unused).args(["skill", "show"]));
    assert_code(&shown, 0);
    assert_eq!(fs::read(&skill).unwrap(), shown.stdout);
    assert_eq!(fs::read(&agents).unwrap(), shown.stdout);
    assert!(sandbox
        .home()
        .join(".claude/skills/parcel/references/cli.md")
        .is_file());
    assert!(stdout(&out).contains(&skill.display().to_string()));

    let path = run(sandbox.parcel(unused).args(["skill", "path"]));
    assert_eq!(stdout(&path).trim(), skill.display().to_string());

    // Repo-local install goes under the working directory instead.
    let out = run(sandbox
        .parcel(unused)
        .args(["skill", "install", "--project"]));
    assert_code(&out, 0);
    assert!(Path::new(&sandbox.root)
        .join(".claude/skills/parcel/SKILL.md")
        .is_file());
}
