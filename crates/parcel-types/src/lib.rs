//! Wire types and validation rules shared by the parcel server, the `parcel`
//! CLI and the test suites.
//!
//! Everything a client and the server must agree on byte-for-byte lives here:
//! the JSON request/response shapes ([`api`], [`comments`], [`data`]), which published paths are legal
//! ([`paths`]), which file types a parcel may contain and how each is served
//! ([`media`]), and what a parcel id looks like ([`ids`]). Keeping one copy is
//! what lets the CLI reject a bad publish before uploading 60 MB of it.

pub mod api;
pub mod comments;
pub mod data;
pub mod ids;
pub mod media;
pub mod paths;

pub use api::*;
pub use comments::*;
pub use data::*;
