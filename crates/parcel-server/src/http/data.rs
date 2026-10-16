//! `/api/v1/parcels/{id}/data` — a parcel's JSON data, for its owner's CLI
//! and agents. The page itself reaches the same data on the content origin
//! (`content::data`); both go through the helpers here.
//!
//! Anyone who can view a parcel can read its data; only the owner can
//! change it.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use parcel_types::{
    validate_data_name, DataCollection, DataItem, DataItemList, DataSummary, ErrorBody,
};
use serde::Deserialize;
use serde_json::Value;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AuthUser, MaybeAuthUser};
use crate::db::data::{self, DataRow, Write, WriteRefused};
use crate::error::{not_found, too_large, AppError, AppResult};
use crate::http::views::{owned, readable};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_data_summary))
        .routes(routes!(list_data_items, delete_data_collection))
        .routes(routes!(
            get_data_item,
            put_data_item,
            patch_data_item,
            delete_data_item
        ))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Page size (1–1000, default 100).
    pub limit: Option<i64>,
    /// Return keys after this one (the previous page's `next_after`).
    pub after: Option<String>,
}

fn item(row: DataRow) -> DataItem {
    DataItem {
        collection: row.collection,
        key: row.key,
        value: row.value.0,
        size_bytes: row.size_bytes,
        updated_at: row.updated_at,
    }
}

pub(crate) fn check_names(collection: &str, key: Option<&str>) -> Result<(), AppError> {
    validate_data_name("collection", collection).map_err(AppError::BadRequest)?;
    if let Some(key) = key {
        validate_data_name("key", key).map_err(AppError::BadRequest)?;
    }
    Ok(())
}

pub(crate) async fn summary(
    state: &AppState,
    parcel_id: &str,
    writable: bool,
) -> AppResult<DataSummary> {
    let (revision, collections) = data::summary(&state.db, parcel_id).await?;
    Ok(DataSummary {
        revision,
        usage_bytes: collections.iter().map(|c| c.usage_bytes).sum(),
        limit_bytes: state.config.limits.max_data_bytes,
        writable,
        collections: collections
            .into_iter()
            .map(|c| DataCollection {
                name: c.name,
                count: c.count,
                usage_bytes: c.usage_bytes,
                updated_at: c.updated_at,
            })
            .collect(),
    })
}

pub(crate) async fn list_items(
    state: &AppState,
    parcel_id: &str,
    collection: &str,
    q: ListQuery,
) -> AppResult<DataItemList> {
    check_names(collection, None)?;
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    // One extra row says whether there is a next page.
    let mut rows = data::list(
        &state.db,
        parcel_id,
        collection,
        q.after.as_deref(),
        limit + 1,
    )
    .await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next_after = more.then(|| rows.last().map(|r| r.key.clone())).flatten();
    Ok(DataItemList {
        items: rows.into_iter().map(item).collect(),
        next_after,
    })
}

pub(crate) async fn get_item(
    state: &AppState,
    parcel_id: &str,
    collection: &str,
    key: &str,
) -> AppResult<DataItem> {
    check_names(collection, Some(key))?;
    data::get(&state.db, parcel_id, collection, key)
        .await?
        .map(item)
        .ok_or_else(|| AppError::NotFound(format!("no item {collection}/{key}")))
}

/// Parse a request body as one JSON value. Bodies far over the value limit
/// are refused before parsing; the exact limit applies to the compact form.
pub(crate) fn parse_value(state: &AppState, body: &Bytes) -> Result<Value, AppError> {
    let max = state.config.limits.max_data_value_bytes;
    let slack = usize::try_from(max)
        .unwrap_or(usize::MAX)
        .saturating_mul(2)
        .saturating_add(4096);
    if body.len() > slack {
        return Err(too_large(format!(
            "a data value can be at most {max} bytes"
        )));
    }
    serde_json::from_slice(body)
        .map_err(|e| AppError::BadRequest(format!("body must be one JSON value: {e}")))
}

pub(crate) async fn write_item(
    state: &AppState,
    parcel_id: &str,
    collection: &str,
    key: &str,
    write: Write<'_>,
) -> AppResult<DataItem> {
    check_names(collection, Some(key))?;
    let limits = &state.config.limits;
    match data::write(
        &state.db,
        parcel_id,
        collection,
        key,
        write,
        limits.max_data_value_bytes,
        limits.max_data_bytes,
    )
    .await?
    {
        Ok(row) => Ok(item(row)),
        Err(WriteRefused::ParcelGone) => Err(not_found()),
        Err(WriteRefused::ValueTooLarge { size }) => Err(too_large(format!(
            "the value is {size} bytes; a data value can be at most {} bytes",
            limits.max_data_value_bytes
        ))),
        Err(WriteRefused::OverLimit { usage }) => Err(too_large(format!(
            "this parcel's data would exceed its {} byte limit ({usage} bytes in use)",
            limits.max_data_bytes
        ))),
    }
}

