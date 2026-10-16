//! Browser sign-in: `/auth/login`, `/auth/callback`, `/auth/logout`.
//!
//! These are redirects and cookies for the web app, not API calls, so they
//! live outside `/api/v1` and its spec. The flow is OIDC authorization code
//! with PKCE and a nonce; the only state between the two legs is a signed,
//! ten-minute cookie (see [`cookie::LoginTx`]). Failures land on the SPA's
//! `/signin?error=<code>` page.

use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, LOCATION, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use parcel_types::{email_domain, normalize_email};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;
use url::form_urlencoded;

use crate::auth::cookie::{self, LoginTx, Session};
use crate::auth::oidc::random_token;
use crate::config::AuthMode;
use crate::db::users;
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
}

/// A path on this origin to return to; anything else becomes `/`.
pub fn safe_return_to(raw: Option<&str>) -> String {
    match raw {
        Some(p)
            if p.starts_with('/')
                && !p.starts_with("//")
                && !p.starts_with("/\\")
                && p.len() <= 2048
                && !p.chars().any(|c| c.is_control() || c == '\\') =>
        {
            p.to_string()
        }
        _ => "/".to_string(),
    }
}

fn encode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn redirect(location: &str, cookies: impl IntoIterator<Item = HeaderValue>) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(
        LOCATION,
        HeaderValue::from_str(location).unwrap_or_else(|_| HeaderValue::from_static("/")),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    cookie::append(headers, cookies);
    response
}

/// Back to the sign-in page with a short reason code and, when known, the
/// provider it happened with (so the page can name it).
fn sign_in_page(
    state: &AppState,
    error: Option<&str>,
    provider: Option<&str>,
    return_to: &str,
) -> Response {
    let mut location = format!("/signin?return_to={}", encode(return_to));
    if let Some(e) = error {
        location.push_str(&format!("&error={e}"));
    }
    if let Some(p) = provider {
        location.push_str(&format!("&provider={}", encode(p)));
    }
    redirect(&location, [cookie::clear_login(&state.config)])
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    provider: Option<String>,
    return_to: Option<String>,
    login_hint: Option<String>,
}

async fn login(State(state): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let return_to = safe_return_to(q.return_to.as_deref());
    if state.config.auth_mode == AuthMode::Dev {
        return redirect(&return_to, []);
    }
    let provider = match q.provider.as_deref() {
        Some(id) => state.provider(id),
        None if state.providers.len() == 1 => state.providers.first(),
        None => None,
    };
    let Some(provider) = provider else {
        return sign_in_page(&state, None, None, &return_to);
    };
    let login_hint = q
        .login_hint
        .as_deref()
        .and_then(|h| normalize_email(h).ok());

    let tx = LoginTx {
        provider: provider.config.id.clone(),
        state: random_token(),
        nonce: random_token(),
        verifier: random_token(),
        return_to: return_to.clone(),
        expires_at: OffsetDateTime::now_utc().unix_timestamp() + cookie::LOGIN_TTL_SECS,
    };
    match provider
        .authorize_url(
            &state.oidc_http,
            &state.config.auth_callback_url(),
            &tx.state,
            &tx.nonce,
            &tx.verifier,
            login_hint.as_deref(),
        )
        .await
    {
        Ok(url) => redirect(
            &url,
            [cookie::set_login(
                &state.config,
                &cookie::seal_login(&state.config, &tx),
            )],
        ),
        Err(e) => {
            tracing::warn!(provider = %provider.config.id, error = %e, "sign-in provider unavailable");
            sign_in_page(
                &state,
                Some("unavailable"),
                Some(&provider.config.id),
                &return_to,
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    /// RFC 9207: the issuer that answered, when the provider says.
    iss: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let config = &state.config;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let Some(tx) = cookie::read(&headers, cookie::names(config).login)
        .and_then(|raw| cookie::open_login(config, raw, now))
    else {
        return sign_in_page(&state, Some("expired"), None, "/");
    };
    // The state ties this callback to the sign-in this browser started, so
    // nobody can complete their own sign-in in someone else's browser.
    let state_matches = q
        .state
        .as_deref()
        .is_some_and(|s| bool::from(s.as_bytes().ct_eq(tx.state.as_bytes())));
    if !state_matches {
        return sign_in_page(&state, Some("expired"), Some(&tx.provider), &tx.return_to);
    }
    if let Some(error) = q.error.as_deref() {
        // OAuth error codes are `[a-z_]`; anything else is not worth logging raw.
        let logged: String = error
            .chars()
            .take(64)
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        tracing::info!(provider = %tx.provider, error = %logged, "provider refused sign-in");
        let code = if error == "access_denied" {
            "denied"
        } else {
            "failed"
        };
        return sign_in_page(&state, Some(code), Some(&tx.provider), &tx.return_to);
    }
    let (Some(code), Some(provider)) = (q.code.as_deref(), state.provider(&tx.provider)) else {
        return sign_in_page(&state, Some("failed"), Some(&tx.provider), &tx.return_to);
    };
    // With several providers, a code must come back from the one this
    // sign-in went to (RFC 9207's mix-up defense), when it names itself.
    if q.iss
        .as_deref()
        .is_some_and(|iss| iss != provider.config.issuer)
    {
        tracing::warn!(provider = %tx.provider, "sign-in refused: callback from another issuer");
        return sign_in_page(&state, Some("failed"), Some(&tx.provider), &tx.return_to);
    }

    let signed_in = match provider
        .redeem(
            &state.oidc_http,
            &config.auth_callback_url(),
            code,
            &tx.verifier,
            &tx.nonce,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(provider = %tx.provider, error = %e, "sign-in failed");
            return sign_in_page(&state, Some("failed"), Some(&tx.provider), &tx.return_to);
        }
    };
    let identity = &signed_in.identity;
    let Ok(email) = normalize_email(&identity.email) else {
        tracing::info!(provider = %tx.provider, "sign-in refused: no usable email");
        return sign_in_page(
            &state,
            Some("unverified"),
            Some(&tx.provider),
            &tx.return_to,
        );
    };
    // Shares and domain rules go by email, so it must be one the provider
    // vouches for.
    if !identity.email_verified && !provider.config.trust_email {
        tracing::info!(provider = %tx.provider, "sign-in refused: email not verified");
        return sign_in_page(
            &state,
            Some("unverified"),
            Some(&tx.provider),
            &tx.return_to,
        );
    }
    if !provider.config.allows_domain(email_domain(&email)) {
        tracing::info!(provider = %tx.provider, domain = %email_domain(&email), "sign-in refused: domain not allowed");
        return sign_in_page(&state, Some("domain"), Some(&tx.provider), &tx.return_to);
    }

    let user = match users::sign_in(
        &state.db,
        &tx.provider,
        &identity.subject,
        &email,
        &identity.name,
    )
    .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "recording sign-in failed");
            return sign_in_page(&state, Some("failed"), Some(&tx.provider), &tx.return_to);
        }
    };
    if user.disabled_at.is_some() {
        return sign_in_page(&state, Some("disabled"), Some(&tx.provider), &tx.return_to);
    }
    let ttl = i64::try_from(config.session_ttl.as_secs()).unwrap_or(i64::MAX);
    let session = Session {
        user_id: user.id,
        epoch: user.session_epoch,
        issued_at: now,
        expires_at: now.saturating_add(ttl),
        provider: tx.provider.clone(),
    };
    tracing::info!(user = %user.id, provider = %tx.provider, "signed in");
    let mut cookies = vec![
        cookie::set_session(config, &cookie::seal_session(config, &session)),
        cookie::clear_login(config),
    ];
    // Replace or clear the logout hint, so an earlier sign-in's ID token is
    // never handed to this provider.
    cookies.push(
        provider
            .config
            .provider_logout
            .then(|| cookie::set_hint(config, &signed_in.id_token))
            .flatten()
            .unwrap_or_else(|| cookie::clear_hint(config)),
    );
    redirect(&tx.return_to, cookies)
}

