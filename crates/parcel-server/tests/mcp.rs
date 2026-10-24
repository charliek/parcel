//! The MCP server at `/mcp`: its metadata and challenge, who a token acts
//! as, scopes, `pcl_` tokens, and the tools end to end.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use common::*;
use parcel_server::config::{Config, McpConfig};
use parcel_types::CreatedToken;
use serde_json::{json, Value};
use stridelabs_auth::test_support::{TestClaims, TestKey};
use stridelabs_mcp::test_support::access_token;
use stridelabs_mcp::ProtectedResource;

const RESOURCE: &str = "https://parcel.test/mcp";

async fn mcp_app(adjust: impl FnOnce(&mut Config)) -> TestApp {
    TestApp::with_config(|c| {
        // The mock provider serves its keys at /jwks.json.
        let issuer = c.providers[0].issuer.clone();
        c.mcp = Some(McpConfig {
            provider: PROVIDER.into(),
            jwks_url: format!("{issuer}/jwks.json"),
            issuer,
        });
        adjust(c);
    })
    .await
}

fn resource(app: &TestApp) -> ProtectedResource {
    ProtectedResource::new(RESOURCE, app.idp.uri(), ["parcel:read", "parcel:write"]).unwrap()
}

/// A slauth-style MCP access token for `sub`/`email` with `scopes`.
fn mcp_token(app: &TestApp, sub: &str, email: &str, scopes: &[&str]) -> String {
    access_token(&resource(app))
        .subject(sub)
        .email(email)
        .scopes(scopes.iter().copied())
        .client_id("parcel-claude")
        .mint()
}

async fn post_mcp(
    app: &TestApp,
    host: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value, Option<String>) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18");
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let response = app
        .send(req.body(Body::from(body.to_string())).unwrap())
        .await;
    let status = response.status();
    let challenge = response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = body_bytes(response).await;
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, challenge)
}

