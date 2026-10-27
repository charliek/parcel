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
            "PARCEL_PROJECT",
            "XDG_CONFIG_HOME",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_CONFIG_GLOBAL",
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
        "auth": {"mode": "oidc", "providers": [{"id": "okta", "label": "Acme Okta"}]},
        "allow_public": true,
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
        "access": "owner",
        "owner_name": "Ada",
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
            "access": "public",
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

    // (`not/a/ref` would be a slug now.)
    let out = run(sandbox.parcel(&server.uri()).args(["info", "not a ref"]));
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

/// A body-less POST states `Content-Length: 0`: over HTTP/1.1 Google's front
/// end in front of production answers one without it with a 411, so
/// `restore` failed there while every local test passed.
#[tokio::test]
async fn restore_states_an_empty_length() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let restore = format!("/api/v1/parcels/{ID}/versions/1/restore");
    Mock::given(method("POST"))
        .and(path(&restore))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 3)))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["restore", ID, "--version", "1"]));
    assert_code(&out, 0);
    let posts = requests_to(&server, "POST", &restore).await;
    assert_eq!(
        posts[0]
            .headers
            .get("content-length")
            .map(|v| v.to_str().unwrap()),
        Some("0")
    );
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

// ---------------------------------------------------------------- sharing

fn share_json(email: &str, role: &str, name: Option<&str>) -> Value {
    let mut share = json!({
        "email": email,
        "role": role,
        "created_at": "2026-10-15T12:00:00Z",
    });
    if let Some(name) = name {
        share["name"] = json!(name);
    }
    share
}

fn shares_path() -> String {
    format!("/api/v1/parcels/{ID}/shares")
}

async fn mount_shares(server: &MockServer, shares: Value, times: Option<u64>) {
    let mock = Mock::given(method("GET"))
        .and(path(shares_path()))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"shares": shares})));
    match times {
        Some(n) => mock.up_to_n_times(n).mount(server).await,
        None => mock.mount(server).await,
    }
}

#[tokio::test]
async fn share_puts_each_email_then_shows_who_has_access() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    for (email, times) in [("ada@example.com", 2), ("bob@example.com", 1)] {
        Mock::given(method("PUT"))
            .and(path(format!("{}/{email}", shares_path())))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .and(body_json(json!({"role": "commenter"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(share_json(
                email,
                "commenter",
                None,
            )))
            .expect(times)
            .mount(&server)
            .await;
    }
    Mock::given(method("PUT"))
        .and(path(format!("{}/x@nowhere.test", shares_path())))
        .respond_with(ResponseTemplate::new(400).set_body_json(error_body(
            "nobody at @nowhere.test can sign in to this Parcel, so sharing with them would do nothing",
            "Bad Request",
        )))
        .mount(&server)
        .await;
    // The name is the person's own (from their provider): never raw to the terminal.
    let raw = format!(
        r#"{{"shares":[{},{}]}}"#,
        share_json("ada@example.com", "commenter", Some("Ada \u{1b}[2J")),
        share_json("bob@example.com", "commenter", None),
    );
    Mock::given(method("GET"))
        .and(path(shares_path()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw.clone(), "application/json"))
        .mount(&server)
        .await;

    // Emails are normalized and deduplicated before anything is sent.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "share",
            ID,
            " Ada@Example.COM",
            "bob@example.com",
            "ada@example.com",
        ])
        .args(["--role", "commenter"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!(
            "Shared {ID} with ada@example.com (commenter)\n\
             Shared {ID} with bob@example.com (commenter)\n\
             \n\
             Shared with 2 people:\n\
             EMAIL            ROLE       NAME\n\
             ada@example.com  commenter  Ada \\u{{1b}}[2J\n\
             bob@example.com  commenter  not signed in yet\n"
        )
    );

    // --json prints only the shares as they now stand, as the server sent them.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "share",
            ID,
            "ada@example.com",
            "--role",
            "commenter",
            "--json",
        ]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed, serde_json::from_str::<Value>(&raw).unwrap());

    // A malformed email is a usage error before any request.
    let before = server.received_requests().await.unwrap().len();
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["share", ID, "ada@example.com", "not-an-email"]));
    assert_code(&out, 2);
    assert!(
        stderr(&out).contains("is not an email address"),
        "{}",
        stderr(&out)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);

    // The server's refusal reaches the user as it was written.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["share", ID, "x@nowhere.test"]));
    assert_code(&out, 1);
    assert_eq!(
        stderr(&out),
        "error: sharing with x@nowhere.test: nobody at @nowhere.test can sign in to this \
         Parcel, so sharing with them would do nothing\n"
    );

    // Sharing needs a login.
    let out = run(sandbox
        .parcel(&server.uri())
        .args(["share", ID, "a@example.com"]));
    assert_code(&out, 3);
}

#[tokio::test]
async fn unshare_removes_shared_people_and_notes_the_rest() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    // Before, then after.
    mount_shares(
        &server,
        json!([
            share_json("ada@example.com", "viewer", Some("Ada")),
            share_json("bob@example.com", "commenter", None),
        ]),
        Some(1),
    )
    .await;
    mount_shares(
        &server,
        json!([share_json("bob@example.com", "commenter", None)]),
        None,
    )
    .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{}/ada@example.com", shares_path())))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["unshare", ID, "ADA@example.com", "carol@example.com"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!(
            "Unshared {ID} with ada@example.com\n\
             \n\
             Shared with 1 person:\n\
             EMAIL            ROLE       NAME\n\
             bob@example.com  commenter  not signed in yet\n"
        )
    );
    assert_eq!(
        stderr(&out),
        format!("note: {ID} was not shared with carol@example.com\n")
    );
    // Nothing is deleted for someone who wasn't on the list.
    let deletes = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "DELETE")
        .count();
    assert_eq!(deletes, 1);
}

