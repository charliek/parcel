//! `GET /api/v1/parcels/by-slug` and resolving parcel references.
//!
//! A clean URL `/@<handle>/<slug>` resolves through the handle (current or
//! one its user had before) to its user, then through the slug (current, or
//! a redirect a rename left) to a parcel, then through the caller's access.
//! Every failure along the way is the same 404 ([`BY_SLUG_NOT_FOUND`]): a
//! miss never says whether the handle, the slug or the parcel exists
//! (invariant 3).

use axum::extract::{Query, State};
use axum::Json;
use parcel_types::slugs::{normalize_handle, normalize_slug};
use parcel_types::{ErrorBody, ParcelAccess, ParcelDetail};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AuthUser, MaybeAuthUser};
use crate::db::parcels::ParcelRow;
use crate::db::slugs;
use crate::error::{AppError, AppResult};
use crate::http::parcels::detail_of;
use crate::http::views::readable;
use crate::state::AppState;

/// The one body every by-slug miss answers with.
pub const BY_SLUG_NOT_FOUND: &str = "no parcel at this address";

fn not_found() -> AppError {
    AppError::NotFound(BY_SLUG_NOT_FOUND.into())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_by_slug))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BySlugQuery {
    /// The owner's handle, or one they had before. Defaults to the caller's
    /// own; required when signed out.
    pub handle: Option<String>,
    /// The parcel's slug, or one it had before.
    pub slug: Option<String>,
}

#[utoipa::path(
    get,
    path = "/parcels/by-slug",
    tag = "parcels",
    description = "Resolve a clean URL (`/@<handle>/<slug>`) to the parcel it names: through \
        the handle (or one its user had before), the slug (or one the parcel had before), then \
        the same access rule as `GET /parcels/{id}`. `parcel.canonical_url` is the current \
        clean URL, which differs from the one asked for after a rename. Every miss is the same \
        404, whatever was missing.",
    params(BySlugQuery),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = ParcelDetail),
        (status = 400, body = ErrorBody, description = "No `slug`, or no `handle` while signed out"),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_by_slug(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Query(q): Query<BySlugQuery>,
) -> AppResult<Json<ParcelDetail>> {
    let slug = q
        .slug
        .ok_or_else(|| AppError::BadRequest("pass the parcel's `slug`".into()))?;
    let (row, access) = resolve_slug(&state, caller.as_ref(), q.handle.as_deref(), &slug).await?;
    Ok(Json(detail_of(&state, caller.as_ref(), row, access).await?))
}

/// The parcel a slug reference names and the caller's access to it (`GET
/// /parcels/by-slug`, and whatever else takes a [`parcel_types::ids::RefTarget::Slug`]),
/// resolved as the module docs say, access included, so every miss is the
/// one constant 404. `handle: None` means the caller's own (a 400 when
/// signed out).
pub(crate) async fn resolve_slug(
    state: &AppState,
    caller: Option<&AuthUser>,
    handle: Option<&str>,
    slug: &str,
) -> AppResult<(ParcelRow, ParcelAccess)> {
    let slug = normalize_slug(slug).map_err(|_| not_found())?;
    let id = match (handle, caller) {
        (Some(h), _) => {
            let handle = normalize_handle(h).map_err(|_| not_found())?;
            slugs::resolve(&state.db, &handle, &slug).await?
        }
        (None, Some(caller)) => slugs::find(&state.db, caller.user.id, &slug).await?,
        (None, None) => {
            return Err(AppError::BadRequest(
                "pass the owner's `handle` (it defaults to yours only when signed in)".into(),
            ))
        }
    };
    let id = id.ok_or_else(not_found)?;
    readable(state, &id, caller).await.map_err(|e| match e {
        AppError::NotFound(_) => not_found(),
        other => other,
    })
}