/// `tools/call` → (isError, structuredContent or the text).
async fn call(app: &TestApp, token: &str, tool: &str, args: Value) -> (bool, Value) {
    let (status, body, _) = post_mcp(
        app,
        "parcel.test",
        Some(token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": tool, "arguments": args}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let value = match result.get("structuredContent") {
        Some(v) if !v.is_null() => v.clone(),
        _ => result["content"][0]["text"].clone(),
    };
    (is_error, value)
}

/// `read_file` → (what the file is, its content).
async fn read_file(app: &TestApp, token: &str, args: Value) -> (Value, String) {
    let (status, body, _) = post_mcp(
        app,
        "parcel.test",
        Some(token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": "read_file", "arguments": args}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(body["result"]["isError"], true, "{body}");
    let content = &body["result"]["content"];
    let about = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    (about, content[1]["text"].as_str().unwrap().to_string())
}

async fn known_user(app: &TestApp, sub: &str, email: &str) {
    app.session_cookie(sub, email).await;
}

#[tokio::test]
async fn metadata_and_challenge_point_at_the_provider() {
    let app = mcp_app(|_| {}).await;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let doc: Value = json(expect_status(app.get(path, None).await, StatusCode::OK).await).await;
        assert_eq!(doc["resource"], RESOURCE);
        assert_eq!(doc["authorization_servers"], json!([app.idp.uri()]));
        assert_eq!(
            doc["scopes_supported"],
            json!(["parcel:read", "parcel:write"])
        );
    }

    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "t", "version": "1"}}});
    let (status, _, challenge) = post_mcp(&app, "parcel.test", None, init.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = challenge.expect("a challenge");
    assert!(
        challenge.contains(
            r#"resource_metadata="https://parcel.test/.well-known/oauth-protected-resource/mcp""#
        ),
        "{challenge}"
    );
    assert!(
        challenge.contains(r#"scope="parcel:read parcel:write""#),
        "{challenge}"
    );

    // A token for the web client (aud = its client id) is no MCP token.
    let web = TestKey::primary().mint(&TestClaims::new(app.idp.uri(), CLIENT_ID).build());
    let (status, _, challenge) = post_mcp(&app, "parcel.test", Some(&web), init.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(challenge.unwrap().contains(r#"error="invalid_token""#));

    // Content host: no MCP there (host dispatch precedes routing).
    let (status, _, _) = post_mcp(&app, CONTENT_HOST, None, init.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let token = mcp_token(&app, "sub-1", "ada@example.com", &["parcel:read"]);
    let (status, body, _) = post_mcp(&app, "parcel.test", Some(&token), init).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["serverInfo"]["name"], "parcel");
}

/// A token whose `kid` the key cache has never seen, inside its refetch
/// floor, may be a rotation not fetched yet: `503` (retry with the same
/// token), not `401` (which would send the client back through sign-in).
#[tokio::test]
async fn an_unseen_kid_is_unavailable_not_invalid() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-k", "k@example.com").await;
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});

    // A good token first, so the cache has fetched (and timed) the set.
    let token = mcp_token(&app, "sub-k", "k@example.com", &["parcel:read"]);
    let (status, body, _) = post_mcp(&app, "parcel.test", Some(&token), list.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let claims = TestClaims::new(app.idp.uri(), RESOURCE)
        .subject("sub-k")
        .email("k@example.com")
        .claim("scope", json!("parcel:read"))
        .claim("client_id", json!("parcel-claude"))
        .build();
    let unseen = TestKey::secondary().mint_with_kid("never-published", &claims);
    let (status, body, challenge) = post_mcp(&app, "parcel.test", Some(&unseen), list).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        !challenge.is_some_and(|c| c.contains("invalid_token")),
        "a 503 asks for no new token"
    );
}

#[tokio::test]
async fn off_unless_configured() {
    let app = TestApp::new().await;
    let r = app
        .get("/.well-known/oauth-protected-resource/mcp", None)
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let (status, _, _) = post_mcp(&app, "parcel.test", None, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publish_read_update_share_delete() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-ada", "ada@example.com").await;
    let token = mcp_token(
        &app,
        "sub-ada",
        "ada@example.com",
        &["parcel:read", "parcel:write"],
    );

    let (err, me) = call(&app, &token, "whoami", json!({})).await;
    assert!(!err, "{me}");
    assert_eq!(me["email"], "ada@example.com");

    let page = "<!doctype html><title>Hello MCP</title><h1>hi</h1>";
    let (err, published) = call(
        &app,
        &token,
        "publish_parcel",
        json!({"files": [
            {"path": "index.html", "content": page},
            {"path": "dot.png", "content": "iVBORw0KGgo=", "encoding": "base64"}
        ]}),
    )
    .await;
    assert!(!err, "{published}");
    let id = published["parcel"]["id"].as_str().unwrap().to_string();
    assert_eq!(published["parcel"]["title"], "Hello MCP");
    assert_eq!(published["parcel"]["visibility"], "private");
    assert_eq!(published["parcel"]["url"], format!("{APP_ORIGIN}/p/{id}"));
    assert_eq!(
        published["parcel"]["canonical_url"],
        format!("{APP_ORIGIN}/p/{id}")
    );
    assert_eq!(published["created"], true);
    // Versions published through MCP say so (shown to the owner only).
    assert_eq!(published["version"]["source"], json!({"client": "mcp"}));

    let (_, list) = call(&app, &token, "list_parcels", json!({})).await;
    assert_eq!(list["total"], 1);
    assert_eq!(list["has_more"], false);
    assert_eq!(list["items"][0]["id"], id.as_str());

    let (about, content) = read_file(
        &app,
        &token,
        json!({"parcel": format!("{APP_ORIGIN}/p/{id}")}),
    )
    .await;
    assert_eq!(about["path"], "index.html");
    assert_eq!(about["encoding"], "utf8");
    assert_eq!(about["version"], 1);
    assert_eq!(content, page);
    let (about, content) = read_file(&app, &token, json!({"parcel": id, "path": "dot.png"})).await;
    assert_eq!(about["encoding"], "base64");
    assert_eq!(about["content_type"], "image/png");
    assert_eq!(content, "iVBORw0KGgo=");

    let (err, updated) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "files": [{"path": "about.html", "content": "<p>about</p>"}],
               "base_version": 1, "label": "add about"}),
    )
    .await;
    assert!(!err, "{updated}");
    assert_eq!(updated["version"]["version"], 2);
    assert_eq!(updated["version"]["source"], json!({"client": "mcp"}));
    let (_, detail) = call(&app, &token, "get_parcel", json!({"parcel": id})).await;
    let paths: Vec<&str> = detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["about.html", "dot.png", "index.html"]);
    // An older version by URL.
    let (_, old) = call(
        &app,
        &token,
        "read_file",
        json!({"parcel": format!("{APP_ORIGIN}/p/{id}/v/1"), "path": "about.html"}),
    )
    .await;
    assert!(
        old.as_str().unwrap().contains("not found in version 1"),
        "{old}"
    );

    let (err, stale) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "files": [{"path": "x.html", "content": "x"}], "base_version": 1}),
    )
    .await;
    assert!(err);
    assert!(stale.as_str().unwrap().contains("version 2"), "{stale}");

    let (_, renamed) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "title": "Renamed", "visibility": "internal"}),
    )
    .await;
    assert_eq!(renamed["parcel"]["title"], "Renamed");
    assert_eq!(renamed["parcel"]["visibility"], "internal");
    // Back to private, so access below comes from the share alone.
    let (_, private) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "visibility": "private"}),
    )
    .await;
    assert_eq!(private["parcel"]["visibility"], "private");

    let (err, share) = call(
        &app,
        &token,
        "share_parcel",
        json!({"parcel": id, "email": "Bob@Example.com", "role": "commenter"}),
    )
    .await;
    assert!(!err, "{share}");
    assert_eq!(share["email"], "bob@example.com");
    // Bob, connecting for the first time, sees it as shared with him.
    let bob = mcp_token(&app, "sub-bob", "bob@example.com", &["parcel:read"]);
    known_user(&app, "sub-bob", "bob@example.com").await;
    let (_, shared) = call(&app, &bob, "list_parcels", json!({"which": "shared"})).await;
    assert_eq!(shared["items"][0]["parcel"]["id"], id.as_str());
    let (_, bob_view) = call(&app, &bob, "get_parcel", json!({"parcel": id})).await;
    assert_eq!(bob_view["access"], "commenter");
    // …but can't change it.
    let bob_rw = mcp_token(
        &app,
        "sub-bob",
        "bob@example.com",
        &["parcel:read", "parcel:write"],
    );
    let (err, refused) = call(&app, &bob_rw, "delete_parcel", json!({"parcel": id})).await;
    assert!(err, "{refused}");

    let (_, _) = call(
        &app,
        &token,
        "unshare_parcel",
        json!({"parcel": id, "email": "bob@example.com"}),
    )
    .await;
    let (err, gone) = call(&app, &bob, "get_parcel", json!({"parcel": id})).await;
    assert!(
        err && gone.as_str().unwrap().contains("not found"),
        "{gone}"
    );

    let (err, deleted) = call(&app, &token, "delete_parcel", json!({"parcel": id})).await;
    assert!(!err, "{deleted}");
    let (err, _) = call(&app, &token, "get_parcel", json!({"parcel": id})).await;
    assert!(err);
}