#[tokio::test]
async fn sharing_is_for_the_owner() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    Mock::given(method("GET"))
        .and(path(shares_path()))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(
            "only the owner can change this parcel",
            "Forbidden",
        )))
        .mount(&server)
        .await;
    for args in [&["unshare", ID, "ada@example.com"][..], &["shares", ID]] {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(args));
        assert_code(&out, 1);
        assert_eq!(
            stderr(&out),
            "error: only the owner can change this parcel\n",
            "{args:?}"
        );
    }
    assert!(requests_to(
        &server,
        "DELETE",
        &format!("{}/ada@example.com", shares_path())
    )
    .await
    .is_empty());
}

#[tokio::test]
async fn shares_lists_people_or_says_there_are_none() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_shares(&server, json!([]), Some(1)).await;
    mount_shares(
        &server,
        json!([share_json(
            "ada@example.com",
            "viewer",
            Some("Ada Lovelace")
        )]),
        None,
    )
    .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["shares", ID]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!("{ID} is not shared with anyone. Share it with `parcel share {ID} <email>`.\n")
    );

    let url = format!("{}/p/{ID}", server.uri());
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["shares", &url]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        "Shared with 1 person:\n\
         EMAIL            ROLE    NAME\n\
         ada@example.com  viewer  Ada Lovelace\n"
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["shares", ID, "--json"]));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed["shares"][0]["email"], "ada@example.com");
}

#[tokio::test]
async fn list_shared_shows_the_owner_and_role() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let mut parcel = summary(&server.uri(), 4);
    parcel["title"] = json!("Q3 \u{1b}]52;c;aGk=\u{7}report");
    let raw = format!(
        r#"{{"total":3,"parcels":[{}]}}"#,
        json!({
            "parcel": parcel,
            "owner_name": "Ada Lovelace",
            "role": "commenter",
            "shared_at": "2026-10-14T12:00:00Z",
        })
    );
    Mock::given(method("GET"))
        .and(path("/api/v1/shared"))
        .and(query_param("limit", "1"))
        .and(query_param_is_missing("q"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw.clone(), "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/shared"))
        .and(query_param_is_missing("limit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"parcels": [], "total": 0})))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["list", "--shared", "--limit", "1"]));
    assert_code(&out, 0);
    let text = stdout(&out);
    let mut lines = text.lines();
    let header_line = lines.next().unwrap();
    for column in [
        "ID", "TITLE", "OWNER", "ROLE", "VERSION", "UPDATED", "COMMENTS",
    ] {
        assert!(
            header_line.contains(column),
            "missing {column:?} in {header_line:?}"
        );
    }
    let row = lines.next().unwrap();
    for cell in [ID, "Ada Lovelace", "commenter", "v4", "2 open"] {
        assert!(row.contains(cell), "missing {cell:?} in {row:?}");
    }
    // A title someone else wrote can't drive the terminal.
    assert!(row.contains(r"Q3 \u{1b}]52;c;aGk=\u{7}report"), "{row:?}");
    assert!(!text.contains('\u{1b}'));
    assert!(text.contains("Showing 1–1 of 3"), "{text}");

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["list", "--shared", "--limit", "1", "--json"]));
    assert_code(&out, 0);
    assert!(
        stdout(&out).starts_with("{\n  \"total\": 3,\n"),
        "{}",
        stdout(&out)
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["list", "--shared"]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "Nothing has been shared with you yet.\n");
    assert!(requests_to(&server, "GET", "/api/v1/parcels")
        .await
        .is_empty());
}

#[tokio::test]
async fn info_shows_the_owner_and_your_access_on_others_parcels() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let mut parcel = summary(&server.uri(), 1);
    parcel["visibility"] = json!("internal");
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "parcel": parcel,
            "is_owner": false,
            "access": "viewer",
            "owner_name": "Ada Lovelace",
            "versions": [],
            "files": [],
        })))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["info", ID]));
    assert_code(&out, 0);
    let text = stdout(&out);
    for needle in [
        "visibility:  internal (anyone signed in to this server)",
        "owner:       Ada Lovelace",
        "access:      viewer (you can view and read comments, not comment)",
    ] {
        assert!(text.contains(needle), "missing {needle:?}\n{text}");
    }
}

#[tokio::test]
async fn internal_visibility_reaches_the_api() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("work/report.html", b"<title>Q3</title>");
    mount_info(&server, limits()).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/parcels"))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 1)))
        .expect(1)
        .mount(&server)
        .await;
    let mut internal = summary(&server.uri(), 1);
    internal["visibility"] = json!("internal");
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .and(body_json(json!({"visibility": "internal"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(internal))
        .expect(1)
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .arg("--internal"));
    assert_code(&out, 0);
    let posts = requests_to(&server, "POST", "/api/v1/parcels").await;
    let metadata: Value = serde_json::from_slice(&parse_multipart(&posts[0])[0].body).unwrap();
    assert_eq!(metadata["visibility"], "internal");

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["update", ID, "--internal"]));
    assert_code(&out, 0);
    assert!(stdout(&out).contains("(internal)"), "{}", stdout(&out));
}

#[tokio::test]
async fn public_links_turned_off_are_explained() {
    const OFF: &str = "public links are turned off on this server; use \"internal\" \
                       (everyone signed in) or share with people by email";
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("work/report.html", b"<title>Q3</title>");
    let mut info = server_info(&server.uri(), limits());
    info["allow_public"] = json!(false);
    Mock::given(method("GET"))
        .and(path("/api/v1/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(info))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(400).set_body_json(error_body(OFF, "Bad Request")))
        .mount(&server)
        .await;

    // `update` gets the server's own words.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["update", ID, "--public"]));
    assert_code(&out, 1);
    assert_eq!(stderr(&out), format!("error: {OFF}\n"));

    // `publish` knows from /info and refuses before uploading anything.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .arg("--public"));
    assert_code(&out, 1);
    let err = stderr(&out);
    assert!(err.contains("public links are turned off on"), "{err}");
    assert!(
        err.contains("--internal") && err.contains("parcel share"),
        "{err}"
    );
    assert!(requests_to(&server, "POST", "/api/v1/parcels")
        .await
        .is_empty());
}

#[tokio::test]
async fn a_paused_token_says_to_sign_in_on_the_web() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let paused = format!(
        "this token is paused: sign in to Parcel on the web ({}) to keep using it \
         (tokens need a web sign-in at least every 30 days)",
        server.uri()
    );
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(error_body(&paused, "Unauthorized")))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("list"));
    assert_code(&out, 3);
    assert_eq!(stderr(&out), format!("error: {paused}\n"));

    // Logging in with such a token passes the reason along too.
    let out = run_with_stdin(
        sandbox
            .parcel(&server.uri())
            .args(["login", "--with-token"]),
        TOKEN,
    );
    assert_code(&out, 3);
    assert!(stderr(&out).contains(&paused), "{}", stderr(&out));
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

