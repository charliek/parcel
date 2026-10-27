//! Deadlock and serialization-failure retry (plan 002 §3.2), over the real
//! router and Postgres.
//!
//! Failures are injected deterministically: a trigger in the test's own
//! schema raises the SQLSTATE under test for its first *k* calls. A
//! sequence counts the calls, because a counter row would roll back with
//! the very transaction the trigger fails. One smoke test then makes
//! Postgres find a real deadlock. Id collisions are real too: a trigger
//! swaps in an existing parcel's id, and Postgres raises on the key.
//!
//! Every test runs on a current-thread runtime: `Captured` subscribes for
//! this thread only, and the retry and dispatch lines are counted.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::Response;
use common::*;
use parcel_server::error::AppError;
use parcel_server::publish::{self, VersionSource};
use parcel_server::search::wait_for_index_tasks;
use parcel_types::{NewVersionMetadata, PublishResult};
use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, PgPool};
use tracing::subscriber::DefaultGuard;

/// `db::retry`'s log line.
const RETRY_LINE: &str = "retrying a transaction after a deadlock or serialization failure";
/// `search::spawn_index`'s, one per committed version.
const DISPATCH_LINE: &str = "indexing a published version";

/// Everything logged on this thread until it drops.
struct Logs {
    captured: Captured,
    _guard: DefaultGuard,
}

impl Logs {
    fn lines(&self, needle: &str) -> Vec<String> {
        self.captured
            .text()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    }
}

fn capture() -> Logs {
    let captured = Captured::default();
    let _guard = captured.install();
    Logs { captured, _guard }
}

/// Make the first `k` rows `when` (`"BEFORE UPDATE ON parcels"`, …) fail
/// with SQLSTATE `code`; later ones pass. `body` replaces the plain raise
/// (the race test waits at a gate first).
async fn inject_with(app: &TestApp, when: &str, k: i64, body: &str) {
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE SEQUENCE inject_calls;
         CREATE FUNCTION inject_fail() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF nextval('inject_calls') <= {k} THEN {body}; END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER inject {when} FOR EACH ROW EXECUTE FUNCTION inject_fail();"
    )))
    .execute(&app.pool)
    .await
    .expect("arm the injected failure");
    // In this test's schema, so no other test sees it.
    let scoped: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         WHERE t.tgname = 'inject' AND c.relnamespace = current_schema()::regnamespace)",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(scoped);
}

async fn inject(app: &TestApp, when: &str, code: &str, k: i64) {
    let raise = format!("RAISE EXCEPTION 'injected' USING ERRCODE = '{code}'");
    inject_with(app, when, k, &raise).await;
}

