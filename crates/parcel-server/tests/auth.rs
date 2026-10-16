//! Browser sign-in end to end against a mock OIDC provider, and the session
//! cookie's rules (expiry, sign-out everywhere, CSRF, disabled users).

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use common::*;
use serde_json::{json, Value};
use stridelabs_auth::test_support::TestKey;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A sign-in started at `/auth/login`.
struct Login {
    /// `name=value` of the login cookie.
    cookie: String,
    state: String,
    nonce: String,
    challenge: String,
    authorize: url::Url,
}

fn set_cookies(response: &Response) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

/// `name=value` of the cookie `name` this response sets (with a value).
fn cookie_named(response: &Response, name: &str) -> Option<String> {
    set_cookies(response)
        .into_iter()
        .filter_map(|c| c.split(';').next().map(str::to_string))
        .find(|pair| pair.starts_with(&format!("{name}=")) && pair.len() > name.len() + 1)
}

fn location(response: &Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .expect("a redirect")
        .to_str()
        .unwrap()
        .to_string()
}

async fn get_with(app: &TestApp, uri: &str, cookie: Option<&str>) -> Response {
    let mut req = Request::builder()
        .uri(uri)
        .header(header::HOST, "parcel.test");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    app.send(req.body(Body::empty()).unwrap()).await
}

async fn start_login(app: &TestApp, query: &str) -> Login {
    let r = get_with(app, &format!("/auth/login{query}"), None).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let authorize = url::Url::parse(&location(&r)).unwrap();
    let q: std::collections::HashMap<String, String> =
        authorize.query_pairs().into_owned().collect();
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["redirect_uri"], format!("{APP_ORIGIN}/auth/callback"));
    assert_eq!(q["code_challenge_method"], "S256");
    Login {
        cookie: cookie_named(&r, "__Host-parcel_login").expect("login cookie"),
        state: q["state"].clone(),
        nonce: q["nonce"].clone(),
        challenge: q["code_challenge"].clone(),
        authorize,
    }
}

fn id_token(issuer: &str, nonce: &str, sub: &str, extra: Value) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let mut claims = json!({
        "iss": issuer, "aud": CLIENT_ID, "sub": sub, "nonce": nonce,
        "iat": now, "exp": now + 300,
        "email": format!("{sub}@example.com"), "email_verified": true, "name": "Ada Lovelace",
    });
    for (k, v) in extra.as_object().unwrap() {
        if v.is_null() {
            claims.as_object_mut().unwrap().remove(k);
        } else {
            claims[k] = v.clone();
        }
    }
    TestKey::primary().mint(&claims)
}

/// Answer the next code exchange (once, so a test can sign in again).
async fn mount_token(idp: &MockServer, id_token: &str) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "provider-access-token", "token_type": "Bearer",
            "id_token": id_token, "expires_in": 300,
        })))
        .up_to_n_times(1)
        .mount(idp)
        .await;
}

async fn callback(app: &TestApp, login: &Login, query: &str) -> Response {
    get_with(app, &format!("/auth/callback{query}"), Some(&login.cookie)).await
}

/// Sign in with these ID-token claim overrides; the callback's response.
async fn sign_in_with(app: &TestApp, sub: &str, extra: Value) -> Response {
    let login = start_login(app, "?provider=test&return_to=/p/abc").await;
    mount_token(
        &app.idp,
        &id_token(&app.idp.uri(), &login.nonce, sub, extra),
    )
    .await;
    callback(app, &login, &format!("?code=c0de&state={}", login.state)).await
}

fn new_sub() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

async fn me(app: &TestApp, session: &str) -> Response {
    get_with(app, "/api/v1/me", Some(session)).await
}