// ---------------------------------------------------------------- tags, slugs, search, pages

/// A server with every optional feature (`ServerInfo.features`).
async fn mount_new_info(server: &MockServer) {
    let mut info = server_info(&server.uri(), limits());
    info["features"] = json!(["tags", "search", "archive", "slugs", "pages"]);
    Mock::given(method("GET"))
        .and(path("/api/v1/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(info))
        .mount(server)
        .await;
}

/// A summary as a server with slugs sends it.
fn slugged_summary(server: &str, version: i32) -> Value {
    let mut s = summary(server, version);
    s["tags"] = json!(["kind:plan", "project:roost"]);
    s["slug"] = json!("roost/plan-14");
    s["owner_handle"] = json!("ada");
    s["canonical_url"] = json!(format!("{server}/@ada/roost/plan-14"));
    s["archived_at"] = Value::Null;
    s
}

fn detail_of(summary: Value, pages: Value) -> Value {
    json!({
        "parcel": summary,
        "is_owner": true,
        "access": "owner",
        "versions": [
            {"version": 2, "entry": "index.html", "label": null, "file_count": 2,
             "total_bytes": 2048, "created_at": "2026-10-15T12:00:00Z",
             "source": {"repo": "github.com/acme/roost", "branch": "main",
                        "commit": "0123456789ab", "host": "laptop",
                        "client": "parcel-cli/0.2.0"}},
            {"version": 1, "entry": "index.html", "label": null, "file_count": 1,
             "total_bytes": 1024, "created_at": "2026-10-01T12:00:00Z"},
        ],
        "files": [],
        "pages": pages,
    })
}

fn two_pages() -> Value {
    json!([
        {"path": "index.html", "title": "Plan"},
        {"path": "notes/b.html", "title": "Build \u{1b}[2Jsettings"},
    ])
}

/// The `metadata` part of every multipart POST to `url_path`.
async fn sent_metadata(server: &MockServer, url_path: &str) -> Vec<Value> {
    requests_to(server, "POST", url_path)
        .await
        .iter()
        .map(|r| serde_json::from_slice(&parse_multipart(r)[0].body).unwrap())
        .collect()
}

async fn mount_create(server: &MockServer, status: u16, created: bool) {
    let mut body = publish_result(&server.uri(), 1);
    body["created"] = json!(created);
    Mock::given(method("POST"))
        .and(path("/api/v1/parcels"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// `git` with no user or system configuration.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit in `<sandbox>/<name>`.
fn git_repo(sandbox: &Sandbox, name: &str, origin: Option<&str>) -> PathBuf {
    let dir = sandbox.root.join(name);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
    if let Some(origin) = origin {
        git(&dir, &["remote", "add", "origin", origin]);
    }
    dir
}

#[tokio::test]
async fn publish_in_a_git_repo_adds_the_project_tag_and_a_sanitized_source() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    let repo = git_repo(
        &sandbox,
        "roost-checkout",
        Some("https://ada:ghp_CANARY7731@github.com/acme/Roost_App.git"),
    );
    fs::create_dir_all(repo.join("src/deep")).unwrap();
    mount_new_info(&server).await;
    mount_create(&server, 201, true).await;

    let publish = |extra: &[&str], cwd: &Path| {
        let mut cmd = sandbox.parcel(&server.uri());
        cmd.env("PARCEL_TOKEN", TOKEN)
            .current_dir(cwd)
            .arg("publish")
            .arg(&page)
            .args(extra);
        run(&mut cmd)
    };
    // From a subdirectory: the repository is the current directory's.
    let out = publish(&[], &repo.join("src/deep"));
    assert_code(&out, 0);
    let out = publish(&["--project", "Other Name"], &repo);
    assert_code(&out, 0);
    let mut cmd = sandbox.parcel(&server.uri());
    cmd.env("PARCEL_TOKEN", TOKEN)
        .env("PARCEL_PROJECT", "from-env")
        .current_dir(&repo)
        .arg("publish")
        .arg(&page);
    assert_code(&run(&mut cmd), 0);
    assert_code(&publish(&["--no-auto-tags"], &repo), 0);
    assert_code(&publish(&["--no-source", "--json"], &repo), 0);

    let sent = sent_metadata(&server, "/api/v1/parcels").await;
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0]["auto_tags"], json!(["project:roost-app"]));
    assert!(sent[0].get("tags").is_none(), "{}", sent[0]);
    let source = &sent[0]["source"];
    assert_eq!(source["repo"], "https://github.com/acme/Roost_App");
    assert_eq!(source["branch"], "main");
    assert_eq!(source["commit"].as_str().unwrap().len(), 12);
    assert!(source["client"]
        .as_str()
        .unwrap()
        .starts_with("parcel-cli/"));
    assert!(source.get("host").is_some(), "{source}");
    assert_eq!(sent[1]["auto_tags"], json!(["project:other-name"]));
    assert_eq!(sent[2]["auto_tags"], json!(["project:from-env"]));
    assert!(sent[3].get("auto_tags").is_none(), "{}", sent[3]);
    assert_eq!(
        sent[3]["source"]["repo"],
        "https://github.com/acme/Roost_App"
    );
    assert_eq!(sent[4]["auto_tags"], json!(["project:roost-app"]));
    assert!(sent[4].get("source").is_none(), "{}", sent[4]);
    // The credential never leaves the machine, in any request.
    for request in server.received_requests().await.unwrap() {
        let body = String::from_utf8_lossy(&request.body);
        assert!(!body.contains("CANARY"), "{body}");
        assert!(!request.url.as_str().contains("CANARY"));
    }

    // A bad --project is a usage error before anything is sent.
    let out = publish(&["--project", "no way!"], &repo);
    assert_code(&out, 2);
    assert_eq!(sent_metadata(&server, "/api/v1/parcels").await.len(), 5);
}

#[tokio::test]
async fn a_worktree_without_origin_is_named_after_the_main_checkout() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    let main = git_repo(&sandbox, "mainproj", None);
    git(
        &main,
        &["worktree", "add", "-q", "-b", "feature", "../feature-wt"],
    );
    mount_new_info(&server).await;
    mount_create(&server, 201, true).await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .current_dir(sandbox.root.join("feature-wt"))
        .arg("publish")
        .arg(&page));
    assert_code(&out, 0);
    let sent = sent_metadata(&server, "/api/v1/parcels").await;
    assert_eq!(sent[0]["auto_tags"], json!(["project:mainproj"]));
    assert_eq!(sent[0]["source"]["branch"], "feature");
    assert!(sent[0]["source"].get("repo").is_none(), "no remote");
}