#[derive(Debug, Deserialize)]
struct LogoutQuery {
    #[serde(default)]
    everywhere: bool,
}

#[derive(serde::Serialize)]
struct LogoutResponse {
    /// Where to send the browser next: the provider's logout page, or `/`.
    redirect: String,
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LogoutQuery>,
) -> Result<Response, AppError> {
    let config = &state.config;
    let origin = headers.get(ORIGIN).and_then(|v| v.to_str().ok());
    if origin != Some(config.app_origin_str().as_str()) {
        return Err(AppError::Forbidden("cross-origin request refused".into()));
    }
    let names = cookie::names(config);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let session = cookie::read(&headers, names.session)
        .and_then(|raw| cookie::open_session(config, raw, now));
    // Only a session that still works may end the others: an old cookie
    // from before an earlier "everywhere" can't keep signing its user out.
    if let (true, Some(s)) = (q.everywhere, &session) {
        users::bump_session_epoch(&state.db, s.user_id, s.epoch).await?;
    }
    let home = format!("{}/", config.app_origin_str());
    let mut redirect = "/".to_string();
    if let Some(provider) = session
        .as_ref()
        .and_then(|s| state.provider(&s.provider))
        .filter(|p| p.config.provider_logout)
    {
        let hint = cookie::read(&headers, names.hint);
        if let Some(url) = provider
            .end_session_url(&state.oidc_http, hint, &home)
            .await
        {
            redirect = url;
        }
    }
    let mut response = Json(LogoutResponse { redirect }).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    cookie::append(
        response.headers_mut(),
        [cookie::clear_session(config), cookie::clear_hint(config)],
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::safe_return_to;

    #[test]
    fn return_to_stays_on_this_origin() {
        assert_eq!(safe_return_to(Some("/p/abc?v=2#x")), "/p/abc?v=2#x");
        for bad in [
            None,
            Some(""),
            Some("https://evil.example/"),
            Some("//evil.example"),
            Some("/\\evil.example"),
            Some("/a\\b"),
            Some("/a\nb"),
            Some("p/abc"),
        ] {
            assert_eq!(safe_return_to(bad), "/", "{bad:?}");
        }
    }
}