#[tokio::test]
async fn tools_enforce_their_scopes() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-r", "reader@example.com").await;
    let read_only = mcp_token(&app, "sub-r", "reader@example.com", &["parcel:read"]);

    let (err, _) = call(&app, &read_only, "list_parcels", json!({})).await;
    assert!(!err);
    let (err, refused) = call(
        &app,
        &read_only,
        "publish_parcel",
        json!({"files": [{"path": "index.html", "content": "x"}]}),
    )
    .await;
    assert!(err);
    assert!(
        refused
            .as_str()
            .unwrap()
            .contains("Missing required scope(s): parcel:write"),
        "{refused}"
    );
    // Nothing was published.
    let (_, list) = call(&app, &read_only, "list_parcels", json!({})).await;
    assert_eq!(list["total"], 0);
}

#[tokio::test]
async fn a_first_connection_signs_in_only_when_the_provider_is_trusted() {
    // trust_email = false (the test provider's default): access tokens carry
    // no email_verified, so a stranger must sign in on the web first.
    let app = mcp_app(|_| {}).await;
    let token = mcp_token(&app, "sub-new", "new@example.com", &["parcel:read"]);
    let (err, refused) = call(&app, &token, "whoami", json!({})).await;
    assert!(err);
    assert!(
        refused
            .as_str()
            .unwrap()
            .contains("sign in to Parcel on the web"),
        "{refused}"
    );

    // A trusted provider (slauth's setting): the first connection signs in,
    // linking by email like the web callback.
    let app = mcp_app(|c| c.providers[0].trust_email = true).await;
    let token = mcp_token(&app, "sub-new", "New@Example.com", &["parcel:read"]);
    let (err, me) = call(&app, &token, "whoami", json!({})).await;
    assert!(!err, "{me}");
    assert_eq!(me["email"], "new@example.com");
}