#[tokio::test]
async fn outside_a_repo_only_the_machine_and_client_are_sent() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    mount_new_info(&server).await;
    mount_create(&server, 201, true).await;

    // Not a repository.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page));
    assert_code(&out, 0);
    // No git at all.
    let empty = sandbox.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let repo = git_repo(&sandbox, "repo", Some("git@github.com:acme/roost.git"));
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .env("PATH", &empty)
        .current_dir(&repo)
        .arg("publish")
        .arg(&page));
    assert_code(&out, 0);
    // A git that hangs is given up on after the time budget.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hung = sandbox.root.join("hung-git");
        fs::create_dir_all(&hung).unwrap();
        fs::write(hung.join("git"), "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(hung.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .env("PATH", &hung)
            .current_dir(&repo)
            .arg("publish")
            .arg(&page));
        assert_code(&out, 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(15));
    }

    for sent in sent_metadata(&server, "/api/v1/parcels").await {
        assert!(sent.get("auto_tags").is_none(), "{sent}");
        let source = sent["source"].as_object().unwrap();
        let mut keys: Vec<&str> = source.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert!(keys == ["client", "host"] || keys == ["client"], "{sent}");
    }
}

#[tokio::test]
async fn new_versions_send_added_tags_and_source_but_never_auto_tags() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    let repo = git_repo(&sandbox, "roost", Some("git@github.com:acme/roost.git"));
    mount_new_info(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/parcels/{ID}/versions")))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 2)))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .current_dir(&repo)
        .arg("publish")
        .arg(&page)
        .args([
            "--parcel",
            ID,
            "--tag",
            "Auth",
            "--kind",
            "plan",
            "--project",
            "x",
        ]));
    assert_code(&out, 0);
    assert!(stdout(&out).starts_with("Updated "), "{}", stdout(&out));
    let sent = sent_metadata(&server, &format!("/api/v1/parcels/{ID}/versions")).await;
    assert_eq!(sent[0]["add_tags"], json!(["auth", "kind:plan"]));
    assert_eq!(sent[0]["source"]["repo"], "ssh://github.com/acme/roost");
    assert!(sent[0].get("auto_tags").is_none(), "{}", sent[0]);
    assert!(sent[0].get("tags").is_none(), "{}", sent[0]);
}

#[tokio::test]
async fn auto_tags_are_dropped_first_at_the_cap() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    let repo = git_repo(&sandbox, "roost", None);
    mount_new_info(&server).await;
    mount_create(&server, 201, true).await;

    for n in [19, 20] {
        let tags: Vec<String> = (0..n)
            .flat_map(|i| ["--tag".into(), format!("t{i:02}")])
            .collect();
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .current_dir(&repo)
            .arg("publish")
            .arg(&page)
            .args(&tags));
        assert_code(&out, 0);
    }
    let sent = sent_metadata(&server, "/api/v1/parcels").await;
    assert_eq!(sent[0]["tags"].as_array().unwrap().len(), 19);
    assert_eq!(sent[0]["auto_tags"], json!(["project:roost"]));
    assert_eq!(sent[1]["tags"].as_array().unwrap().len(), 20);
    assert!(sent[1].get("auto_tags").is_none(), "{}", sent[1]);
}

#[tokio::test]
async fn slug_publishes_upsert_and_say_which_happened() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    mount_new_info(&server).await;
    let mut versioned = publish_result(&server.uri(), 3);
    versioned["parcel"] = slugged_summary(&server.uri(), 3);
    versioned["created"] = json!(false);
    Mock::given(method("POST"))
        .and(path("/api/v1/parcels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(versioned.clone()))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .args([
            "--slug",
            "Roost/Plan-14",
            "--base-version",
            "2",
            "--kind",
            "plan",
        ]));
    assert_code(&out, 0);
    let text = stdout(&out);
    assert!(text.starts_with("Updated \"Q3 report\""), "{text}");
    assert!(
        text.contains(&format!("url:     {}/@ada/roost/plan-14", server.uri())),
        "{text}"
    );
    assert!(text.contains("tags:    kind:plan, project:roost"), "{text}");
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .args(["--slug", "roost/plan-14", "--json"]));
    assert_code(&out, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        versioned
    );

    let sent = sent_metadata(&server, "/api/v1/parcels").await;
    assert_eq!(sent[0]["slug"], "roost/plan-14");
    assert_eq!(sent[0]["base_version"], 2);
    assert_eq!(sent[0]["tags"], json!(["kind:plan"]));

    // --slug is a create-or-version on its own; it can't be combined with
    // a target or the new-version flags.
    for extra in [
        &["--parcel", ID][..],
        &["--mode", "merge"],
        &["--remove", "a.css"],
    ] {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .arg("publish")
            .arg(&page)
            .args(["--slug", "s"])
            .args(extra));
        assert_code(&out, 2);
    }
}

