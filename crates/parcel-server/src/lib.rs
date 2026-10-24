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
pub mod state;
pub mod storage;

pub use http::openapi;

use stridelabs_observability::TargetCaps;
use tracing::level_filters::LevelFilter;

/// The log targets capped whatever `RUST_LOG` says (invariant 5):
/// stridelabs-postgres's sqlx targets, which can print a raw `sqlx::Error`
/// (the server's message quotes the value it rejected), a URL parameter's
/// value or a server notice. `main.rs` installs these at startup.
pub fn log_cap_list() -> Vec<(&'static str, LevelFilter)> {
    stridelabs_postgres::LOG_CAPS.to_vec()
}

/// [`log_cap_list`] as the caps `init_logging_with` takes.
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