#[tokio::test]
async fn disabled_people_and_disallowed_domains_are_refused() {
    let app = mcp_app(|c| {
        c.providers[0].trust_email = true;
        c.providers[0].allowed_domains = vec!["example.com".into()];
    })
    .await;
    known_user(&app, "sub-d", "dee@example.com").await;
    let token = mcp_token(&app, "sub-d", "dee@example.com", &["parcel:read"]);
    let (err, _) = call(&app, &token, "whoami", json!({})).await;
    assert!(!err);

    // A trusted provider's email moved to a domain it may not sign in.
    let moved = mcp_token(&app, "sub-d", "dee@elsewhere.org", &["parcel:read"]);
    let (err, refused) = call(&app, &moved, "whoami", json!({})).await;
    assert!(err);
    assert!(refused.as_str().unwrap().contains("domain"), "{refused}");

    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = $1")
        .bind("dee@example.com")
        .execute(&app.pool)
        .await
        .unwrap();
    let (err, refused) = call(&app, &token, "whoami", json!({})).await;
    assert!(err);
    assert!(refused.as_str().unwrap().contains("disabled"), "{refused}");
}

#[tokio::test]
async fn an_untrusted_providers_token_email_does_not_decide_the_domain() {
    let app = mcp_app(|c| c.providers[0].allowed_domains = vec!["example.com".into()]).await;
    known_user(&app, "sub-u", "u@example.com").await;
    // Unverified, so the email of the verified web sign-in decides.
    let elsewhere = mcp_token(&app, "sub-u", "u@elsewhere.org", &["parcel:read"]);
    let (err, me) = call(&app, &elsewhere, "whoami", json!({})).await;
    assert!(!err, "{me}");
    assert_eq!(me["email"], "u@example.com");

    // And the other way round: narrowing the domains cuts the stored one off,
    // whatever the token says.
    let app = mcp_app(|c| c.providers[0].allowed_domains = vec!["example.com".into()]).await;
    known_user(&app, "sub-v", "v@legacy.org").await;
    let claims_allowed = mcp_token(&app, "sub-v", "v@example.com", &["parcel:read"]);
    let (err, refused) = call(&app, &claims_allowed, "whoami", json!({})).await;
    assert!(err);
    assert!(refused.as_str().unwrap().contains("domain"), "{refused}");
}