#[tokio::test]
async fn an_older_server_is_never_sent_what_it_does_not_know() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let page = sandbox.write("scratch/plan.html", b"<title>Plan</title>");
    let repo = git_repo(&sandbox, "roost", Some("git@github.com:acme/roost.git"));
    // No `features`: a server from before tags.
    mount_info(&server, limits()).await;
    mount_create(&server, 201, false).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"parcels": [], "total": 0})))
        .mount(&server)
        .await;

    for (args, refusal) in [
        (
            &["list", "--tag", "plan"][..],
            "this server doesn't support tags yet",
        ),
        (
            &["list", "--project", "roost"],
            "this server doesn't support tags yet",
        ),
        (
            &["list", "--archived"],
            "this server doesn't support archived parcels yet",
        ),
        (&["list", "--all"], "this server doesn't support --all yet"),
        (
            &["list", "--shared", "-q", "x"],
            "doesn't support searching shared parcels",
        ),
        (&["tags"], "this server doesn't support tags yet"),
        (
            &["update", ID, "--slug", "x"],
            "this server doesn't support slugs yet",
        ),
        (
            &["update", ID, "--set-tags", ""],
            "this server doesn't support tags yet",
        ),
        (
            &["archive", ID],
            "this server doesn't support archiving yet",
        ),
        (
            &["info", "@ada/plan"],
            "this server doesn't support slugs yet",
        ),
        (
            &["link", ID, "--page", "b.html"],
            "this server doesn't support pages yet",
        ),
        (&["handle"], "this server doesn't support handles yet"),
    ] {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(args));
        assert_code(&out, 1);
        assert!(stderr(&out).contains(refusal), "{args:?}: {}", stderr(&out));
    }
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .args(["--tag", "x"]));
    assert_code(&out, 1);
    assert!(
        stderr(&out).contains("doesn't support tags yet"),
        "{}",
        stderr(&out)
    );
    // An explicit --project is a request for a tag, so it fails the same
    // way; PARCEL_PROJECT is ambient and is dropped silently (below).
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .arg("publish")
        .arg(&page)
        .args(["--project", "roost"]));
    assert_code(&out, 1);
    assert!(
        stderr(&out).contains("doesn't support tags yet"),
        "{}",
        stderr(&out)
    );
    // Nothing reached the old server but `GET /info`.
    for request in server.received_requests().await.unwrap() {
        assert_eq!(request.url.path(), "/api/v1/info", "{}", request.url);
    }

    // A plain publish from a repository works, without the new fields, and
    // `created` (absent from an old server) is not trusted.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .env("PARCEL_PROJECT", "roost")
        .current_dir(&repo)
        .arg("publish")
        .arg(&page));
    assert_code(&out, 0);
    assert!(stdout(&out).starts_with("Published "), "{}", stdout(&out));
    let sent = sent_metadata(&server, "/api/v1/parcels").await;
    assert_eq!(sent, vec![json!({"entry": "plan.html"})]);
}

#[tokio::test]
async fn list_filters_reach_the_server_and_snippets_stay_printable() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let mut item = slugged_summary(&server.uri(), 2);
    item["match"] = json!({"segments": [
        {"text": "rotate the\nrefresh ", "hit": false},
        {"text": "token", "hit": true},
        {"text": " \u{1b}]52;c;aGk=\u{7} daily", "hit": false},
    ]});
    let mut shared = summary(&server.uri(), 1);
    shared["id"] = json!("bcdefghijkmn");
    shared["title"] = json!("Theirs \u{1b}[31m");
    shared["shared"] = json!({"owner_name": "Bob", "role": "commenter",
                              "shared_at": "2026-10-14T12:00:00Z"});
    shared["archived_at"] = json!("2026-10-16T12:00:00Z");
    let raw = format!(
        r#"{{"total":2,"parcels":[{},{}]}}"#,
        serde_json::to_string(&item).unwrap(),
        serde_json::to_string(&shared).unwrap()
    );
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .and(query_param("scope", "all"))
        .and(query_param("archived", "include"))
        .and(query_param("q", "refresh token"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw.clone(), "application/json"))
        .mount(&server)
        .await;

    let args = [
        "list",
        "--all",
        "--tag",
        "Auth",
        "--project",
        "roost",
        "--kind",
        "plan",
        "-q",
        "refresh token",
        "--archived",
    ];
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(args));
    assert_code(&out, 0);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    for column in ["ID", "TITLE", "OWNER", "ROLE", "TAGS", "SLUG"] {
        assert!(lines[0].contains(column), "{column}: {text}");
    }
    assert!(
        lines[1].contains("you") && lines[1].contains("roost/plan-14"),
        "{text}"
    );
    assert!(lines[1].contains("kind:plan,project:roost"), "{text}");
    assert_eq!(
        lines[2],
        r"    … rotate the refresh **token** \u{1b}]52;c;aGk=\u{7} daily …"
    );
    assert!(
        lines[3].contains("Bob") && lines[3].contains("(archived)"),
        "{text}"
    );
    assert!(lines[3].contains(r"Theirs \u{1b}[31m"), "{text}");
    assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'));
    // A body without `relaxed` (every exact list, and any older server) is exact.
    assert!(
        !stderr(&out).contains("No exact matches"),
        "{}",
        stderr(&out)
    );

    let gets = requests_to(&server, "GET", "/api/v1/parcels").await;
    let tags: Vec<String> = gets[0]
        .url
        .query_pairs()
        .filter(|(k, _)| k == "tag")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(tags, ["auth", "kind:plan", "project:roost"]);

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(args)
        .arg("--json"));
    assert_code(&out, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        serde_json::from_str::<Value>(&raw).unwrap()
    );
}

