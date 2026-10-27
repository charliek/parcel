//! MCP content never reaches the server log, at any `RUST_LOG` (plan 003
//! §3.2): rmcp logs requests and results whole below ERROR, and the caps
//! `main.rs` installs ([`parcel_server::log_caps`]) hold it there.
//!
//! The caps live in the process-wide subscriber, which a test can install
//! only once and can't read back, so [`mcp_content_stays_out_of_the_log`]
//! re-runs this test binary as a child that runs only [`log_child`], with
//! `RUST_LOG=trace` and [`CHILD`] set on the child alone (this process's
//! environment is never touched). The child installs the production stack
//! (`try_init_logging_with`, JSON to stdout) with the caps, or with none
//! as the control, then drives `/mcp` through the request, result and
//! malformed-argument paths with per-run canaries. The parent reads what
//! the child printed.

mod common;

use std::process::Command;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use common::*;
use parcel_server::config::McpConfig;
use parcel_types::{CreatedToken, PublishResult};
use serde_json::{json, Value};
use stridelabs_observability::{try_init_logging_with, LogFormat, TargetCaps};

/// Set on the child only: `capped` or `uncapped`. Unset, [`log_child`] does
/// nothing (as it does in every ordinary test run).
const CHILD: &str = "PARCEL_LOG_CHILD";
/// Set on the child only: this run's id, which every canary carries.
const RUN: &str = "PARCEL_LOG_RUN";
/// The child's name as libtest matches it with `--exact`.
const CHILD_TEST: &str = "log_child";
/// What the child prints once logging is installed and again once every
/// request got its expected response, so a child that never ran its body
/// can't pass for a clean one.
const SENTINEL: &str = "parcel-log-child:";

/// One child's output, split into its JSON log lines and everything else.
struct ChildRun {
    mode: &'static str,
    run: String,
    output: String,
    logs: Vec<Value>,
}

impl ChildRun {
    fn canary(&self, kind: &str) -> String {
        format!("CANARY_{kind}_{}", self.run)
    }

    /// The log lines whose target is `rmcp` or beneath it.
    fn rmcp_lines(&self) -> impl Iterator<Item = &Value> {
        self.logs.iter().filter(|l| {
            let target = l["target"].as_str().unwrap_or_default();
            target == "rmcp" || target.starts_with("rmcp::")
        })
    }

    /// The output for a failure message, every line cut short.
    fn excerpt(&self) -> String {
        let mut out = format!(
            "--- {} child output (lines cut at 400 chars) ---\n",
            self.mode
        );
        for line in self.output.lines() {
            let cut: String = line.chars().take(400).collect();
            out.push_str(&cut);
            out.push('\n');
        }
        out
    }
}

/// Run [`log_child`] in a fresh copy of this binary, with `mode` and
/// `RUST_LOG=trace` set on it alone, and check it ran its body to the end.
fn run_child(mode: &'static str) -> ChildRun {
    let run = uuid::Uuid::new_v4().simple().to_string();
    let exe = std::env::current_exe().expect("this test binary's path");
    let child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env("RUST_LOG", "trace")
        .env(CHILD, mode)
        .env(RUN, &run)
        .output()
        .expect("running the child test");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    let logs: Vec<Value> = output
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    let child_run = ChildRun {
        mode,
        run,
        output,
        logs,
    };
    let excerpt = || child_run.excerpt();
    assert!(child.status.success(), "{mode} child failed\n{}", excerpt());
    // `--exact` matched exactly the child, and it ran its body to the end.
    assert!(child_run.output.contains("running 1 test"), "{}", excerpt());
    for stage in ["logging installed", "every response as expected"] {
        let line = format!("{SENTINEL} {mode} {stage}");
        assert!(
            child_run.output.contains(&line),
            "no {line:?}\n{}",
            excerpt()
        );
    }
    // `RUST_LOG` reached the child: the default filter is `info`. (Only rmcp
    // logs at TRACE here, so the control checks for that level.)
    assert!(
        child_run
            .logs
            .iter()
            .any(|l| l["level"] == "DEBUG" || l["level"] == "TRACE"),
        "nothing below INFO\n{}",
        excerpt()
    );
    child_run
}

