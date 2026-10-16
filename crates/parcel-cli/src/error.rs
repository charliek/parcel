//! Exit codes and user-facing error formatting.
//!
//! Commands return `anyhow::Result`. Anything that should exit with a code
//! other than 1 carries a [`CliError`] or an [`ApiError`] somewhere in its
//! chain; [`exit_code`] finds it. Messages are printed once, by `main`, as
//! `error: <message>`.

use std::fmt;

use parcel_types::ErrorBody;

/// Success.
pub const EXIT_OK: u8 = 0;
/// Any failure without a more specific code.
pub const EXIT_ERROR: u8 = 1;
/// Bad arguments (clap uses the same code for its own parse errors).
pub const EXIT_USAGE: u8 = 2;
/// Not logged in, or the server rejected the credentials (HTTP 401).
pub const EXIT_AUTH: u8 = 3;
/// The parcel, version or file does not exist (HTTP 404).
pub const EXIT_NOT_FOUND: u8 = 4;

/// A failure detected by the CLI itself, with the exit code it maps to.
#[derive(Debug)]
pub struct CliError {
    pub code: u8,
    pub message: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

pub fn usage(message: impl Into<String>) -> anyhow::Error {
    CliError {
        code: EXIT_USAGE,
        message: message.into(),
    }
    .into()
}

pub fn auth(message: impl Into<String>) -> anyhow::Error {
    CliError {
        code: EXIT_AUTH,
        message: message.into(),
    }
    .into()
}

/// A non-2xx response from the API, decoded from the
/// `{"error": {"message", "type"}}` envelope when the body has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    /// Build from a status and raw body. Falls back to a short excerpt of a
    /// non-JSON body (a proxy's HTML error page, axum's plain-text 413), then
    /// to the status line.
    pub fn from_response(status: u16, body: &[u8]) -> Self {
        let message = match serde_json::from_slice::<ErrorBody>(body) {
            Ok(envelope) if !envelope.error.message.trim().is_empty() => {
                envelope.error.message.trim().to_string()
            }
            Ok(envelope) if !envelope.error.kind.is_empty() => envelope.error.kind,
            _ => plain_excerpt(body).unwrap_or_else(|| status_line(status)),
        };
        ApiError { status, message }
    }

    pub fn exit_code(&self) -> u8 {
        match self.status {
            401 => EXIT_AUTH,
            404 => EXIT_NOT_FOUND,
            _ => EXIT_ERROR,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        // A 401 that already says how to sign in is its own instruction: a
        // paused token ("sign in to Parcel on the web …") works again after
        // a web sign-in, with no new token needed.
        if self.status == 401 && !self.message.to_ascii_lowercase().contains("sign in") {
            f.write_str(" (run `parcel login` to sign in)")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// A one-line excerpt of a short text body, or `None` for HTML/binary/empty.
fn plain_excerpt(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?.trim();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    let line: String = text.lines().next()?.chars().take(200).collect();
    Some(line)
}

fn status_line(status: u16) -> String {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("error");
    format!("server returned HTTP {status} {reason}")
}

/// The exit code for an error: the first [`CliError`] or [`ApiError`] in the
/// chain decides, everything else is 1.
pub fn exit_code(err: &anyhow::Error) -> u8 {
    for cause in err.chain() {
        if let Some(e) = cause.downcast_ref::<CliError>() {
            return e.code;
        }
        if let Some(e) = cause.downcast_ref::<ApiError>() {
            return e.exit_code();
        }
    }
    EXIT_ERROR
}

/// The text after `error: `. Context layers are joined with `: `.
pub fn format_error(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

/// Whether the failure is a closed stdout (`parcel cat … | head`), which is
/// not an error worth reporting.
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn decodes_the_error_envelope() {
        let err = ApiError::from_response(
            404,
            br#"{"error":{"message":"parcel not found","type":"Not Found"}}"#,
        );
        assert_eq!(err.message, "parcel not found");
        assert_eq!(err.exit_code(), EXIT_NOT_FOUND);
        assert_eq!(err.to_string(), "parcel not found");
    }

    #[test]
    fn unauthorized_suggests_login() {
        let err = ApiError::from_response(
            401,
            br#"{"error":{"message":"invalid token","type":"Unauthorized"}}"#,
        );
        assert_eq!(err.exit_code(), EXIT_AUTH);
        assert_eq!(
            err.to_string(),
            "invalid token (run `parcel login` to sign in)"
        );
    }

    #[test]
    fn a_paused_token_keeps_the_servers_advice() {
        let message = "this token is paused: sign in to Parcel on the web \
                       (https://parcel.example) to keep using it \
                       (tokens need a web sign-in at least every 30 days)";
        let err = ApiError::from_response(
            401,
            serde_json::json!({"error": {"message": message, "type": "Unauthorized"}})
                .to_string()
                .as_bytes(),
        );
        assert_eq!(err.exit_code(), EXIT_AUTH);
        assert_eq!(err.to_string(), message);
    }

    #[test]
    fn falls_back_for_non_json_bodies() {
        let err = ApiError::from_response(413, b"length limit exceeded\n");
        assert_eq!(err.message, "length limit exceeded");
        assert_eq!(err.exit_code(), EXIT_ERROR);

        let err = ApiError::from_response(502, b"<html><body>Bad gateway</body></html>");
        assert_eq!(err.message, "server returned HTTP 502 Bad Gateway");

        let err = ApiError::from_response(500, b"");
        assert_eq!(
            err.message,
            "server returned HTTP 500 Internal Server Error"
        );
    }

    #[test]
    fn exit_codes_follow_the_chain() {
        let err = usage("bad flag");
        assert_eq!(exit_code(&err), EXIT_USAGE);

        let err: anyhow::Error = ApiError::from_response(404, b"").into();
        let wrapped = Err::<(), _>(err).context("reading parcel").unwrap_err();
        assert_eq!(exit_code(&wrapped), EXIT_NOT_FOUND);
        assert_eq!(
            format_error(&wrapped),
            "reading parcel: server returned HTTP 404 Not Found"
        );

        assert_eq!(exit_code(&anyhow::anyhow!("boom")), EXIT_ERROR);
        assert_eq!(exit_code(&auth("log in")), EXIT_AUTH);
    }

    #[test]
    fn detects_broken_pipes() {
        let io = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        let err = Err::<(), _>(io).context("writing").unwrap_err();
        assert!(is_broken_pipe(&err));
        assert!(!is_broken_pipe(&anyhow::anyhow!("other")));
    }
}