#[tokio::test]
async fn list_shared_with_filters_uses_the_unified_list() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .and(query_param("scope", "shared"))
        .and(query_param("archived", "only"))
        .and(query_param("q", "plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"parcels": [], "total": 0})))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["list", "--shared", "-q", "plan", "--archived-only"]));
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "No parcels match \"plan\".\n");
    assert!(requests_to(&server, "GET", "/api/v1/shared")
        .await
        .is_empty());
}

/// A relaxed list says so on stderr, above the table; `--json` prints the
/// body alone; a relaxed list with nothing in it is the plain "no match".
#[tokio::test]
async fn a_relaxed_list_says_so_on_stderr_only() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let phrase = "how do we handle failed payments";
    let raw = format!(
        r#"{{"parcels":[{}],"total":1,"relaxed":true}}"#,
        serde_json::to_string(&summary(&server.uri(), 1)).unwrap()
    );
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .and(query_param("q", phrase))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw.clone(), "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels"))
        .and(query_param("q", "nothing at all"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"parcels": [], "total": 0, "relaxed": true})),
        )
        .mount(&server)
        .await;
    let list = |args: &[&str]| {
        run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .arg("list")
            .args(args))
    };

    let out = list(&["-q", phrase]);
    assert_code(&out, 0);
    assert_eq!(
        stderr(&out),
        "No exact matches; showing parcels that match any of the words.\n"
    );
    assert!(stdout(&out).starts_with("ID "), "{}", stdout(&out));
    assert!(
        !stdout(&out).contains("No exact matches"),
        "{}",
        stdout(&out)
    );

    let out = list(&["-q", phrase, "--json"]);
    assert_code(&out, 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        serde_json::from_str::<Value>(&raw).unwrap()
    );

    let out = list(&["-q", "nothing at all"]);
    assert_code(&out, 0);
    assert_eq!(stdout(&out), "No parcels match \"nothing at all\".\n");
    assert_eq!(stderr(&out), "");
}

#[tokio::test]
async fn update_edits_tags_slug_and_archive_state() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let mut archived = slugged_summary(&server.uri(), 2);
    archived["archived_at"] = json!("2026-10-20T12:00:00Z");
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(archived.clone()))
        .mount(&server)
        .await;

    let runs: [&[&str]; 5] = [
        &["--add-tag", "Auth", "--remove-tag", "draft", "--archive"],
        &["--set-tags", ""],
        &["--set-tags", "b, A,,a"],
        &["--slug", "", "--unarchive"],
        &["--slug", " Roost/Plan-14 ", "--json"],
    ];
    for extra in runs {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(["update", ID])
            .args(extra));
        assert_code(&out, 0);
        if extra.contains(&"--json") {
            assert_eq!(
                serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
                archived
            );
        } else {
            let text = stdout(&out);
            assert!(
                text.contains(&format!("url:  {}/@ada/roost/plan-14", server.uri())),
                "{text}"
            );
            assert!(text.contains("tags: kind:plan, project:roost"), "{text}");
            assert!(text.contains("archived"), "{text}");
        }
    }
    let bodies: Vec<Value> = requests_to(&server, "PATCH", &format!("/api/v1/parcels/{ID}"))
        .await
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(
        bodies,
        vec![
            json!({"add_tags": ["auth"], "remove_tags": ["draft"], "archived": true}),
            json!({"tags": []}),
            json!({"tags": ["a", "b"]}),
            json!({"archived": false, "slug": ""}),
            json!({"slug": "roost/plan-14"}),
        ]
    );

    for bad in [
        &["--set-tags", "a", "--add-tag", "b"][..],
        &["--archive", "--unarchive"],
        &["--add-tag", "x", "--remove-tag", "X"],
        &["--slug", "p/abcdefghijkm"],
        &["--add-tag", "no way!"],
        &[],
    ] {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(["update", ID])
            .args(bad));
        assert_code(&out, 2);
    }
}

#[tokio::test]
async fn tag_archive_and_unarchive_print_the_summary() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let body = slugged_summary(&server.uri(), 2);
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
        .mount(&server)
        .await;

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args([
            "tag",
            ID,
            "--add",
            "kind:report",
            "Topic",
            "--remove",
            "kind:plan",
        ]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!("Tags on {ID}: kind:plan, project:roost\n")
    );
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["archive", ID]));
    assert_code(&out, 0);
    assert!(
        stdout(&out).contains(&format!("parcel unarchive {ID}")),
        "{}",
        stdout(&out)
    );
    for command in ["tag", "archive", "unarchive"] {
        let mut cmd = sandbox.parcel(&server.uri());
        cmd.env("PARCEL_TOKEN", TOKEN).args([command, ID, "--json"]);
        if command == "tag" {
            cmd.args(["--add", "x"]);
        }
        let out = run(&mut cmd);
        assert_code(&out, 0);
        assert_eq!(serde_json::from_str::<Value>(&stdout(&out)).unwrap(), body);
    }
    let bodies: Vec<Value> = requests_to(&server, "PATCH", &format!("/api/v1/parcels/{ID}"))
        .await
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(
        bodies,
        vec![
            json!({"add_tags": ["kind:report", "topic"], "remove_tags": ["kind:plan"]}),
            json!({"archived": true}),
            json!({"add_tags": ["x"]}),
            json!({"archived": true}),
            json!({"archived": false}),
        ]
    );
    // `tag` needs something to do.
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["tag", ID]));
    assert_code(&out, 2);
}