#[tokio::test]
async fn signs_in_through_the_provider() {
    let app = TestApp::new().await;
    let sub = new_sub();
    let login = start_login(
        &app,
        "?return_to=%2Fp%2Fabc%3Fv%3D2&login_hint=Ada%40Example.com",
    )
    .await;
    assert_eq!(
        login
            .authorize
            .query_pairs()
            .find(|(k, _)| k == "login_hint")
            .unwrap()
            .1,
        "ada@example.com"
    );
    mount_token(
        &app.idp,
        &id_token(&app.idp.uri(), &login.nonce, &sub, json!({})),
    )
    .await;

    let r = callback(&app, &login, &format!("?code=c0de&state={}", login.state)).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&r), "/p/abc?v=2");
    let session = cookie_named(&r, "__Host-parcel_session").expect("session cookie");
    assert!(
        set_cookies(&r)
            .iter()
            .any(|c| c.starts_with("__Host-parcel_login=;")),
        "login cookie cleared"
    );

    // PKCE: the server proved it holds the verifier behind the challenge.
    let requests = app.idp.received_requests().await.unwrap();
    let token_req = requests.iter().find(|r| r.url.path() == "/token").unwrap();
    let form: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(&token_req.body)
            .into_owned()
            .collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "c0de");
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(
        parcel_server::auth::oidc::challenge(&form["code_verifier"]),
        login.challenge
    );

    let me: parcel_types::Me =
        json(expect_status(me(&app, &session).await, StatusCode::OK).await).await;
    assert_eq!(me.email, format!("{sub}@example.com"));
    assert_eq!(me.name, "Ada Lovelace");
    assert_eq!(me.signed_in_with.as_deref(), Some("Test IdP"));
}

#[tokio::test]
async fn callbacks_that_dont_match_their_login_are_refused() {
    let app = TestApp::new().await;
    let login = start_login(&app, "?return_to=/p/x").await;
    mount_token(
        &app.idp,
        &id_token(&app.idp.uri(), &login.nonce, &new_sub(), json!({})),
    )
    .await;

    // Wrong state: someone else's code, or a replay.
    let r = callback(&app, &login, "?code=c0de&state=forged").await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fx&error=expired&provider=test"
    );
    // No login cookie at all.
    let r = get_with(
        &app,
        &format!("/auth/callback?code=c0de&state={}", login.state),
        None,
    )
    .await;
    assert_eq!(location(&r), "/signin?return_to=%2F&error=expired");
    // A code from a different issuer than this sign-in went to (RFC 9207).
    let r = callback(
        &app,
        &login,
        &format!(
            "?code=c0de&state={}&iss=https%3A%2F%2Fevil.example",
            login.state
        ),
    )
    .await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fx&error=failed&provider=test"
    );
    // The provider said no.
    let r = callback(
        &app,
        &login,
        &format!("?error=access_denied&state={}", login.state),
    )
    .await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fx&error=denied&provider=test"
    );
}

#[tokio::test]
async fn bad_id_tokens_are_refused() {
    let app = TestApp::new().await;
    for extra in [
        json!({"nonce": "not-the-one"}),
        json!({"aud": "someone-else"}),
        json!({"aud": [CLIENT_ID, "other"], "azp": "other"}),
        json!({"iss": "https://elsewhere.example"}),
        json!({"exp": 1_000}),
    ] {
        let r = sign_in_with(&app, &new_sub(), extra.clone()).await;
        assert_eq!(
            location(&r),
            "/signin?return_to=%2Fp%2Fabc&error=failed&provider=test",
            "{extra}"
        );
        assert!(
            cookie_named(&r, "__Host-parcel_session").is_none(),
            "{extra}"
        );
    }
    // Several audiences are fine when we're the authorized party.
    let r = sign_in_with(
        &app,
        &new_sub(),
        json!({"aud": [CLIENT_ID, "other"], "azp": CLIENT_ID}),
    )
    .await;
    assert_eq!(location(&r), "/p/abc");
}

