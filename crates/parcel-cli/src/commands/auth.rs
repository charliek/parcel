//! `login`, `logout`, `whoami`, `handle`.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parcel_types::slugs::normalize_handle;
use parcel_types::{
    DevicePollRequest, DevicePollResponse, DeviceStartRequest, DeviceStartResponse, DeviceStatus,
    Me, UpdateMeRequest,
};
use serde::Serialize;

use super::{authed_api, can_launch_browser, open_in_browser};
use crate::cli::{HandleArgs, LoginArgs};
use crate::client::Api;
use crate::config::{self, Profile, Settings, TokenSource};
use crate::error::{self, ApiError};
use crate::output::{self, format_size};

/// Added to the poll interval each time the server says `slow_down`.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

pub fn login(mut settings: Settings, args: LoginArgs) -> Result<()> {
    if settings.token_source == TokenSource::Env {
        eprintln!(
            "warning: PARCEL_TOKEN is set and will keep overriding the token saved by this login"
        );
    }
    warn_if_plaintext(&settings);

    let given = if args.with_token {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("reading a token from stdin")?;
        Some(input)
    } else {
        args.token
    };

    let (token, email) = match given {
        Some(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                return Err(error::usage("no token given"));
            }
            let me = verify_token(&settings, &token)?;
            (token, me.email)
        }
        None => device_flow(&settings, args.no_browser)?,
    };

    let mut profile = settings.profile_entry();
    profile.server = Some(settings.server_display());
    profile.token = Some(token);
    settings.save_profile(profile)?;
    output::print(&format!(
        "Logged in to {} as {email}\n",
        settings.server_display()
    ))
}

fn verify_token(settings: &Settings, token: &str) -> Result<Me> {
    let api = Api::new(&settings.server, Some(token))?;
    match api.get::<Me>(api.endpoint(["me"])) {
        Ok(me) => Ok(me.value),
        // The server's reason matters: a paused token needs a web sign-in,
        // not a different token.
        Err(e) => match e.downcast_ref::<ApiError>().filter(|a| a.status == 401) {
            Some(rejected) => Err(error::auth(format!(
                "{} rejected the token: {}",
                settings.server_display(),
                rejected.message
            ))),
            None => Err(e),
        },
    }
}

fn device_flow(settings: &Settings, no_browser: bool) -> Result<(String, String)> {
    let api = Api::new(&settings.server, None)?;
    let host = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    let client_name = if host.is_empty() {
        "parcel CLI".to_string()
    } else {
        format!("parcel CLI on {host}")
    };
    let start: DeviceStartResponse = api
        .post(
            api.endpoint(["cli", "device"]),
            &DeviceStartRequest {
                client_name: Some(client_name),
            },
        )
        .context("starting the device login")?
        .value;

    eprintln!(
        "To sign in, open this URL and confirm the code {}:\n\n  {}\n",
        start.user_code, start.verification_uri_complete
    );
    if !no_browser && can_launch_browser() {
        open_in_browser(&start.verification_uri_complete);
    }
    eprintln!("Waiting for approval (Ctrl-C to cancel)…");

    let deadline = Instant::now() + Duration::from_secs(start.expires_in.max(1) as u64);
    let mut interval = Duration::from_secs(start.interval.max(1) as u64);
    let expired =
        || error::auth("the login code expired before it was approved; run `parcel login` again");
    loop {
        std::thread::sleep(interval);
        if Instant::now() >= deadline {
            return Err(expired());
        }
        let poll: DevicePollResponse = api.post_lenient(
            api.endpoint(["cli", "device", "token"]),
            &DevicePollRequest {
                device_code: start.device_code.clone(),
            },
        )?;
        match poll.status {
            DeviceStatus::Pending => {}
            DeviceStatus::SlowDown => interval += SLOW_DOWN_STEP,
            DeviceStatus::Denied => return Err(error::auth("the login was denied in the browser")),
            DeviceStatus::Expired => return Err(expired()),
            DeviceStatus::Approved => {
                let token = poll
                    .token
                    .filter(|t| !t.is_empty())
                    .context("the server approved the login but sent no token")?;
                let email = match poll.email {
                    Some(email) => email,
                    None => verify_token(settings, &token)?.email,
                };
                return Ok((token, email));
            }
        }
    }
}

