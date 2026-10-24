//! The app origin's HTTP surface: `/api/v1` (documented in OpenAPI), browser
//! sign-in (`/auth/*`), health, Swagger UI and the SPA. The content origin lives in `crate::content`.

pub mod account;
pub mod auth;
pub mod comments;
pub mod data;
pub mod device;
pub mod health;
pub mod openapi;
pub mod parcels;
pub mod server;
pub mod sharing;
pub mod spa;
pub mod tags;
pub mod tokens;
pub mod versions;
pub mod views;
