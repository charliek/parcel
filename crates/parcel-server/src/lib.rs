//! parcel-server — publish HTML pages (plus supporting files) to private,
//! sandboxed URLs.
//!
//! All logic lives in this library; `main.rs` only parses the CLI, loads
//! config, initializes logging, and dispatches, which keeps the server
//! testable without binding sockets.

pub mod auth;
pub mod config;
pub mod content;
pub mod db;
pub mod error;
pub mod gc;
pub mod http;
pub mod mcp;
pub mod publish;
pub mod search;
pub mod state;
pub mod storage;

pub use http::openapi;

use stridelabs_observability::TargetCaps;
use tracing::level_filters::LevelFilter;

/// The log targets capped whatever `RUST_LOG` says (invariant 5).
/// `main.rs` installs these at startup and refuses to start without them.
///
/// - stridelabs-postgres's sqlx targets, which can print a raw `sqlx::Error`
///   (the server's message quotes the value it rejected), a URL parameter's
///   value or a server notice.
/// - `rmcp`, at ERROR: below that it logs MCP content whole, at DEBUG each
///   request (a `tools/call`'s arguments: page HTML, queries, page data) and
///   each result (file contents, comment text, page data, and for malformed
///   arguments serde's error quoting the caller's value), at TRACE every
///   transport message. Its ERROR lines carry only transport and task
///   failures.
/// - Two rmcp modules at OFF, whose ERROR lines can quote a client message
///   whole (`{:?}` of an unexpected session event, or of whatever came
///   instead of `initialize`). Both sit on rmcp's session paths, which
///   parcel's stateless transport never takes; the caps hold if it ever
///   does.
///
/// Every cap covering a target applies, so the strictest wins: the module
/// caps hold under the crate-wide one. Each rmcp bump means re-auditing its
/// log sites (`error!` especially, the level this lets through).
pub fn log_cap_list() -> Vec<(&'static str, LevelFilter)> {
    let mut caps = stridelabs_postgres::LOG_CAPS.to_vec();
    caps.extend([
        ("rmcp", LevelFilter::ERROR),
        ("rmcp::transport::worker", LevelFilter::OFF),
        (
            "rmcp::transport::streamable_http_server::tower",
            LevelFilter::OFF,
        ),
    ]);
    caps
}

/// [`log_cap_list`] as the caps `try_init_logging_with` takes.
pub fn log_caps() -> TargetCaps {
    log_cap_list()
        .into_iter()
        .fold(TargetCaps::new(), |caps, (target, level)| {
            caps.cap(target, level)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rmcp logs MCP requests and results whole below ERROR, and two of its
    /// modules can quote a client message at ERROR.
    #[test]
    fn log_caps_hold_rmcp_to_its_content_free_errors() {
        let list = log_cap_list();
        for cap in [
            ("rmcp", LevelFilter::ERROR),
            ("rmcp::transport::worker", LevelFilter::OFF),
            (
                "rmcp::transport::streamable_http_server::tower",
                LevelFilter::OFF,
            ),
        ] {
            assert!(list.contains(&cap), "{cap:?} missing from {list:?}");
        }
    }

    /// The pool's background work logs a raw `sqlx::Error` under
    /// `sqlx_core::pool`; it must be off at every level, and every target
    /// stridelabs-postgres names must reach the caps main.rs installs.
    #[test]
    fn log_caps_silence_the_sqlx_pool() {
        let list = log_cap_list();
        assert!(
            list.contains(&("sqlx_core::pool", LevelFilter::OFF)),
            "{list:?}"
        );
        for cap in stridelabs_postgres::LOG_CAPS {
            assert!(list.contains(&cap), "{cap:?} missing from {list:?}");
        }
        let installed = format!("{:?}", log_caps());
        for (target, _) in &list {
            assert!(installed.contains(target), "{target} not in {installed}");
        }
    }
}