/// Tokens are bearer credentials; say so when one is about to cross a
/// network in the clear.
fn warn_if_plaintext(settings: &Settings) {
    let url = &settings.server;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "http" && !loopback {
        eprintln!(
            "warning: {} is plain http; your token will be sent unencrypted",
            settings.server_display()
        );
    }
}

pub fn logout(mut settings: Settings) -> Result<()> {
    let profile = settings.profile_entry();
    let Some(token) = profile.token.clone() else {
        output::print(&format!("Not logged in (profile {:?})\n", settings.profile))?;
        if settings.token_source == TokenSource::Env {
            eprintln!("note: PARCEL_TOKEN is set in the environment; unset it to stop using it");
        }
        return Ok(());
    };
    // Revoke against the server the token belongs to, not --server.
    let server = config::parse_server(profile.server.as_deref().unwrap_or(config::DEFAULT_SERVER))?;
    let server_display = config::server_string(&server);
    let api = Api::new(&server, Some(&token))?;
    match api.delete(api.endpoint(["tokens", "current"])) {
        Ok(()) => {}
        Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.status == 401) => {}
        Err(e) => eprintln!(
            "warning: could not revoke the token on {server_display} ({}); removing it locally anyway",
            error::format_error(&e)
        ),
    }
    settings.save_profile(Profile {
        token: None,
        ..profile
    })?;
    output::print(&format!("Logged out of {server_display}\n"))?;
    if settings.token_source == TokenSource::Env {
        eprintln!("note: PARCEL_TOKEN is still set in the environment");
    }
    Ok(())
}

pub fn whoami(settings: &Settings, json: bool) -> Result<()> {
    let api = authed_api(settings)?;
    let me = api.get::<Me>(api.endpoint(["me"]))?;
    if json {
        return output::print_json(&me.raw);
    }
    let Me {
        email,
        name,
        usage,
        limits,
        ..
    } = me.value;
    let mut out = String::new();
    if name.trim().is_empty() || name == email {
        out.push_str(&format!("{email}\n"));
    } else {
        out.push_str(&format!("{name} <{email}>\n"));
    }
    out.push_str(&format!("server:  {}\n", settings.server_display()));
    let quota = if limits.user_quota_bytes > 0 {
        format!(" of {}", format_size(limits.user_quota_bytes))
    } else {
        String::new()
    };
    out.push_str(&format!(
        "storage: {}{quota} ({} parcels, {} versions)\n",
        format_size(usage.stored_bytes),
        usage.parcel_count,
        usage.version_count
    ));
    output::print(&out)
}

/// `handle --json`.
#[derive(Serialize)]
struct HandleJson {
    handle: String,
    /// The handle before this change (links with it keep working).
    #[serde(skip_serializing_if = "Option::is_none")]
    previous: Option<String>,
}

pub fn handle(settings: &Settings, args: HandleArgs) -> Result<()> {
    let wanted = match &args.handle {
        Some(raw) => Some(normalize_handle(raw).map_err(|e| error::usage(e.to_string()))?),
        None => None,
    };
    let api = authed_api(settings)?;
    api.require_feature("slugs", "handles")?;
    let current = api.get::<Me>(api.endpoint(["me"]))?.value.handle;
    let (handle, previous) = match wanted {
        Some(new) if new != current => {
            let me = api
                .patch::<_, Me>(api.endpoint(["me"]), &UpdateMeRequest { handle: Some(new) })?
                .value;
            (me.handle, Some(current))
        }
        _ => (current, None),
    };
    if args.json {
        let body = HandleJson { handle, previous };
        return output::print_json(&serde_json::to_vec(&body)?);
    }
    // Clean URLs live on the web app's origin (already read for the feature check).
    let app = api
        .server_info()?
        .app_origin
        .trim_end_matches('/')
        .to_string();
    let example = format!("{app}/@{handle}/<slug>");
    output::print(&match previous {
        Some(old) => format!(
            "Your handle is now @{handle} (was @{old}); links with @{old} keep working.\n\
             Clean URLs: {example}\n"
        ),
        None => format!("@{handle}\nClean URLs: {example}\n"),
    })
}