#[tokio::test]
async fn emails_must_be_verified_unless_the_provider_is_trusted() {
    let app = TestApp::new().await;
    let r = sign_in_with(&app, &new_sub(), json!({"email_verified": false})).await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fabc&error=unverified&provider=test"
    );
    let r = sign_in_with(&app, &new_sub(), json!({"email_verified": "true"})).await;
    assert_eq!(location(&r), "/p/abc", "string booleans count");

    let trusting = TestApp::with_config(|c| c.providers[0].trust_email = true).await;
    let r = sign_in_with(&trusting, &new_sub(), json!({"email_verified": null})).await;
    assert_eq!(location(&r), "/p/abc");
}

#[tokio::test]
async fn providers_only_sign_in_their_domains() {
    let app =
        TestApp::with_config(|c| c.providers[0].allowed_domains = vec!["acme.com".into()]).await;
    let r = sign_in_with(&app, &new_sub(), json!({"email": "ada@gmail.com"})).await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fabc&error=domain&provider=test"
    );
    let r = sign_in_with(&app, &new_sub(), json!({"email": "Ada@ACME.com"})).await;
    assert_eq!(location(&r), "/p/abc");
}

#[tokio::test]
async fn userinfo_fills_in_a_missing_email() {
    let app = TestApp::new().await;
    let sub = new_sub();
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer provider-access-token",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": sub, "email": "from-userinfo@example.com", "email_verified": true,
        })))
        .mount(&app.idp)
        .await;
    let r = sign_in_with(&app, &sub, json!({"email": null, "email_verified": null})).await;
    assert_eq!(location(&r), "/p/abc");
    let session = cookie_named(&r, "__Host-parcel_session").unwrap();
    let me: parcel_types::Me = json(me(&app, &session).await).await;
    assert_eq!(me.email, "from-userinfo@example.com");

    // An answer about someone else is discarded.
    let r = sign_in_with(&app, &new_sub(), json!({"email": null})).await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fabc&error=failed&provider=test"
    );
}

#[tokio::test]
async fn a_second_provider_with_the_same_email_is_the_same_person() {
    let other = MockServer::start().await;
    mount_provider(&other).await;
    let other_issuer = other.uri();
    let app = TestApp::with_config(|c| {
        let mut second = c.providers[0].clone();
        second.id = "other".into();
        second.label = "Other IdP".into();
        second.issuer = other_issuer;
        c.providers.push(second);
    })
    .await;
    let email = format!("{}@example.com", new_sub());

    let r = sign_in_with(&app, &new_sub(), json!({"email": email})).await;
    let first: parcel_types::Me =
        json(me(&app, &cookie_named(&r, "__Host-parcel_session").unwrap()).await).await;

    // With two providers, /auth/login needs one named.
    let r = get_with(&app, "/auth/login?return_to=/x", None).await;
    assert_eq!(location(&r), "/signin?return_to=%2Fx");
    let login = start_login(&app, "?provider=other&return_to=/p/abc").await;
    assert!(login.authorize.as_str().starts_with(&other.uri()));
    mount_token(
        &other,
        &id_token(
            &other.uri(),
            &login.nonce,
            &new_sub(),
            json!({"email": email.to_uppercase()}),
        ),
    )
    .await;
    let r = callback(&app, &login, &format!("?code=c0de&state={}", login.state)).await;
    let session = cookie_named(&r, "__Host-parcel_session").unwrap();
    let second: parcel_types::Me = json(me(&app, &session).await).await;
    assert_eq!(second.id, first.id);
    assert_eq!(second.signed_in_with.as_deref(), Some("Other IdP"));

    let info: parcel_types::ServerInfo = json(app.get("/api/v1/info", None).await).await;
    let labels: Vec<&str> = info
        .auth
        .providers
        .iter()
        .map(|p| p.label.as_str())
        .collect();
    assert_eq!(labels, ["Test IdP", "Other IdP"]);
}

#[tokio::test]
async fn an_unreachable_provider_is_reported() {
    let app = TestApp::with_config(|c| c.providers[0].issuer = "http://127.0.0.1:9".into()).await;
    let r = get_with(&app, "/auth/login?return_to=/x", None).await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fx&error=unavailable&provider=test"
    );
}