/// How many times the trigger has run.
async fn calls(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM inject_calls")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// The writes that retry, each run over HTTP against [`setup`]'s parcel.
#[derive(Debug, Clone, Copy)]
enum Op {
    Create,
    SlugUpsert,
    NewVersion,
    Restore,
    Patch,
    TagRename,
    TagDelete,
}

impl Op {
    const ALL: [Op; 7] = [
        Op::Create,
        Op::SlugUpsert,
        Op::NewVersion,
        Op::Restore,
        Op::Patch,
        Op::TagRename,
        Op::TagDelete,
    ];

    /// Where the injected failure goes: the statement each one can't skip.
    fn trigger(self) -> &'static str {
        match self {
            Op::Create | Op::SlugUpsert | Op::NewVersion | Op::Restore => {
                "BEFORE INSERT ON parcel_versions"
            }
            Op::Patch | Op::TagRename | Op::TagDelete => "BEFORE UPDATE ON parcels",
        }
    }

    /// Blobs it writes (before its transaction), and versions it commits
    /// (each dispatching one indexing).
    fn blobs_and_versions(self) -> (usize, usize) {
        match self {
            Op::Create | Op::SlugUpsert | Op::NewVersion => (1, 1),
            Op::Restore => (0, 1),
            Op::Patch | Op::TagRename | Op::TagDelete => (0, 0),
        }
    }

    async fn run(self, app: &TestApp, s: &Setup) -> Response {
        let token = Some(s.who.token.as_str());
        let id = &s.id;
        match self {
            Op::Create => {
                let form = Form::new().file("index.html", "<h1>Created</h1>");
                app.post_multipart("/api/v1/parcels", token, &form).await
            }
            Op::SlugUpsert => {
                let form = Form::new()
                    .metadata(json!({"slug": "plan"}))
                    .file("index.html", "<h1>Upserted</h1>");
                app.post_multipart("/api/v1/parcels", token, &form).await
            }
            Op::NewVersion => {
                let form = Form::new().file("index.html", "<h1>Two</h1>");
                app.post_multipart(&format!("/api/v1/parcels/{id}/versions"), token, &form)
                    .await
            }
            Op::Restore => {
                let uri = format!("/api/v1/parcels/{id}/versions/1/restore");
                app.post_json(&uri, token, &json!({})).await
            }
            Op::Patch => patch(app, &s.who, id, json!({"title": "Renamed"})).await,
            Op::TagRename => {
                let body = json!({"from": "kind:plan", "to": "kind:report"});
                app.post_json("/api/v1/tags/rename", token, &body).await
            }
            Op::TagDelete => {
                let body = json!({"tag": "kind:plan"});
                app.post_json("/api/v1/tags/delete", token, &body).await
            }
        }
    }

    fn success(self) -> StatusCode {
        match self {
            Op::Create | Op::NewVersion | Op::Restore => StatusCode::CREATED,
            Op::SlugUpsert | Op::Patch | Op::TagRename | Op::TagDelete => StatusCode::OK,
        }
    }

    /// The state after it committed once, from the state before.
    fn applied(self, before: &Snapshot) -> Snapshot {
        let mut after = before.clone();
        match self {
            Op::Create => {
                after.parcels += 1;
                after.versions += 1;
            }
            Op::SlugUpsert | Op::NewVersion | Op::Restore => {
                after.versions += 1;
                after.current_version += 1;
            }
            Op::Patch => after.title = "Renamed".into(),
            Op::TagRename => after.tags = vec!["kind:report".into()],
            Op::TagDelete => after.tags = vec![],
        }
        after
    }
}

/// A user's parcel, slugged and tagged so every [`Op`] has something to
/// change, with its post-publish indexing finished.
struct Setup {
    who: Person,
    id: String,
    owner: uuid::Uuid,
}

async fn setup(app: &TestApp) -> Setup {
    let who = person("ada");
    let form = Form::new()
        .metadata(json!({"title": "Plan", "slug": "plan", "tags": ["kind:plan"]}))
        .file("index.html", "<h1>One</h1>");
    let published: PublishResult = expect_json(
        app.post_multipart("/api/v1/parcels", Some(&who.token), &form)
            .await,
        StatusCode::CREATED,
    )
    .await;
    wait_for_index_tasks(&app.state).await;
    let id = published.parcel.id;
    let owner = owner_id(app, &id).await;
    Setup { who, id, owner }
}

/// What the writes change: the owner's parcel and version counts, and the
/// setup parcel's version, title and tags.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    parcels: i64,
    versions: i64,
    current_version: i32,
    title: String,
    tags: Vec<String>,
}

async fn snapshot(app: &TestApp, s: &Setup) -> Snapshot {
    let (parcels, versions): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM parcels WHERE owner_id = $1),
                (SELECT count(*) FROM parcel_versions v JOIN parcels p ON p.id = v.parcel_id
                 WHERE p.owner_id = $1)",
    )
    .bind(s.owner)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let (current_version, title, tags): (i32, String, Vec<String>) =
        sqlx::query_as("SELECT current_version, title, tags FROM parcels WHERE id = $1")
            .bind(&s.id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    Snapshot {
        parcels,
        versions,
        current_version,
        title,
        tags,
    }
}