#[tokio::test]
async fn tool_input_is_held_to_the_api_rules() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-i", "i@example.com").await;
    let token = mcp_token(
        &app,
        "sub-i",
        "i@example.com",
        &["parcel:read", "parcel:write"],
    );

    for (files, expected) in [
        (
            json!([{"path": "a.html", "content": "1"}, {"path": "a.html", "content": "2"}]),
            "uploaded twice",
        ),
        (
            json!([{"path": "a.png", "content": "%%%", "encoding": "base64"}]),
            "not valid base64",
        ),
        (
            json!([{"path": "a.exe", "content": "x"}]),
            "unsupported file type",
        ),
        (json!([]), "at least one file"),
    ] {
        let (err, refused) = call(&app, &token, "publish_parcel", json!({"files": files})).await;
        assert!(err, "{refused}");
        assert!(
            refused.as_str().unwrap().contains(expected),
            "{expected}: {refused}"
        );
    }

    let (_, published) = call(
        &app,
        &token,
        "publish_parcel",
        json!({"files": [{"path": "index.html", "content": "<title>t</title>"},
                         {"path": "big.txt", "content": "x".repeat(1024 * 1024 + 1)}]}),
    )
    .await;
    let id = published["parcel"]["id"].as_str().unwrap().to_string();
    let (err, refused) = call(
        &app,
        &token,
        "read_file",
        json!({"parcel": id, "path": "big.txt"}),
    )
    .await;
    assert!(err);
    assert!(
        refused.as_str().unwrap().contains("parcel CLI"),
        "{refused}"
    );

    // Version-only fields without a new version are a mistake, not a no-op.
    let (err, refused) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "title": "T", "base_version": 1}),
    )
    .await;
    assert!(err);
    assert!(
        refused.as_str().unwrap().contains("apply to a new version"),
        "{refused}"
    );

    // Someone else's private parcel doesn't exist for them.
    known_user(&app, "sub-o", "o@example.com").await;
    let other = mcp_token(
        &app,
        "sub-o",
        "o@example.com",
        &["parcel:read", "parcel:write"],
    );
    for (tool, args) in [
        (
            "update_parcel",
            json!({"parcel": id, "files": [{"path": "x.html", "content": "x"}]}),
        ),
        (
            "share_parcel",
            json!({"parcel": id, "email": "z@example.com"}),
        ),
        ("read_file", json!({"parcel": id})),
    ] {
        let (err, refused) = call(&app, &other, tool, args).await;
        assert!(err, "{tool}");
        assert!(
            refused.as_str().unwrap().contains("not found"),
            "{tool}: {refused}"
        );
    }
}