#[test]
fn mcp_content_stays_out_of_the_log() {
    let capped = run_child("capped");
    let uncapped = run_child("uncapped");

    // The control: without the caps the content does reach the log, through
    // rmcp's DEBUG lines: the request with its arguments, the result with
    // the page, and the malformed-argument result quoting serde's error
    // over the caller's value.
    for (kind, message) in [
        ("ARG", "received request"),
        ("RESULT", "response message"),
        ("BAD", "response message"),
    ] {
        let canary = uncapped.canary(kind);
        assert!(
            uncapped.rmcp_lines().any(|l| l["level"] == "DEBUG"
                && l["fields"]["message"] == message
                && l.to_string().contains(&canary)),
            "no rmcp {message:?} line with {canary}\n{}",
            uncapped.excerpt()
        );
    }
    // And `RUST_LOG=trace` took: rmcp's TRACE transport lines are there.
    assert!(
        uncapped.rmcp_lines().any(|l| l["level"] == "TRACE"),
        "{}",
        uncapped.excerpt()
    );

    // Capped: no canary anywhere, no rmcp line below ERROR.
    assert!(
        !capped.output.contains("CANARY_"),
        "a canary leaked\n{}",
        capped.excerpt()
    );
    if let Some(line) = capped.rmcp_lines().find(|l| l["level"] != "ERROR") {
        panic!("rmcp logged below ERROR: {line}");
    }
    // And logging was on: parcel's MCP audit line for each tool call that
    // reached a tool, still inside the request's span with its `requestId`.
    for tool in ["list_parcels", "read_file", "get_parcel"] {
        assert!(
            capped.logs.iter().any(|l| {
                l["target"] == stridelabs_mcp::AUDIT_TARGET
                    && l["fields"]["tool"] == tool
                    && l["span"]["requestId"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty() && id != "-")
            }),
            "no {tool} audit line with a requestId\n{}",
            capped.excerpt()
        );
    }
}

/// The child: a no-op unless [`CHILD`] is set (see the module docs).
#[tokio::test]
async fn log_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let run = std::env::var(RUN).expect("the parent sets the run id");
    let caps = match mode.as_str() {
        "capped" => parcel_server::log_caps(),
        "uncapped" => TargetCaps::new(),
        other => panic!("unknown {CHILD} {other:?}"),
    };
    try_init_logging_with(LogFormat::Json, "info", &caps).expect("logging installs");
    println!("{SENTINEL} {mode} logging installed");
    let canary = |kind: &str| format!("CANARY_{kind}_{run}");

    let app = TestApp::with_config(|c| {
        let issuer = c.providers[0].issuer.clone();
        c.mcp = Some(McpConfig {
            provider: PROVIDER.into(),
            jwks_url: format!("{issuer}/jwks.json"),
            issuer,
        });
    })
    .await;
    let ada = person("ada");
    // In a script, which search doesn't index, so the query below can't
    // find the page on the canary's shared words.
    let page = format!(
        "<!doctype html><html><head><title>Hello Parcel</title></head>\
         <body><h1>Hi</h1><script>var c = \"{}\";</script></body></html>",
        canary("RESULT")
    );
    let published: PublishResult = expect_json(
        app.post_multipart(
            "/api/v1/parcels",
            Some(&ada.token),
            &Form::new().file("index.html", page),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    let id = published.parcel.id;
    let token: CreatedToken = expect_json(
        app.post_json("/api/v1/tokens", Some(&ada.token), &json!({"name": "mcp"}))
            .await,
        StatusCode::CREATED,
    )
    .await;
    let post = |body: Value| post_mcp(&app, &token.secret, body);

    let initialized = post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "mcp-logs", "version": "1"}}}))
    .await;
    assert!(
        initialized["result"]["serverInfo"]["name"].is_string(),
        "{initialized}"
    );

    let listed = post(tool_call(
        2,
        "list_parcels",
        json!({"query": canary("ARG")}),
    ))
    .await;
    assert_ne!(listed["result"]["isError"], true, "{listed}");
    assert_eq!(
        listed["result"]["structuredContent"]["items"],
        json!([]),
        "{listed}"
    );

    let read = post(tool_call(3, "read_file", json!({"parcel": id}))).await;
    assert_ne!(read["result"]["isError"], true, "{read}");
    let text = read["result"]["content"][1]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains(&canary("RESULT")), "{read}");

    // rmcp refuses arguments that don't deserialize before any tool runs,
    // as an `isError` result quoting serde's error (the value included).
    let bad = post(tool_call(
        canary("ID"),
        "list_parcels",
        json!({"limit": canary("BAD")}),
    ))
    .await;
    assert_eq!(bad["id"], canary("ID"), "{bad}");
    assert_eq!(bad["result"]["isError"], true, "{bad}");
    let text = bad["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.starts_with("failed to deserialize parameters"),
        "{bad}"
    );

    // A business-level failure is a tool result, not a protocol error.
    let missing = parcel_types::ids::parcel_id_from_bytes(
        uuid::Uuid::new_v4().as_bytes()[..parcel_types::ids::ID_LEN]
            .try_into()
            .unwrap(),
    );
    let failed = post(tool_call(5, "get_parcel", json!({"parcel": missing}))).await;
    assert_eq!(failed["result"]["isError"], true, "{failed}");

    println!("{SENTINEL} {mode} every response as expected");
}

fn tool_call(id: impl Into<Value>, tool: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.into(), "method": "tools/call",
           "params": {"name": tool, "arguments": arguments}})
}

/// POST a JSON-RPC message to `/mcp` with a `pcl_` token; 200 and its body.
async fn post_mcp(app: &TestApp, token: &str, body: Value) -> Value {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "parcel.test")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    expect_json(app.send(request).await, StatusCode::OK).await
}