#[tokio::test]
async fn return_to_never_leaves_the_app() {
    let app = TestApp::new().await;
    let login = start_login(&app, "?return_to=//evil.example/x").await;
    mount_token(
        &app.idp,
        &id_token(&app.idp.uri(), &login.nonce, &new_sub(), json!({})),
    )
    .await;
    let r = callback(&app, &login, &format!("?code=c0de&state={}", login.state)).await;
    assert_eq!(location(&r), "/");
}

#[tokio::test]
async fn broken_or_ended_sessions_are_401_and_cleared() {
    let app = TestApp::new().await;
    let session = app
        .session_cookie(&new_sub(), &format!("{}@example.com", new_sub()))
        .await;

    let tampered = format!("{}x", session);
    let r = me(&app, &tampered).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(set_cookies(&r)
        .iter()
        .any(|c| c.starts_with("__Host-parcel_session=;") && c.contains("Max-Age=0")));
    // Even on a route that serves anonymous callers.
    let r = get_with(&app, "/api/v1/parcels/abcdefghijkm", Some(&tampered)).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    // Sign out everywhere ends this session too.
    expect_status(me(&app, &session).await, StatusCode::OK).await;
    let r = app
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout?everywhere=true")
                .header(header::HOST, "parcel.test")
                .header(header::ORIGIN, APP_ORIGIN)
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(set_cookies(&r)
        .iter()
        .any(|c| c.starts_with("__Host-parcel_session=;")));
    let body: Value = json(r).await;
    assert_eq!(body["redirect"], "/");
    expect_status(me(&app, &session).await, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn logout_can_end_the_provider_session_too() {
    let app = TestApp::with_config(|c| c.providers[0].provider_logout = true).await;
    let r = sign_in_with(&app, &new_sub(), json!({})).await;
    let session = cookie_named(&r, "__Host-parcel_session").unwrap();
    let hint = cookie_named(&r, "__Secure-parcel_hint").expect("ID token kept for logout");
    assert!(set_cookies(&r)
        .iter()
        .any(|c| c.starts_with("__Secure-parcel_hint=") && c.contains("Path=/auth/logout")));
    let r = app
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header(header::HOST, "parcel.test")
                .header(header::ORIGIN, APP_ORIGIN)
                .header(header::COOKIE, format!("{session}; {hint}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let body: Value = json(expect_status(r, StatusCode::OK).await).await;
    let redirect = url::Url::parse(body["redirect"].as_str().unwrap()).unwrap();
    assert_eq!(redirect.path(), "/logout");
    let q: std::collections::HashMap<String, String> =
        redirect.query_pairs().into_owned().collect();
    assert_eq!(q["post_logout_redirect_uri"], format!("{APP_ORIGIN}/"));
    assert_eq!(q["id_token_hint"], hint.split_once('=').unwrap().1);
}

#[tokio::test]
async fn cookie_writes_need_the_apps_origin() {
    let app = TestApp::new().await;
    let session = app
        .session_cookie(&new_sub(), &format!("{}@example.com", new_sub()))
        .await;
    let post = |origin: Option<&'static str>, fetch_site: Option<&'static str>| {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/tokens")
            .header(header::HOST, "parcel.test")
            .header(header::COOKIE, &session)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        if let Some(s) = fetch_site {
            req = req.header("sec-fetch-site", s);
        }
        req.body(Body::from(r#"{"name":"t"}"#)).unwrap()
    };
    expect_status(app.send(post(None, None)).await, StatusCode::FORBIDDEN).await;
    expect_status(
        app.send(post(Some("https://evil.example"), None)).await,
        StatusCode::FORBIDDEN,
    )
    .await;
    expect_status(
        app.send(post(Some("null"), None)).await,
        StatusCode::FORBIDDEN,
    )
    .await;
    // A sibling subdomain is "same-site" to SameSite cookies, but not to us.
    expect_status(
        app.send(post(Some(APP_ORIGIN), Some("same-site"))).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    expect_status(
        app.send(post(Some(APP_ORIGIN), Some("same-origin"))).await,
        StatusCode::CREATED,
    )
    .await;

    let logout = Request::builder()
        .method(Method::POST)
        .uri("/auth/logout")
        .header(header::HOST, "parcel.test")
        .header(header::COOKIE, &session)
        .body(Body::empty())
        .unwrap();
    expect_status(app.send(logout).await, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn only_parcel_tokens_are_bearer_credentials() {
    let app = TestApp::new().await;
    let r = app
        .get("/api/v1/me", Some("eyJhbGciOiJSUzI1NiJ9.e30.sig"))
        .await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

async fn mint_pcl(app: &TestApp, user: &str) -> String {
    let created: parcel_types::CreatedToken = json(
        expect_status(
            app.post_json("/api/v1/tokens", Some(user), &json!({"name": "cli"}))
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    created.secret
}

#[tokio::test]
async fn tokens_pause_without_a_recent_web_sign_in() {
    let app = TestApp::new().await;
    let sub = new_sub();
    let user = token_for(&sub, &format!("{sub}@example.com"));
    let token = mint_pcl(&app, &user).await;
    expect_status(app.get("/api/v1/me", Some(&token)).await, StatusCode::OK).await;

    sqlx::query("UPDATE users SET last_login_at = now() - interval '31 days' WHERE email = $1")
        .bind(format!("{sub}@example.com"))
        .execute(&app.pool)
        .await
        .unwrap();
    let r = expect_status(
        app.get("/api/v1/me", Some(&token)).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let body: Value = json(r).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("sign in to Parcel on the web"),
        "{body}"
    );

    let lenient = TestApp::with_config(|c| c.token_login_max_age = None).await;
    let token = mint_pcl(&lenient, &user).await;
    sqlx::query("UPDATE users SET last_login_at = now() - interval '400 days' WHERE email = $1")
        .bind(format!("{sub}@example.com"))
        .execute(&lenient.pool)
        .await
        .unwrap();
    expect_status(
        lenient.get("/api/v1/me", Some(&token)).await,
        StatusCode::OK,
    )
    .await;
}

#[tokio::test]
async fn disabled_users_lose_sessions_and_tokens() {
    let app = TestApp::new().await;
    let sub = new_sub();
    let email = format!("{sub}@example.com");
    let session = app.session_cookie(&sub, &email).await;
    let token = mint_pcl(&app, &token_for(&sub, &email)).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = $1")
        .bind(&email)
        .execute(&app.pool)
        .await
        .unwrap();
    expect_status(me(&app, &session).await, StatusCode::UNAUTHORIZED).await;
    expect_status(
        app.get("/api/v1/me", Some(&token)).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    // And can't sign back in.
    let r = sign_in_with(&app, &sub, json!({})).await;
    assert_eq!(
        location(&r),
        "/signin?return_to=%2Fp%2Fabc&error=disabled&provider=test"
    );
}

#[tokio::test]
async fn dev_mode_login_goes_straight_back() {
    let app = TestApp::with_config(|c| c.auth_mode = parcel_server::config::AuthMode::Dev).await;
    let r = get_with(&app, "/auth/login?return_to=/p/abc", None).await;
    assert_eq!(location(&r), "/p/abc");
    let info: parcel_types::ServerInfo = json(app.get("/api/v1/info", None).await).await;
    assert_eq!(info.auth.mode, "dev");
    assert!(info.auth.providers.is_empty());
}

#[tokio::test]
async fn id_tokens_must_be_signed_by_the_providers_keys() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine as _;
    let app = TestApp::new().await;
    let issuer = app.idp.uri();
    let claims = |nonce: &str| {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        json!({
            "iss": issuer, "aud": CLIENT_ID, "sub": new_sub(), "nonce": nonce,
            "iat": now, "exp": now + 300, "email": "x@example.com", "email_verified": true,
        })
    };
    type Forge<'a> = Box<dyn Fn(&str) -> String + 'a>;
    let forgeries: Vec<Forge<'_>> = vec![
        // A key the provider doesn't publish, under its own kid…
        Box::new(|n| TestKey::secondary().mint(&claims(n))),
        // …or claiming the provider's kid.
        Box::new(|n| TestKey::secondary().mint_with_kid(TestKey::primary().kid(), &claims(n))),
        // A genuine signature over different claims.
        Box::new(|n| {
            let genuine = TestKey::primary().mint(&claims("other-nonce"));
            let mut parts: Vec<String> = genuine.split('.').map(String::from).collect();
            parts[1] = B64.encode(serde_json::to_vec(&claims(n)).unwrap());
            parts.join(".")
        }),
        // HS256 "signed" with the provider's public key material.
        Box::new(|n| {
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
            header.kid = Some(TestKey::primary().kid().into());
            let key =
                jsonwebtoken::EncodingKey::from_secret(TestKey::primary().jwks_json().as_bytes());
            jsonwebtoken::encode(&header, &claims(n), &key).unwrap()
        }),
        // No signature at all.
        Box::new(|n| {
            format!(
                "{}.{}.",
                B64.encode(r#"{"alg":"none","kid":"stridelabs-test-key-1"}"#),
                B64.encode(serde_json::to_vec(&claims(n)).unwrap())
            )
        }),
    ];
    for (i, forge) in forgeries.iter().enumerate() {
        let login = start_login(&app, "?return_to=/p/abc").await;
        mount_token(&app.idp, &forge(&login.nonce)).await;
        let r = callback(&app, &login, &format!("?code=c0de&state={}", login.state)).await;
        assert_eq!(
            location(&r),
            "/signin?return_to=%2Fp%2Fabc&error=failed&provider=test",
            "forgery {i}"
        );
    }
}

#[tokio::test]
async fn signing_in_again_replaces_the_logout_hint() {
    let app = TestApp::with_config(|c| c.providers[0].provider_logout = true).await;
    let sub = new_sub();
    let r = sign_in_with(&app, &sub, json!({})).await;
    assert!(cookie_named(&r, "__Secure-parcel_hint").is_some());
    // A token too big to keep as a cookie: the old hint must not linger.
    let r = sign_in_with(&app, &sub, json!({"padding": "x".repeat(4000)})).await;
    assert_eq!(location(&r), "/p/abc");
    assert!(cookie_named(&r, "__Secure-parcel_hint").is_none());
    assert!(set_cookies(&r)
        .iter()
        .any(|c| c.starts_with("__Secure-parcel_hint=;") && c.contains("Max-Age=0")));
}

#[tokio::test]
async fn an_ended_session_cant_sign_out_everyone_again() {
    let app = TestApp::new().await;
    let (sub, email) = (new_sub(), format!("{}@example.com", new_sub()));
    let old = app.session_cookie(&sub, &email).await;
    let logout_everywhere = |cookie: String| {
        Request::builder()
            .method(Method::POST)
            .uri("/auth/logout?everywhere=true")
            .header(header::HOST, "parcel.test")
            .header(header::ORIGIN, APP_ORIGIN)
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap()
    };
    expect_status(
        app.send(logout_everywhere(old.clone())).await,
        StatusCode::OK,
    )
    .await;
    let fresh = app.session_cookie(&sub, &email).await;
    expect_status(me(&app, &fresh).await, StatusCode::OK).await;
    // Replaying the ended session is harmless.
    expect_status(app.send(logout_everywhere(old)).await, StatusCode::OK).await;
    expect_status(me(&app, &fresh).await, StatusCode::OK).await;
}

#[tokio::test]
async fn concurrent_first_sign_ins_make_one_user() {
    let app = TestApp::new().await;
    let email = format!("{}@example.com", new_sub());
    let (sub_a, sub_b) = (new_sub(), new_sub());
    let (a, b) = tokio::join!(
        parcel_server::db::users::sign_in(&app.pool, "test", &sub_a, &email, "A"),
        parcel_server::db::users::sign_in(&app.pool, "other", &sub_b, &email, "B"),
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
}
