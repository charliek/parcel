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
pub mod publish;
pub mod state;
pub mod storage;

pub use http::openapi;
