//! Wire types and validation rules shared by the parcel server, the `parcel`
//! CLI and the test suites.
//!
//! Everything a client and the server must agree on byte-for-byte lives here:
//! the JSON request/response shapes ([`api`], [`comments`], [`data`], [`sharing`]), which published paths are legal
//! ([`paths`]), which file types a parcel may contain and how each is served
//! ([`media`]), what a parcel id, reference, handle and slug look like
//! ([`ids`], [`slugs`]), what a tag looks like ([`tags`]) and which version
//! sources are accepted ([`source`]). Keeping one copy is
//! what lets the CLI reject a bad publish before uploading 60 MB of it.

pub mod api;
pub mod comments;
pub mod data;
pub mod ids;
pub mod media;
pub mod paths;
pub mod sharing;
pub mod slugs;
pub mod source;
pub mod tags;

pub use api::*;
pub use comments::*;
pub use data::*;
pub use sharing::*;
pub use source::SourceInfo;
