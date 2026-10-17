//! The OpenAPI document and the one place `/api/v1` routers are assembled.
//!
//! A route reaches the document only by living on an `OpenApiRouter` built
//! with `.routes(routes!(..))`; health, the content origin and the SPA are
//! plain axum routers and never pass through here, so their exclusion is
//! structural. `OpenApiRouter::route` is a silent pass-through that adds
//! nothing to the document, which is why `tests/openapi_shape.rs` pins the
//! exact (method, path) set — that test is the real guard.
//!
//! Versioning happens here, once, with `OpenApiRouter::nest` — which prefixes
//! both the runtime routes and the document's path keys. (Nesting an
//! already-split `axum::Router` would prefix only the runtime half.)

use axum::http::StatusCode;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{OpenApi, RefOr};
use utoipa::{Modify, OpenApi as OpenApiDerive, ToSchema};
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

#[derive(OpenApiDerive)]
#[openapi(
    info(
        title = "Parcel API",
        description = "Publish HTML pages (with optional supporting files) to private, \
            sandboxed URLs, manage their versions, visibility and sharing, and read them back. \
            Authenticate with a `pcl_` API token (the CLI and agents); the web app uses the \
            session cookie its server-side sign-in (`/auth/login`) sets. Health probes, \
            sign-in, the sandboxed content origin (`/v/{token}/…`) and the web app itself are \
            deliberately not part of this document.",
        contact(
            name = "Charlie Knudsen",
            url = "https://github.com/charliek/parcel",
            email = "charlie.knudsen@gmail.com"
        )
    ),
    servers((url = "/", description = "This server, relative to wherever it is deployed.")),
    modifiers(&BearerAuth),
    tags(
        (name = "account", description = "Server description and the signed-in user."),
        (name = "parcels", description = "Create, list, read, change and delete parcels."),
        (name = "versions", description = "A parcel's immutable versions and their files."),
        (name = "sharing", description = "Who a parcel is shared with, and what is shared with \
            you."),
        (name = "comments", description = "Feedback threads pinned to spots on a parcel's page."),
        (name = "data", description = "JSON data a parcel keeps between visits: readable by its \
            viewers, writable by its owner."),
        (name = "tokens", description = "API tokens for the CLI and agents."),
        (name = "device", description = "The CLI's device login flow."),
    )
)]
struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "A `pcl_` API token. (The web app sends its session cookie instead.)",
                    ))
                    .build(),
            ),
        );
    }
}

/// Multipart body of `POST /api/v1/parcels` (documentation only).
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct PublishUpload {
    /// JSON-encoded `CreateParcelMetadata`.
    metadata: Option<String>,
    /// One part per file; each part's filename is its published path.
    #[schema(value_type = Vec<String>, format = Binary)]
    file: Vec<Vec<u8>>,
}

/// Multipart body of `POST /api/v1/parcels/{id}/versions` (documentation only).
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct NewVersionUpload {
    /// JSON-encoded `NewVersionMetadata`.
    metadata: Option<String>,
    /// One part per file; each part's filename is its published path.
    #[schema(value_type = Vec<String>, format = Binary)]
    file: Vec<Vec<u8>>,
}

pub fn router() -> OpenApiRouter<AppState> {
    let v1 = OpenApiRouter::new()
        .merge(super::account::router())
        .merge(super::parcels::router())
        .merge(super::versions::router())
        .merge(super::sharing::router())
        .merge(super::comments::router())
        .merge(super::data::router())
        .merge(super::tokens::router())
        .merge(super::device::router());
    OpenApiRouter::with_openapi(ApiDoc::openapi()).nest("/api/v1", v1)
}

/// Axum spells a catch-all segment `{*path}`; OpenAPI has no such syntax.
/// The runtime route keeps the wildcard, the document gets `{path}`.
///
/// Also gives every response a description: OpenAPI 3.1 requires one, and
/// utoipa 6 omits an empty one rather than writing `""`, so a response
/// annotated without one gets its status's reason phrase ("Not Found").
pub fn finalize(mut doc: OpenApi) -> OpenApi {
    let paths = std::mem::take(&mut doc.paths.paths);
    doc.paths.paths = paths
        .into_iter()
        .map(|(path, item)| (path.replace("{*", "{"), item))
        .collect();
    for item in doc.paths.paths.values_mut() {
        let operations = [
            &mut item.get,
            &mut item.put,
            &mut item.post,
            &mut item.delete,
            &mut item.options,
            &mut item.head,
            &mut item.patch,
            &mut item.trace,
            &mut item.query,
        ]
        .into_iter()
        .flatten()
        .chain(item.additional_operations.values_mut());
        for operation in operations {
            for (status, response) in &mut operation.responses.responses {
                if let RefOr::T(response) = response {
                    if response.description.is_empty() {
                        response.description = reason_phrase(status).to_string();
                    }
                }
            }
        }
    }
    doc
}

fn reason_phrase(status: &str) -> &'static str {
    StatusCode::from_bytes(status.as_bytes())
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("Response")
}

/// The document alone: what `parcel-server openapi`, `/openapi.json` and the
/// tests read. Needs no `AppState`.
pub fn spec() -> OpenApi {
    finalize(router().split_for_parts().1)
}