#[tokio::test]
async fn tags_lists_every_page_then_renames_and_deletes() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let first: Vec<Value> = (0..1000)
        .map(|i| json!({"tag": format!("t{i:04}"), "count": 2}))
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/v1/tags"))
        .and(query_param("limit", "1000"))
        .and(query_param("prefix", "project:"))
        .and(query_param("scope", "all"))
        .and(query_param_is_missing("offset"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"tags": first, "total": 1001})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tags"))
        .and(query_param("offset", "1000"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"tags": [{"tag": "zz", "count": 1}], "total": 1001})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tags/rename"))
        .and(body_json(
            json!({"from": "project:roost-app", "to": "project:roost"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"updated": 3})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tags/delete"))
        .and(body_json(json!({"tag": "draft"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"updated": 1})))
        .mount(&server)
        .await;

    let list = ["tags", "--prefix", "project:", "--all"];
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(list)
        .arg("--json"));
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed["tags"].as_array().unwrap().len(), 1001);
    assert_eq!(printed["tags"][1000], json!({"tag": "zz", "count": 1}));
    assert_eq!(printed["total"], 1001);
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(list));
    assert_code(&out, 0);
    let text = stdout(&out);
    assert!(text.starts_with("TAG"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("zz") && l.ends_with('1')),
        "{text}"
    );

    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["tags", "rename", "Project:Roost_App", "project:roost"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        "Renamed project:roost-app to project:roost on 3 parcels\n"
    );
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["tags", "delete", "draft", "--json"]));
    assert_code(&out, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        json!({"updated": 1})
    );
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["tags", "delete", "draft"]));
    assert_eq!(stdout(&out), "Removed draft from 1 parcel\n");
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["tags", "rename", "ok", "bad tag!"]));
    assert_code(&out, 2);
}

/// Every REF command takes a slug: one `by-slug` lookup, then by id.
#[tokio::test]
async fn slug_refs_resolve_once_then_work_by_id() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    sandbox.write("scratch/x.html", b"<title>x</title>");
    mount_new_info(&server).await;
    let detail = detail_of(slugged_summary(&server.uri(), 2), two_pages());
    Mock::given(method("GET"))
        .and(path("/api/v1/parcels/by-slug"))
        .and(query_param("slug", "roost/plan-14"))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail.clone()))
        .mount(&server)
        .await;
    // Whatever by-id request follows gets a body its command accepts.
    let id_path = format!("/api/v1/parcels/{ID}");
    Mock::given(method("GET"))
        .and(path(format!("{id_path}/versions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versions": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{id_path}/shares")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"shares": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{id_path}/comments")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"comments": [], "open": 0})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{id_path}/data")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"revision": 0, "usage_bytes": 0, "limit_bytes": 1024, "writable": true,
                   "collections": []}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{id_path}/versions/2/files/notes/b.html")))
        .respond_with(ResponseTemplate::new(200).set_body_string("<p>b</p>"))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(&id_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(slugged_summary(&server.uri(), 2)))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(&id_path))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{id_path}/versions/1/restore")))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 3)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{id_path}/versions")))
        .respond_with(ResponseTemplate::new(201).set_body_json(publish_result(&server.uri(), 3)))
        .mount(&server)
        .await;

    let commands: [&[&str]; 14] = [
        &["info", "@ada/roost/plan-14"],
        &["versions", "roost/plan-14"],
        &["shares", "roost/plan-14"],
        &["comments", "list", "roost/plan-14"],
        &["data", "list", "roost/plan-14"],
        &["cat", "@ada/roost/plan-14?v=2&page=notes/b.html"],
        &["update", "roost/plan-14", "--title", "T"],
        &["tag", "roost/plan-14", "--add", "x"],
        &["archive", "roost/plan-14"],
        &["restore", "roost/plan-14", "--version", "1"],
        &["link", "roost/plan-14"],
        &["publish", "scratch/x.html", "--parcel", "roost/plan-14"],
        &["delete", "roost/plan-14", "--yes"],
        &["info", &format!("{}/@ada/roost/plan-14#top", server.uri())],
    ];
    for (n, args) in commands.iter().enumerate() {
        let out = run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .args(*args));
        assert_code(&out, 0);
        let lookups = requests_to(&server, "GET", "/api/v1/parcels/by-slug").await;
        assert_eq!(lookups.len(), n + 1, "{args:?}: one lookup each");
        if args[0] == "cat" {
            assert_eq!(stdout(&out), "<p>b</p>");
        }
    }
    let lookups = requests_to(&server, "GET", "/api/v1/parcels/by-slug").await;
    let handles: Vec<Option<String>> = lookups
        .iter()
        .map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == "handle")
                .map(|(_, v)| v.into_owned())
        })
        .collect();
    assert_eq!(handles[0].as_deref(), Some("ada"));
    assert_eq!(handles[1], None, "a bare slug is the caller's own");
    // info and link use the detail the lookup returned.
    assert!(requests_to(&server, "GET", &id_path).await.is_empty());
}

#[tokio::test]
async fn info_shows_tags_slug_archive_source_and_pages() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    let mut parcel = slugged_summary(&server.uri(), 2);
    parcel["archived_at"] = json!("2026-10-20T12:00:00Z");
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail_of(parcel, two_pages())))
        .mount(&server)
        .await;
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["info", ID]));
    assert_code(&out, 0);
    let text = stdout(&out);
    for needle in [
        &format!("url:         {}/@ada/roost/plan-14", server.uri()),
        "slug:        roost/plan-14",
        "tags:        kind:plan, project:roost",
        &format!("(left out of lists and search; `parcel unarchive {ID}`)"),
        "source:      github.com/acme/roost, branch main, commit 0123456789ab, on laptop, \
         via parcel-cli/0.2.0",
        "Pages in v2",
        "notes/b.html",
        r"Build \u{1b}[2Jsettings",
    ] {
        assert!(text.contains(needle), "missing {needle:?}\n{text}");
    }
    assert!(!text.contains('\u{1b}'));

    // One page is shown compactly, so its title isn't lost.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail_of(
            slugged_summary(&server.uri(), 2),
            json!([{"path": "index.html", "title": "Implementation details"}]),
        )))
        .mount(&server)
        .await;
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .args(["info", ID]));
    assert_code(&out, 0);
    let text = stdout(&out);
    assert!(
        text.contains("Pages: index.html — Implementation details\n"),
        "{text}"
    );
    assert!(!text.contains("Pages in v"), "{text}");
}