#[tokio::test]
async fn oversized_requests_are_refused() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-b", "b@example.com").await;
    let token = mcp_token(&app, "sub-b", "b@example.com", &["parcel:write"]);
    let content = "x".repeat(parcel_server::mcp::MAX_BODY_BYTES);
    let (status, _, _) = post_mcp(
        &app,
        "parcel.test",
        Some(&token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": "publish_parcel",
                          "arguments": {"files": [{"path": "a.txt", "content": content}]}}}),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn api_tokens_work_as_mcp_credentials() {
    let app = mcp_app(|_| {}).await;
    let web = user_token();
    let created: CreatedToken = json(
        expect_status(
            app.post_json("/api/v1/tokens", Some(&web), &json!({"name": "cursor"}))
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;

    let (err, published) = call(
        &app,
        &created.secret,
        "publish_parcel",
        json!({"files": [{"path": "index.html", "content": "<title>T</title>"}]}),
    )
    .await;
    assert!(!err, "{published}");
    // The same person, through the REST API.
    let list: parcel_types::ParcelList = json(app.get("/api/v1/parcels", Some(&web)).await).await;
    assert_eq!(list.total, 1);

    let r = app
        .delete(&format!("/api/v1/tokens/{}", created.token.id), Some(&web))
        .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let (status, _, challenge) = post_mcp(
        &app,
        "parcel.test",
        Some(&created.secret),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(challenge.unwrap().contains(r#"error="invalid_token""#));
}

#[tokio::test]
async fn the_rest_api_refuses_mcp_tokens() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-x", "x@example.com").await;
    let token = mcp_token(
        &app,
        "sub-x",
        "x@example.com",
        &["parcel:read", "parcel:write"],
    );
    let r = app.get("/api/v1/parcels", Some(&token)).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn other_hosts_are_refused() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-h", "h@example.com").await;
    let token = mcp_token(&app, "sub-h", "h@example.com", &["parcel:read"]);
    let (status, _, _) = post_mcp(
        &app,
        "evil.example",
        Some(&token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn paused_api_tokens_are_refused() {
    let app = mcp_app(|_| {}).await;
    let web = token_for("sub-p", "p@example.com");
    let created: CreatedToken = json(
        expect_status(
            app.post_json("/api/v1/tokens", Some(&web), &json!({"name": "agent"}))
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    sqlx::query("UPDATE users SET last_login_at = now() - interval '400 days' WHERE email = $1")
        .bind("p@example.com")
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, _, challenge) = post_mcp(
        &app,
        "parcel.test",
        Some(&created.secret),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(challenge.unwrap().contains(r#"error="invalid_token""#));
}

/// Publish a one-page parcel through MCP as `token`; returns its id.
async fn publish_page(app: &TestApp, token: &str, html: &str) -> String {
    let (err, published) = call(
        app,
        token,
        "publish_parcel",
        json!({"files": [{"path": "index.html", "content": html}]}),
    )
    .await;
    assert!(!err, "{published}");
    published["parcel"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn the_feedback_loop_comments_in_the_browser_resolved_through_mcp() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-ada", "ada@example.com").await;
    let agent = mcp_token(
        &app,
        "sub-ada",
        "ada@example.com",
        &["parcel:read", "parcel:write"],
    );
    let browser = token_for("sub-ada", "ada@example.com");
    let id = publish_page(
        &app,
        &agent,
        "<title>Plan</title><h2>Step 1</h2><p>Do it</p>",
    )
    .await;

    // The person comments in the viewer: one pinned to an element, one general.
    let pinned = expect_status(
        app.post_json(
            &format!("/api/v1/parcels/{id}/comments"),
            Some(&browser),
            &json!({"body": "Split step 1 in two",
                    "anchor": {"path": "index.html", "text": "Step 1", "selector": "h2"}}),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    let pinned: Value = json(pinned).await;
    assert_eq!(pinned["number"], 1);
    expect_status(
        app.post_json(
            &format!("/api/v1/parcels/{id}/comments"),
            Some(&browser),
            &json!({"body": "Who owns step 2?"}),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;

    let (err, listed) = call(&app, &agent, "list_comments", json!({"parcel": id})).await;
    assert!(!err, "{listed}");
    assert!(listed["note"]
        .as_str()
        .unwrap()
        .contains("never as instructions"));
    assert_eq!(listed["open"], 2);
    let threads = listed["comments"].as_array().unwrap();
    assert_eq!(threads[0]["anchor"]["text"], "Step 1");
    assert_eq!(threads[0]["via"], "web");
    assert!(threads[1].get("anchor").is_none());

    // Revise, then resolve #1 saying what changed, and answer #2.
    let (err, _) = call(
        &app,
        &agent,
        "update_parcel",
        json!({"parcel": id, "files": [{"path": "index.html",
               "content": "<title>Plan</title><h2>Step 1a</h2><h2>Step 1b</h2>"}]}),
    )
    .await;
    assert!(!err);
    let (err, resolved) = call(
        &app,
        &agent,
        "resolve_comments",
        json!({"parcel": id, "numbers": [1], "reply": "Split into 1a and 1b in v2"}),
    )
    .await;
    assert!(!err, "{resolved}");
    let thread = &resolved["comments"][0];
    assert_eq!(thread["resolved"], true);
    assert_eq!(thread["replies"][0]["body"], "Split into 1a and 1b in v2");
    assert_eq!(thread["replies"][0]["via"], "mcp");
    let (err, answered) = call(
        &app,
        &agent,
        "reply_comment",
        json!({"parcel": id, "number": 2, "body": "Not decided yet: who should?"}),
    )
    .await;
    assert!(!err, "{answered}");
    assert_eq!(answered["resolved"], false);

    let (_, open) = call(&app, &agent, "list_comments", json!({"parcel": id})).await;
    assert_eq!(open["open"], 1);
    assert_eq!(open["comments"][0]["number"], 2);
    let (_, all) = call(
        &app,
        &agent,
        "list_comments",
        json!({"parcel": id, "status": "all"}),
    )
    .await;
    assert_eq!(all["comments"].as_array().unwrap().len(), 2);

    // The viewer shows the MCP reply as the owner's, via MCP.
    let viewer: Value = json(
        app.get(
            &format!("/api/v1/parcels/{id}/comments?status=all"),
            Some(&browser),
        )
        .await,
    )
    .await;
    assert_eq!(viewer["comments"][0]["replies"][0]["via"], "mcp");
    assert_eq!(
        viewer["comments"][0]["replies"][0]["author"]["is_owner"],
        true
    );

    let (err, reopened) = call(
        &app,
        &agent,
        "reopen_comment",
        json!({"parcel": id, "number": 1}),
    )
    .await;
    assert!(!err);
    assert_eq!(reopened["resolved"], false);
    let (err, added) = call(
        &app,
        &agent,
        "add_comment",
        json!({"parcel": id, "body": "Is 1b right?", "path": "index.html", "element_text": "Step 1b"}),
    )
    .await;
    assert!(!err, "{added}");
    assert_eq!(added["number"], 5);
    assert_eq!(added["version"], 2);
    let (err, refused) = call(
        &app,
        &agent,
        "add_comment",
        json!({"parcel": id, "body": "x", "element_text": "Step 1b"}),
    )
    .await;
    assert!(err, "{refused}");
    let (err, _) = call(
        &app,
        &agent,
        "delete_comment",
        json!({"parcel": id, "number": 5}),
    )
    .await;
    assert!(!err);
}

#[tokio::test]
async fn comment_tools_follow_share_roles() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-o", "owner@example.com").await;
    let owner = mcp_token(
        &app,
        "sub-o",
        "owner@example.com",
        &["parcel:read", "parcel:write"],
    );
    let id = publish_page(&app, &owner, "<title>Doc</title>").await;
    for (email, role) in [
        ("viewer@example.com", "viewer"),
        ("commenter@example.com", "commenter"),
    ] {
        let (err, _) = call(
            &app,
            &owner,
            "share_parcel",
            json!({"parcel": id, "email": email, "role": role}),
        )
        .await;
        assert!(!err);
    }
    known_user(&app, "sub-v", "viewer@example.com").await;
    known_user(&app, "sub-c", "commenter@example.com").await;
    let viewer = mcp_token(
        &app,
        "sub-v",
        "viewer@example.com",
        &["parcel:read", "parcel:write"],
    );
    let commenter = mcp_token(
        &app,
        "sub-c",
        "commenter@example.com",
        &["parcel:read", "parcel:write"],
    );

    let (err, added) = call(
        &app,
        &commenter,
        "add_comment",
        json!({"parcel": id, "body": "Looks good"}),
    )
    .await;
    assert!(!err, "{added}");
    let (err, listed) = call(&app, &viewer, "list_comments", json!({"parcel": id})).await;
    assert!(!err, "{listed}");
    assert_eq!(listed["open"], 1);
    let (err, refused) = call(
        &app,
        &viewer,
        "add_comment",
        json!({"parcel": id, "body": "Me too"}),
    )
    .await;
    assert!(err);
    assert!(
        refused
            .as_str()
            .unwrap()
            .contains("to view, not to comment"),
        "{refused}"
    );
    // Resolving is for the owner or the thread's author.
    let (err, _) = call(
        &app,
        &viewer,
        "resolve_comments",
        json!({"parcel": id, "numbers": [1]}),
    )
    .await;
    assert!(err);
    let (err, _) = call(
        &app,
        &owner,
        "resolve_comments",
        json!({"parcel": id, "numbers": [1]}),
    )
    .await;
    assert!(!err);
}

#[tokio::test]
async fn page_data_is_read_by_viewers_and_written_by_the_owner() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-o", "owner@example.com").await;
    let owner = mcp_token(
        &app,
        "sub-o",
        "owner@example.com",
        &["parcel:read", "parcel:write"],
    );
    let id = publish_page(&app, &owner, "<title>Dash</title>").await;

    let (err, set) = call(
        &app,
        &owner,
        "set_data",
        json!({"parcel": id, "collection": "dashboard", "key": "metrics", "value": {"visits": 1, "errors": 0}}),
    )
    .await;
    assert!(!err, "{set}");
    let (err, merged) = call(
        &app,
        &owner,
        "update_data",
        json!({"parcel": id, "collection": "dashboard", "key": "metrics", "patch": {"visits": 42, "errors": null}}),
    )
    .await;
    assert!(!err, "{merged}");
    assert_eq!(merged["value"], json!({"visits": 42}));
    let (err, refused) = call(
        &app,
        &owner,
        "update_data",
        json!({"parcel": id, "collection": "dashboard", "key": "metrics", "patch": [1]}),
    )
    .await;
    assert!(err, "{refused}");

    let (_, summary) = call(&app, &owner, "list_data", json!({"parcel": id})).await;
    assert_eq!(summary["writable"], true);
    assert_eq!(summary["collections"][0]["name"], "dashboard");
    let (_, items) = call(
        &app,
        &owner,
        "list_data",
        json!({"parcel": id, "collection": "dashboard"}),
    )
    .await;
    assert_eq!(items["items"][0]["key"], "metrics");

    // A viewer reads it but can't change it.
    let (_, _) = call(
        &app,
        &owner,
        "share_parcel",
        json!({"parcel": id, "email": "v@example.com"}),
    )
    .await;
    known_user(&app, "sub-v", "v@example.com").await;
    let viewer = mcp_token(
        &app,
        "sub-v",
        "v@example.com",
        &["parcel:read", "parcel:write"],
    );
    let (err, got) = call(
        &app,
        &viewer,
        "get_data",
        json!({"parcel": id, "collection": "dashboard", "key": "metrics"}),
    )
    .await;
    assert!(!err, "{got}");
    assert_eq!(got["value"]["visits"], 42);
    let (_, summary) = call(&app, &viewer, "list_data", json!({"parcel": id})).await;
    assert_eq!(summary["writable"], false);
    let (err, _) = call(
        &app,
        &viewer,
        "set_data",
        json!({"parcel": id, "collection": "dashboard", "key": "metrics", "value": 0}),
    )
    .await;
    assert!(err);

    let (err, _) = call(
        &app,
        &owner,
        "delete_data",
        json!({"parcel": id, "collection": "dashboard"}),
    )
    .await;
    assert!(!err);
    let (_, summary) = call(&app, &owner, "list_data", json!({"parcel": id})).await;
    assert_eq!(summary["collections"], json!([]));
}

#[tokio::test]
async fn restore_makes_an_old_version_current() {
    let app = mcp_app(|_| {}).await;
    known_user(&app, "sub-r", "r@example.com").await;
    let token = mcp_token(
        &app,
        "sub-r",
        "r@example.com",
        &["parcel:read", "parcel:write"],
    );
    let id = publish_page(&app, &token, "<title>One</title>").await;
    // A new version, published or restored here, un-archives as it does
    // through the REST API (the version-change trigger, migration 0006).
    let archive = || async {
        sqlx::query("UPDATE parcels SET archived_at = now() WHERE id = $1")
            .bind(&id)
            .execute(&app.state.db)
            .await
            .unwrap();
    };
    let archived = || async {
        sqlx::query_scalar::<_, bool>("SELECT archived_at IS NOT NULL FROM parcels WHERE id = $1")
            .bind(&id)
            .fetch_one(&app.state.db)
            .await
            .unwrap()
    };
    archive().await;
    let (_, _) = call(
        &app,
        &token,
        "update_parcel",
        json!({"parcel": id, "mode": "replace", "files": [{"path": "index.html", "content": "<title>Two</title>"}]}),
    )
    .await;
    assert!(!archived().await, "update_parcel un-archives");
    archive().await;
    let (err, restored) = call(
        &app,
        &token,
        "restore_version",
        json!({"parcel": id, "version": 1}),
    )
    .await;
    assert!(!err, "{restored}");
    assert_eq!(restored["version"]["version"], 3);
    assert!(!archived().await, "restore_version un-archives");
    let (_, content) = read_file(&app, &token, json!({"parcel": id})).await;
    assert_eq!(content, "<title>One</title>");
}
