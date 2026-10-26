//! Parcel data: keyed JSON items in named collections, per parcel.
//!
//! Every change locks the parcel row first (by bumping `parcels.data_rev`,
//! or `FOR UPDATE`), then item rows: one lock order, so concurrent changes
//! queue instead of deadlocking, and two writes can't both squeeze under the
//! parcel's data limit.

use anyhow::Result;
use parcel_types::DATA_ITEM_OVERHEAD;
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use time::OffsetDateTime;

use super::error::DbContext;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DataRow {
    pub collection: String,
    pub key: String,
    pub value: Json<Value>,
    pub size_bytes: i64,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CollectionRow {
    pub name: String,
    pub count: i64,
    pub usage_bytes: i64,
    pub updated_at: OffsetDateTime,
}

const ITEM_COLUMNS: &str = "collection, key, value, size_bytes, updated_at";

/// The parcel's data revision and its collections, by name.
pub async fn summary(pool: &PgPool, parcel_id: &str) -> Result<(i64, Vec<CollectionRow>)> {
    let revision: i64 = sqlx::query_scalar("SELECT data_rev FROM parcels WHERE id = $1")
        .bind(parcel_id)
        .fetch_optional(pool)
        .await
        .db_context("loading data revision")?
        .unwrap_or(0);
    let collections = sqlx::query_as::<_, CollectionRow>(
        "SELECT collection AS name, count(*) AS count,
                sum(size_bytes + $2)::bigint AS usage_bytes, max(updated_at) AS updated_at
         FROM parcel_data WHERE parcel_id = $1
         GROUP BY collection ORDER BY collection COLLATE \"C\"",
    )
    .bind(parcel_id)
    .bind(DATA_ITEM_OVERHEAD)
    .fetch_all(pool)
    .await
    .db_context("summarizing parcel data")?;
    Ok((revision, collections))
}

/// A page of a collection, by key (byte order), after `after`.
pub async fn list(
    pool: &PgPool,
    parcel_id: &str,
    collection: &str,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<DataRow>> {
    sqlx::query_as::<_, DataRow>(AssertSqlSafe(format!(
        "SELECT {ITEM_COLUMNS} FROM parcel_data
         WHERE parcel_id = $1 AND collection = $2
           AND ($3::text IS NULL OR key COLLATE \"C\" > $3)
         ORDER BY key COLLATE \"C\" LIMIT $4"
    )))
    .bind(parcel_id)
    .bind(collection)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
    .db_context("listing parcel data")
}

pub async fn get(
    pool: &PgPool,
    parcel_id: &str,
    collection: &str,
    key: &str,
) -> Result<Option<DataRow>> {
    sqlx::query_as::<_, DataRow>(AssertSqlSafe(format!(
        "SELECT {ITEM_COLUMNS} FROM parcel_data
         WHERE parcel_id = $1 AND collection = $2 AND key = $3"
    )))
    .bind(parcel_id)
    .bind(collection)
    .bind(key)
    .fetch_optional(pool)
    .await
    .db_context("loading parcel data item")
}

/// How a write changes the stored value.
pub enum Write<'a> {
    Replace(&'a Value),
    /// RFC 7386 JSON merge patch onto the current value.
    Merge(&'a Value),
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteRefused {
    ParcelGone,
    /// The parcel's data would exceed its limit; carries current usage.
    OverLimit {
        usage: i64,
    },
    /// The merged value is larger than one value may be.
    ValueTooLarge {
        size: i64,
    },
}

/// Replace or merge one item, enforcing the per-value and per-parcel limits.
pub async fn write(
    pool: &PgPool,
    parcel_id: &str,
    collection: &str,
    key: &str,
    write: Write<'_>,
    max_value_bytes: i64,
    max_parcel_bytes: i64,
) -> Result<Result<DataRow, WriteRefused>> {
    let mut tx = pool.begin().await.db_context("beginning data write")?;
    if !bump_rev(&mut tx, parcel_id).await? {
        return Ok(Err(WriteRefused::ParcelGone));
    }

    let value = match write {
        Write::Replace(value) => value.clone(),
        Write::Merge(patch) => {
            let current: Option<Json<Value>> = sqlx::query_scalar(
                "SELECT value FROM parcel_data
                 WHERE parcel_id = $1 AND collection = $2 AND key = $3",
            )
            .bind(parcel_id)
            .bind(collection)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            .db_context("loading data item to merge")?;
            let mut value = current.map(|j| j.0).unwrap_or(Value::Null);
            merge_patch(&mut value, patch);
            value
        }
    };
    let size = json_size(&value);
    if size > max_value_bytes {
        return Ok(Err(WriteRefused::ValueTooLarge { size }));
    }

    let others: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(size_bytes + $4), 0)::bigint FROM parcel_data
         WHERE parcel_id = $1 AND NOT (collection = $2 AND key = $3)",
    )
    .bind(parcel_id)
    .bind(collection)
    .bind(key)
    .bind(DATA_ITEM_OVERHEAD)
    .fetch_one(&mut *tx)
    .await
    .db_context("measuring parcel data")?;
    if others + size + DATA_ITEM_OVERHEAD > max_parcel_bytes {
        return Ok(Err(WriteRefused::OverLimit { usage: others }));
    }

    let row = sqlx::query_as::<_, DataRow>(AssertSqlSafe(format!(
        "INSERT INTO parcel_data (parcel_id, collection, key, value, size_bytes)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (parcel_id, collection, key) DO UPDATE
           SET value = EXCLUDED.value, size_bytes = EXCLUDED.size_bytes, updated_at = now()
         RETURNING {ITEM_COLUMNS}"
    )))
    .bind(parcel_id)
    .bind(collection)
    .bind(key)
    .bind(Json(&value))
    .bind(size)
    .fetch_one(&mut *tx)
    .await
    .db_context("writing data item")?;
    tx.commit().await.db_context("committing data write")?;
    Ok(Ok(row))
}

/// Delete one item (`Some(key)`) or a whole collection (`None`); returns how
/// many items went. The revision only moves when something did.
pub async fn delete(
    pool: &PgPool,
    parcel_id: &str,
    collection: &str,
    key: Option<&str>,
) -> Result<u64> {
    let mut tx = pool.begin().await.db_context("beginning data delete")?;
    // Parcel row first, as in `write`: one lock order, so a delete and a
    // write can't deadlock.
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM parcels WHERE id = $1 FOR UPDATE")
        .bind(parcel_id)
        .fetch_optional(&mut *tx)
        .await
        .db_context("locking parcel for data delete")?;
    if exists.is_none() {
        return Ok(0);
    }
    let deleted = sqlx::query(
        "DELETE FROM parcel_data
         WHERE parcel_id = $1 AND collection = $2 AND ($3::text IS NULL OR key = $3)",
    )
    .bind(parcel_id)
    .bind(collection)
    .bind(key)
    .execute(&mut *tx)
    .await
    .db_context("deleting parcel data")?
    .rows_affected();
    if deleted > 0 {
        bump_rev(&mut tx, parcel_id).await?;
    }
    tx.commit().await.db_context("committing data delete")?;
    Ok(deleted)
}

/// Bump the parcel's data revision, taking its row lock. False if the
/// parcel no longer exists.
async fn bump_rev(tx: &mut Transaction<'_, Postgres>, parcel_id: &str) -> Result<bool> {
    let bumped = sqlx::query("UPDATE parcels SET data_rev = data_rev + 1 WHERE id = $1")
        .bind(parcel_id)
        .execute(&mut **tx)
        .await
        .db_context("bumping data revision")?
        .rows_affected();
    Ok(bumped > 0)
}

/// Size of a value as compact JSON — what the limits count.
pub fn json_size(value: &Value) -> i64 {
    serde_json::to_vec(value).map_or(i64::MAX, |v| i64::try_from(v.len()).unwrap_or(i64::MAX))
}

/// RFC 7386: objects merge key by key (a `null` removes the key); anything
/// else replaces the target outright.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let Value::Object(target) = target else {
        unreachable!("target was just made an object")
    };
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::merge_patch;
    use serde_json::json;

    #[test]
    fn merge_patch_follows_rfc_7386() {
        let mut v = json!({"a": "b", "c": {"d": "e", "f": "g"}});
        merge_patch(&mut v, &json!({"a": "z", "c": {"f": null}}));
        assert_eq!(v, json!({"a": "z", "c": {"d": "e"}}));

        let mut v = json!([1, 2]);
        merge_patch(&mut v, &json!({"x": 1}));
        assert_eq!(v, json!({"x": 1}));

        let mut v = json!({"x": 1});
        merge_patch(&mut v, &json!([3]));
        assert_eq!(v, json!([3]));

        let mut v = serde_json::Value::Null;
        merge_patch(&mut v, &json!({"n": {"deep": null, "keep": 1}}));
        assert_eq!(v, json!({"n": {"keep": 1}}));
    }
}