#[tokio::test]
async fn link_prints_canonical_markdown_and_json() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let mut parcel = slugged_summary(&server.uri(), 2);
    parcel["title"] = json!("Evil [x](javascript:alert(1)) \\ \u{1b}[2J");
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail_of(parcel, two_pages())))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}/versions/1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(version_detail(1, json!([]))))
        .mount(&server)
        .await;

    let link = |args: &[&str]| {
        run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .arg("link")
            .args(args))
    };
    let base = server.uri();
    let out = link(&[ID]);
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("{base}/@ada/roost/plan-14\n"));

    let out = link(&[ID, "--page", "notes/b.html", "--markdown"]);
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!(
            "[Evil \\[x\\]\\(javascript:alert\\(1\\)\\) \\\\ \\[2J]\
             ({base}/@ada/roost/plan-14?page=notes/b.html)\n"
        )
    );

    let out = link(&[&format!("{base}/p/{ID}/v/2"), "--json"]);
    assert_code(&out, 0);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed["url"], format!("{base}/p/{ID}/v/2"));
    assert_eq!(
        printed["canonical_url"],
        format!("{base}/@ada/roost/plan-14?v=2")
    );
    assert!(printed["markdown"]
        .as_str()
        .unwrap()
        .ends_with(&format!("({base}/@ada/roost/plan-14?v=2)")));
    let keys: Vec<&str> = printed
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 3);

    // A page the version doesn't have is not found (v1 has no pages).
    let out = link(&[ID, "--page", "missing.html"]);
    assert_code(&out, 4);
    assert!(
        stderr(&out).contains("index.html, notes/b.html"),
        "{}",
        stderr(&out)
    );
    let out = link(&[&format!("{base}/p/{ID}/v/1"), "--page", "notes/b.html"]);
    assert_code(&out, 4);
    // `?v=` on an id URL pins nothing (the web app ignores it too): the link is
    // the current version's, never `?v=1`.
    let out = link(&[&format!("{base}/p/{ID}?v=1")]);
    assert_code(&out, 0);
    assert_eq!(stdout(&out), format!("{base}/@ada/roost/plan-14\n"));
    let out = link(&[ID, "--page", "../x"]);
    assert_code(&out, 2);
}

/// Stand-in browser openers, so `open` launches nothing.
fn no_browser(sandbox: &Sandbox) -> std::ffi::OsString {
    let dir = sandbox.root.join("no-browser");
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    for opener in [
        "xdg-open",
        "gio",
        "gnome-open",
        "kde-open",
        "wslview",
        "open",
    ] {
        use std::os::unix::fs::PermissionsExt;
        let stub = dir.join(opener);
        fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut path = dir.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    path
}

#[tokio::test]
async fn open_prints_the_canonical_page_link() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/parcels/{ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(detail_of(slugged_summary(&server.uri(), 2), two_pages())),
        )
        .mount(&server)
        .await;
    let path_var = no_browser(&sandbox);
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .env("PATH", &path_var)
        .args(["open", &format!("{ID}#intro"), "--page", "notes/b.html"]));
    assert_code(&out, 0);
    assert_eq!(
        stdout(&out),
        format!("{}/@ada/roost/plan-14?page=notes/b.html\n", server.uri())
    );
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .env("PATH", &path_var)
        .args(["open", &format!("{ID}?page=notes/b.html#intro"), "--json"]));
    assert_code(&out, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        json!({"url": format!("{}/@ada/roost/plan-14?page=notes/b.html#intro", server.uri())})
    );
    // A raw fragment is user-supplied: human output escapes it, as `link`
    // does (--json carries it as an escaped JSON string).
    let out = run(sandbox
        .parcel(&server.uri())
        .env("PARCEL_TOKEN", TOKEN)
        .env("PATH", &path_var)
        .args(["open", &format!("{ID}#a\u{1b}[2Jb")]));
    assert_code(&out, 0);
    assert!(!stdout(&out).contains('\u{1b}'), "{:?}", stdout(&out));
    assert!(stdout(&out).contains(r"\u{1b}[2J"), "{:?}", stdout(&out));
}

#[tokio::test]
async fn handle_shows_and_changes_the_handle() {
    let server = MockServer::start().await;
    let sandbox = Sandbox::new();
    mount_new_info(&server).await;
    let me = |handle: &str| {
        json!({
            "id": "u1", "email": "ada@example.com", "name": "Ada",
            "usage": {"parcel_count": 0, "version_count": 0, "stored_bytes": 0},
            "limits": limits(), "handle": handle,
        })
    };
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me("ada")))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/me"))
        .and(body_json(json!({"handle": "lovelace"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(me("lovelace")))
        .expect(2)
        .mount(&server)
        .await;

    let handle = |args: &[&str]| {
        run(sandbox
            .parcel(&server.uri())
            .env("PARCEL_TOKEN", TOKEN)
            .arg("handle")
            .args(args))
    };
    let out = handle(&[]);
    assert_code(&out, 0);
    assert!(stdout(&out).starts_with("@ada\n"), "{}", stdout(&out));
    let out = handle(&["--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        json!({"handle": "ada"})
    );
    let out = handle(&["Lovelace"]);
    assert_code(&out, 0);
    assert!(
        stdout(&out).contains("now @lovelace (was @ada); links with @ada keep working"),
        "{}",
        stdout(&out)
    );
    let out = handle(&["lovelace", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        json!({"handle": "lovelace", "previous": "ada"})
    );
    // Asking for the handle you have changes nothing.
    let out = handle(&["ada", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&out)).unwrap(),
        json!({"handle": "ada"})
    );
    let out = handle(&["-bad-"]);
    assert_code(&out, 2);
}
