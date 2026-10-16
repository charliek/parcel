//! API tokens and the CLI device login.

mod common;

use axum::http::StatusCode;
use common::*;
use parcel_types::{
    CreatedToken, DevicePollResponse, DeviceStartResponse, DeviceStatus, TokenList,
};
use serde_json::json;

#[tokio::test]
async fn tokens_authenticate_and_revoke() {
    let app = TestApp::new().await;
    let jwt = user_token();
    let created: CreatedToken = json(
        expect_status(
            app.post_json(
                "/api/v1/tokens",
                Some(&jwt),
                &json!({"name": "laptop", "expires_in_days": 30}),
            )
            .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert!(created.secret.starts_with("pcl_"));
    assert!(created.secret.starts_with(&created.token.prefix));
    assert!(created.token.expires_at.is_some());

    // The token is a full credential for the same user.
    let via_jwt: parcel_types::Me = json(app.get("/api/v1/me", Some(&jwt)).await).await;
    let via_token: parcel_types::Me =
        json(app.get("/api/v1/me", Some(&created.secret)).await).await;
    assert_eq!(via_jwt.id, via_token.id);

    let list: TokenList = json(app.get("/api/v1/tokens", Some(&jwt)).await).await;
    assert_eq!(list.tokens.len(), 1);
    assert_eq!(list.tokens[0].name, "laptop");

    // Revoking one of my tokens by id…
    expect_status(
        app.delete(&format!("/api/v1/tokens/{}", created.token.id), Some(&jwt))
            .await,
        StatusCode::NO_CONTENT,
    )
    .await;
    expect_status(
        app.get("/api/v1/me", Some(&created.secret)).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    // …but not someone else's.
    let other = user_token();
    expect_status(
        app.delete(
            &format!("/api/v1/tokens/{}", created.token.id),
            Some(&other),
        )
        .await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn current_token_can_revoke_itself() {
    let app = TestApp::new().await;
    let jwt = user_token();
    let created: CreatedToken = json(
        app.post_json("/api/v1/tokens", Some(&jwt), &json!({"name": "cli"}))
            .await,
    )
    .await;
    expect_status(
        app.delete("/api/v1/tokens/current", Some(&jwt)).await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    expect_status(
        app.delete("/api/v1/tokens/current", Some(&created.secret))
            .await,
        StatusCode::NO_CONTENT,
    )
    .await;
    expect_status(
        app.get("/api/v1/me", Some(&created.secret)).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn token_requests_are_validated() {
    let app = TestApp::new().await;
    let jwt = user_token();
    for body in [
        json!({"name": ""}),
        json!({"name": "x", "expires_in_days": 0}),
        json!({"nme": "x"}),
    ] {
        let r = app.post_json("/api/v1/tokens", Some(&jwt), &body).await;
        assert!(r.status().is_client_error(), "{body}: {}", r.status());
    }
    expect_status(
        app.get("/api/v1/me", Some("pcl_notarealtoken")).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

async fn start(app: &TestApp) -> DeviceStartResponse {
    json(
        expect_status(
            app.post_json(
                "/api/v1/cli/device",
                None,
                &json!({"client_name": "parcel CLI on test"}),
            )
            .await,
            StatusCode::OK,
        )
        .await,
    )
    .await
}

async fn poll(app: &TestApp, device_code: &str) -> DevicePollResponse {
    json(
        app.post_json(
            "/api/v1/cli/device/token",
            None,
            &json!({"device_code": device_code}),
        )
        .await,
    )
    .await
}

#[tokio::test]
async fn device_login_approve_flow() {
    let app = TestApp::new().await;
    let started = start(&app).await;
    assert_eq!(started.verification_uri, format!("{APP_ORIGIN}/device"));
    assert!(started
        .verification_uri_complete
        .ends_with(&started.user_code));
    assert_eq!(started.interval, 5);

    assert_eq!(
        poll(&app, &started.device_code).await.status,
        DeviceStatus::Pending
    );
    assert_eq!(
        poll(&app, &started.device_code).await.status,
        DeviceStatus::SlowDown,
        "polling faster than the interval"
    );

    let jwt = user_token();
    // The approval page can look the code up in any spelling.
    let lowered = started.user_code.to_lowercase().replace('-', "");
    let info: parcel_types::DeviceInfo = json(
        app.get(&format!("/api/v1/cli/device/{lowered}"), Some(&jwt))
            .await,
    )
    .await;
    assert_eq!(info.client_name, "parcel CLI on test");
    expect_status(
        app.get(&format!("/api/v1/cli/device/{}", started.user_code), None)
            .await,
        StatusCode::UNAUTHORIZED,
    )
    .await;

    expect_status(
        app.post_json(
            &format!("/api/v1/cli/device/{}/approve", started.user_code),
            Some(&jwt),
            &json!({}),
        )
        .await,
        StatusCode::NO_CONTENT,
    )
    .await;

    let approved = poll(&app, &started.device_code).await;
    assert_eq!(approved.status, DeviceStatus::Approved);
    let secret = approved.token.expect("token delivered");
    assert!(secret.starts_with("pcl_"));
    let me: parcel_types::Me = json(app.get("/api/v1/me", Some(&secret)).await).await;
    assert_eq!(Some(me.email), approved.email);

    // Exactly once.
    let again = poll(&app, &started.device_code).await;
    assert_eq!(again.status, DeviceStatus::Expired);
    assert!(again.token.is_none());
    // And the code can't be approved twice.
    expect_status(
        app.post_json(
            &format!("/api/v1/cli/device/{}/approve", started.user_code),
            Some(&jwt),
            &json!({}),
        )
        .await,
        StatusCode::NOT_FOUND,
    )
    .await;

    // The minted token is listed as a device token.
    let tokens: TokenList = json(app.get("/api/v1/tokens", Some(&jwt)).await).await;
    assert_eq!(tokens.tokens[0].created_via, "device");
}

#[tokio::test]
async fn device_login_deny_and_unknown_codes() {
    let app = TestApp::new().await;
    let started = start(&app).await;
    let jwt = user_token();
    expect_status(
        app.post_json(
            &format!("/api/v1/cli/device/{}/deny", started.user_code),
            Some(&jwt),
            &json!({}),
        )
        .await,
        StatusCode::NO_CONTENT,
    )
    .await;
    assert_eq!(
        poll(&app, &started.device_code).await.status,
        DeviceStatus::Denied
    );
    assert_eq!(
        poll(&app, "not-a-device-code").await.status,
        DeviceStatus::Expired
    );
    expect_status(
        app.get("/api/v1/cli/device/BCDF-GHJK", Some(&jwt)).await,
        StatusCode::NOT_FOUND,
    )
    .await;
}