/// With `code` injected on the first attempt, `op` succeeds having
/// committed exactly once: one retry logged (value-free: the operation and
/// the SQLSTATE), no blob written again, one indexing dispatch per version.
async fn retried_once(op: Op) {
    for code in ["40P01", "40001"] {
        let app = TestApp::new().await;
        let s = setup(&app).await;
        let before = snapshot(&app, &s).await;
        inject(&app, op.trigger(), code, 1).await;
        let puts = app.blobs.puts();
        let logs = capture();

        expect_status(op.run(&app, &s).await, op.success()).await;

        assert_eq!(calls(&app).await, 2, "{op:?} {code}: one retry");
        assert_eq!(
            snapshot(&app, &s).await,
            op.applied(&before),
            "{op:?} {code}"
        );
        let retries = logs.lines(RETRY_LINE);
        assert_eq!(retries.len(), 1, "{op:?} {code}: {retries:?}");
        assert!(retries[0].contains(code), "{}", retries[0]);
        assert!(!retries[0].contains("injected"), "{}", retries[0]);
        let (blobs, versions) = op.blobs_and_versions();
        assert_eq!(app.blobs.puts() - puts, blobs, "{op:?} {code}: blob writes");
        assert_eq!(
            logs.lines(DISPATCH_LINE).len(),
            versions,
            "{op:?} {code}: indexing dispatches"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_create_retries_once() {
    retried_once(Op::Create).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_slug_upsert_retries_once() {
    retried_once(Op::SlugUpsert).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_new_version_retries_once() {
    retried_once(Op::NewVersion).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_restore_retries_once() {
    retried_once(Op::Restore).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_patch_retries_once() {
    retried_once(Op::Patch).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_tag_rename_retries_once() {
    retried_once(Op::TagRename).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_tag_delete_retries_once() {
    retried_once(Op::TagDelete).await;
}

/// Run `op` with the first `k` attempts failing with `code`, expecting the
/// generic 500, `attempts` trigger calls, `retries` retry lines and nothing
/// committed (no dispatch either).
async fn fails(op: Op, code: &str, k: i64, attempts: i64, retries: usize) {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let before = snapshot(&app, &s).await;
    inject(&app, op.trigger(), code, k).await;
    let logs = capture();

    let body: Value = expect_json(op.run(&app, &s).await, StatusCode::INTERNAL_SERVER_ERROR).await;
    assert_eq!(body["error"]["message"], "internal server error");

    assert_eq!(calls(&app).await, attempts, "{op:?} {code}");
    assert_eq!(snapshot(&app, &s).await, before, "{op:?} {code}");
    assert_eq!(logs.lines(RETRY_LINE).len(), retries, "{op:?} {code}");
    assert!(logs.lines(DISPATCH_LINE).is_empty(), "{op:?} {code}");
}

/// Two failures in a row are the caller's 500: there is no third attempt.
#[tokio::test(flavor = "current_thread")]
async fn a_second_failure_is_a_500_without_a_third_attempt() {
    for op in Op::ALL {
        fails(op, "40P01", 2, 2, 1).await;
    }
}

/// Only 40P01 and 40001 retry: a check violation fails at once.
#[tokio::test(flavor = "current_thread")]
async fn other_failures_are_not_retried() {
    for op in Op::ALL {
        fails(op, "23514", 1, 1, 0).await;
    }
}

/// Arm `create`'s two retries together. Every parcel insert records a hash
/// of the id it tried, and the ones numbered in `collide_on` collide for
/// real: the trigger swaps in the setup parcel's id, so Postgres itself
/// raises on the primary key (a drift between `parcels::PARCEL_PKEY` and
/// the schema fails these tests rather than passing them vacuously). The
/// first `deadlocks` version inserts fail with 40P01.
async fn inject_mixed(
    app: &TestApp,
    s: &Setup,
    collide_on: &[i64],
    deadlocks: i64,
) -> Fingerprints {
    let tried = Fingerprints::new(6);
    let collide = collide_on
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let version_calls = tried.sibling("versions");
    let mut sql = tried.create_sql();
    sql.push_str(&format!(
        "CREATE SEQUENCE {version_calls};
         CREATE FUNCTION mixed_parcels() RETURNS trigger LANGUAGE plpgsql AS $$
         DECLARE n bigint;
         BEGIN
             {record}
             IF n IN ({collide}) THEN NEW.id := '{taken}'; END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER mixed_parcels BEFORE INSERT ON parcels
             FOR EACH ROW EXECUTE FUNCTION mixed_parcels();
         CREATE FUNCTION mixed_versions() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF nextval('{version_calls}') <= {deadlocks} THEN
                 RAISE EXCEPTION 'injected' USING ERRCODE = '40P01';
             END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER mixed_versions BEFORE INSERT ON parcel_versions
             FOR EACH ROW EXECUTE FUNCTION mixed_versions();",
        record = tried.record("NEW.id"),
        taken = s.id,
    ));
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(&app.pool)
        .await
        .expect("arm the collisions and the deadlock");
    tried
}

/// A create's two retries keep their own budgets: a deadlock retries with
/// the same id (only a collision spends one), so a collision, a deadlock and
/// two more collisions try exactly three ids in four transactions and stop
/// at the third collision; with one collision the deadlock's retry
/// publishes under the id it was already trying.
#[tokio::test(flavor = "current_thread")]
async fn a_create_spends_ids_only_on_collisions() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let tried = inject_mixed(&app, &s, &[1, 3, 4], 1).await;
    let logs = capture();
    expect_status(
        Op::Create.run(&app, &s).await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 4, "four transactions: {ids:?}");
    assert_eq!(ids[1], ids[2], "the deadlock's retry keeps its id");
    assert_eq!(distinct(&ids), 3, "three ids: {ids:?}");
    assert_eq!(logs.lines(RETRY_LINE).len(), 1);
    drop(logs);

    let app = TestApp::new().await;
    let s = setup(&app).await;
    let tried = inject_mixed(&app, &s, &[1], 1).await;
    let logs = capture();
    let published: PublishResult =
        expect_json(Op::Create.run(&app, &s).await, StatusCode::CREATED).await;
    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[1], ids[2], "the deadlock's retry keeps its id");
    assert_eq!(
        text_hash(&app.pool, &published.parcel.id).await,
        ids[2],
        "published under the retried id"
    );
    assert_eq!(logs.lines(RETRY_LINE).len(), 1);
    assert_eq!(logs.lines(DISPATCH_LINE).len(), 1);
}

/// Two id collisions in a row cost two ids, not the publish: the third
/// insert succeeds under a third id, committing one parcel and one version,
/// with the blobs (written before the transaction) written once and no
/// deadlock retry spent.
#[tokio::test(flavor = "current_thread")]
async fn a_create_survives_two_id_collisions() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let before = snapshot(&app, &s).await;
    let tried = inject_mixed(&app, &s, &[1, 2], 0).await;
    let puts = app.blobs.puts();
    let logs = capture();

    let published: PublishResult =
        expect_json(Op::Create.run(&app, &s).await, StatusCode::CREATED).await;

    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 3, "three inserts: {ids:?}");
    assert_eq!(distinct(&ids), 3, "a fresh id per collision: {ids:?}");
    assert_eq!(
        text_hash(&app.pool, &published.parcel.id).await,
        ids[2],
        "published under the third id"
    );
    assert_eq!(snapshot(&app, &s).await, Op::Create.applied(&before));
    assert_eq!(app.blobs.puts() - puts, 1, "blob writes");
    assert!(logs.lines(RETRY_LINE).is_empty());
    assert_eq!(logs.lines(DISPATCH_LINE).len(), 1);
}

/// The third collision is the caller's 500: three distinct ids tried, no
/// fourth insert, nothing committed.
#[tokio::test(flavor = "current_thread")]
async fn a_third_id_collision_is_a_500() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let before = snapshot(&app, &s).await;
    let tried = inject_mixed(&app, &s, &[1, 2, 3], 0).await;
    let logs = capture();

    let body: Value = expect_json(
        Op::Create.run(&app, &s).await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(body["error"]["message"], "internal server error");

    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 3, "three inserts, no fourth: {ids:?}");
    assert_eq!(distinct(&ids), 3, "a fresh id per collision: {ids:?}");
    assert_eq!(snapshot(&app, &s).await, before, "nothing committed");
    assert!(logs.lines(RETRY_LINE).is_empty());
    assert!(logs.lines(DISPATCH_LINE).is_empty());
}

/// A deadlock before any collision: the retry tries the same id again,
/// collides for real, and only then spends a new one. Three inserts, the
/// first two under one id, and the publish lands under the third.
#[tokio::test(flavor = "current_thread")]
async fn a_create_collides_after_a_deadlock() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let before = snapshot(&app, &s).await;
    let tried = inject_mixed(&app, &s, &[2], 1).await;
    let puts = app.blobs.puts();
    let logs = capture();

    let published: PublishResult =
        expect_json(Op::Create.run(&app, &s).await, StatusCode::CREATED).await;

    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 3, "three inserts: {ids:?}");
    assert_eq!(ids[0], ids[1], "the deadlock's retry keeps its id");
    assert_ne!(ids[1], ids[2], "the collision spends it");
    assert_eq!(
        text_hash(&app.pool, &published.parcel.id).await,
        ids[2],
        "published under the third id"
    );
    assert_eq!(snapshot(&app, &s).await, Op::Create.applied(&before));
    assert_eq!(app.blobs.puts() - puts, 1, "blob writes");
    assert_eq!(logs.lines(RETRY_LINE).len(), 1);
    assert_eq!(logs.lines(DISPATCH_LINE).len(), 1);
}

/// A collision doesn't refill the deadlock budget: after a deadlock and a
/// collision, a second deadlock is the caller's 500, with the same three
/// inserts as above and nothing committed.
#[tokio::test(flavor = "current_thread")]
async fn a_collision_does_not_reset_the_deadlock_retry() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let before = snapshot(&app, &s).await;
    let tried = inject_mixed(&app, &s, &[2], 2).await;
    let logs = capture();

    let body: Value = expect_json(
        Op::Create.run(&app, &s).await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(body["error"]["message"], "internal server error");

    let ids = tried.hashes(&app.pool).await;
    assert_eq!(ids.len(), 3, "three inserts: {ids:?}");
    assert_eq!(ids[0], ids[1], "the deadlock's retry keeps its id");
    assert_ne!(ids[1], ids[2], "the collision spends it");
    assert_eq!(logs.lines(RETRY_LINE).len(), 1, "one deadlock retry");
    assert_eq!(snapshot(&app, &s).await, before, "nothing committed");
    assert!(logs.lines(DISPATCH_LINE).is_empty());
}

/// Poll until some backend waits on `pid` (a lock it holds), returning the
/// waiter. Bounded, so a broken setup fails instead of hanging.
async fn blocked_by(pool: &PgPool, pid: i32) -> i32 {
    for _ in 0..3000 {
        let waiter: Option<i32> = sqlx::query_scalar(
            "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
        )
        .bind(pid)
        .fetch_optional(pool)
        .await
        .unwrap();
        if let Some(waiter) = waiter {
            return waiter;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("nothing waited on backend {pid}");
}

async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(conn)
        .await
        .unwrap()
}

fn new_version(text: &str, base_version: Option<i32>) -> VersionSource {
    VersionSource::Upload {
        meta: NewVersionMetadata {
            base_version,
            ..NewVersionMetadata::default()
        },
        uploads: upload(text),
    }
}

/// A retry re-reads the parcel under the lock: when another version lands
/// between the attempts, a publish against the old `base_version` is a
/// 409, not a version on top of one it never saw.
///
/// The first attempt holds the owner's lock and the row lock, then waits at
/// a gate (an advisory lock the test holds) inside the injected trigger.
/// The rival's version bump queues on the row lock; the gate opens, the
/// attempt fails with 40P01 and aborts, the rival commits, and the
/// retry finds version 2.
#[tokio::test(flavor = "current_thread")]
async fn a_base_version_change_between_attempts_is_a_409() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    // Advisory locks are database-wide: a key of this test's own.
    let gate: i64 = rand_key();
    inject_with(
        &app,
        "BEFORE INSERT ON parcel_versions",
        1,
        &format!(
            "PERFORM pg_advisory_xact_lock({gate}); \
             RAISE EXCEPTION 'injected' USING ERRCODE = '40P01'"
        ),
    )
    .await;
    let mut gatekeeper = app.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(gate)
        .execute(&mut *gatekeeper)
        .await
        .unwrap();
    let gate_pid = backend_pid(&mut gatekeeper).await;
    let logs = capture();

    let publish = publish::add_version(
        &app.state,
        &s.id,
        s.owner,
        new_version("<h1>Two</h1>", Some(1)),
    );
    let (publisher_tx, publisher) = tokio::sync::oneshot::channel();
    let rival = async {
        let publisher: i32 = publisher.await.unwrap();
        let mut tx = app.pool.begin().await.unwrap();
        // Waits on the first attempt's row lock.
        sqlx::query("UPDATE parcels SET current_version = 2 WHERE id = $1")
            .bind(&s.id)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO parcel_versions (parcel_id, version, entry, file_count, total_bytes)
             SELECT parcel_id, 2, entry, file_count, total_bytes FROM parcel_versions
             WHERE parcel_id = $1 AND version = 1",
        )
        .bind(&s.id)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        publisher
    };
    let driver = async {
        // The first attempt is at the gate, holding both locks.
        let publisher = blocked_by(&app.pool, gate_pid).await;
        publisher_tx.send(publisher).unwrap();
        // The rival queues behind it on the row.
        blocked_by(&app.pool, publisher).await;
        gatekeeper.commit().await.unwrap();
    };
    let (published, _, ()) =
        within_a_minute("the race", async { tokio::join!(publish, rival, driver) }).await;

    match published {
        Err(AppError::Conflict(message)) => {
            assert!(message.contains("version 2, not 1"), "{message}")
        }
        other => panic!("expected a 409, got {other:?}"),
    }
    // The first attempt and the rival's insert; the retry stopped at the
    // version check, before inserting.
    assert_eq!(calls(&app).await, 2);
    assert_eq!(logs.lines(RETRY_LINE).len(), 1);
    assert!(logs.lines(DISPATCH_LINE).is_empty());
    let after = snapshot(&app, &s).await;
    assert_eq!((after.versions, after.current_version), (2, 2));
}

fn rand_key() -> i64 {
    // Away from `lock_owner_quota`'s keys only by chance, like any two
    // 64-bit hashes; a clash would only make a test wait.
    i64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap())
}