pub(crate) fn merge_body(value: &Value) -> Result<(), AppError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "a merge patch must be a JSON object (use PUT to replace a value)".into(),
        ))
    }
}

pub(crate) async fn delete_items(
    state: &AppState,
    parcel_id: &str,
    collection: &str,
    key: Option<&str>,
) -> AppResult<()> {
    check_names(collection, key)?;
    data::delete(&state.db, parcel_id, collection, key).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/data",
    tag = "data",
    description = "The parcel's data collections and how much of its data limit they use. \
        Readable wherever the parcel is; `writable` says whether the caller may change it.",
    params(("id" = String, Path, description = "Parcel id")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = DataSummary),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_data_summary(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<DataSummary>> {
    let (_, is_owner) = readable(&state, &id, caller.as_ref()).await?;
    Ok(Json(summary(&state, &id, is_owner).await?))
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/data/{collection}",
    tag = "data",
    description = "A page of a collection's items, by key.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
        ListQuery,
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = DataItemList),
        (status = 400, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn list_data_items(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path((id, collection)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<DataItemList>> {
    readable(&state, &id, caller.as_ref()).await?;
    Ok(Json(list_items(&state, &id, &collection, q).await?))
}

#[utoipa::path(
    delete,
    path = "/parcels/{id}/data/{collection}",
    tag = "data",
    description = "Delete every item in a collection (the owner only).",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Deleted (or already empty)"),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn delete_data_collection(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, collection)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    owned(&state, &id, &caller).await?;
    delete_items(&state, &id, &collection, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/data/{collection}/{key}",
    tag = "data",
    description = "One item.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
        ("key" = String, Path, description = "Item key"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = DataItem),
        (status = 400, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_data_item(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path((id, collection, key)): Path<(String, String, String)>,
) -> AppResult<Json<DataItem>> {
    readable(&state, &id, caller.as_ref()).await?;
    Ok(Json(get_item(&state, &id, &collection, &key).await?))
}

#[utoipa::path(
    put,
    path = "/parcels/{id}/data/{collection}/{key}",
    tag = "data",
    description = "Create or replace an item. The body is the value: any JSON.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
        ("key" = String, Path, description = "Item key"),
    ),
    request_body(content = Value, content_type = "application/json"),
    security(("bearer" = [])),
    responses(
        (status = 200, body = DataItem),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 413, body = ErrorBody, description = "The value or the parcel's data is over its limit"),
    )
)]
async fn put_data_item(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, collection, key)): Path<(String, String, String)>,
    body: Bytes,
) -> AppResult<Json<DataItem>> {
    owned(&state, &id, &caller).await?;
    let value = parse_value(&state, &body)?;
    Ok(Json(
        write_item(&state, &id, &collection, &key, Write::Replace(&value)).await?,
    ))
}

#[utoipa::path(
    patch,
    path = "/parcels/{id}/data/{collection}/{key}",
    tag = "data",
    description = "Merge a JSON object into an item (RFC 7386 merge patch: `null` removes a \
        field), creating it if it doesn't exist.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
        ("key" = String, Path, description = "Item key"),
    ),
    request_body(content = Value, content_type = "application/merge-patch+json"),
    security(("bearer" = [])),
    responses(
        (status = 200, body = DataItem),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 413, body = ErrorBody, description = "The value or the parcel's data is over its limit"),
    )
)]
async fn patch_data_item(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, collection, key)): Path<(String, String, String)>,
    body: Bytes,
) -> AppResult<Json<DataItem>> {
    owned(&state, &id, &caller).await?;
    let patch = parse_value(&state, &body)?;
    merge_body(&patch)?;
    Ok(Json(
        write_item(&state, &id, &collection, &key, Write::Merge(&patch)).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/parcels/{id}/data/{collection}/{key}",
    tag = "data",
    description = "Delete one item (the owner only).",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("collection" = String, Path, description = "Collection name"),
        ("key" = String, Path, description = "Item key"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Deleted (or already absent)"),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn delete_data_item(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, collection, key)): Path<(String, String, String)>,
) -> AppResult<StatusCode> {
    owned(&state, &id, &caller).await?;
    delete_items(&state, &id, &collection, Some(&key)).await?;
    Ok(StatusCode::NO_CONTENT)
}