/// A real deadlock, found by Postgres. The test's session takes the
/// parcel's row lock first (the order no write uses), the publish takes the
/// owner's lock and waits on the row, then the test asks for the owner's
/// lock. The test's session raises its `deadlock_timeout` (a superuser
/// setting; the tests connect as one) well above the server's 1 s, so the
/// publish checks first and is the victim; its retry succeeds once the
/// test's session commits.
#[tokio::test(flavor = "current_thread")]
async fn a_real_deadlock_is_retried() {
    let app = TestApp::new().await;
    let s = setup(&app).await;
    let timeout: String = sqlx::query_scalar("SHOW deadlock_timeout")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(timeout, "1s", "the publish's deadlock_timeout");

    let mut rival = app.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL deadlock_timeout = '10s'")
        .execute(&mut *rival)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM parcels WHERE id = $1 FOR UPDATE")
        .bind(&s.id)
        .execute(&mut *rival)
        .await
        .unwrap();
    let rival_pid = backend_pid(&mut rival).await;
    let logs = capture();

    let publish = publish::add_version(
        &app.state,
        &s.id,
        s.owner,
        new_version("<h1>Two</h1>", None),
    );
    let cycle = async {
        // The publish holds the owner's lock and waits on the row.
        blocked_by(&app.pool, rival_pid).await;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 7101))")
            .bind(s.owner)
            .execute(&mut *rival)
            .await
            .unwrap();
        rival.commit().await.unwrap();
    };
    let (published, ()) =
        within_a_minute("the deadlock", async { tokio::join!(publish, cycle) }).await;

    assert_eq!(published.expect("the retry succeeds"), 2);
    let retries = logs.lines(RETRY_LINE);
    assert_eq!(retries.len(), 1, "{retries:?}");
    assert!(retries[0].contains("40P01"), "{}", retries[0]);
    assert_eq!(logs.lines(DISPATCH_LINE).len(), 1);
    assert_eq!(snapshot(&app, &s).await.versions, 2);
}
