//! One more attempt for a transaction that failed with a deadlock (40P01)
//! or a serialization failure (40001).
//!
//! Insurance rather than a fix: every multi-lock write takes the owner's
//! advisory lock before any row lock, so no in-revision cycle is known. But
//! a future lock change, or a rollover pairing two revisions that lock in
//! different orders, could make Postgres pick a victim; the victim's work
//! was rolled back whole, so running it again from the start is safe.
//!
//! The rules for an `op` handed to [`retry_once`]: it begins, works and
//! commits its own transaction, and nothing fallible runs after `commit()`
//! (an error there would rerun a transaction that already committed).
//! Whatever must happen once per committed change (the post-commit indexing
//! spawn) happens after the commit and can't fail. Writes outside the
//! database (blobs) happen before `op`, never inside it.

use std::future::Future;
use std::time::Duration;

use stridelabs_http::AppError;

use super::error::DbError;

/// Run `op`; if it fails with an error `retryable` gives a SQLSTATE for,
/// wait 10–50 ms and run it once more. Exactly two attempts at most: a
/// second failure, of any kind, is returned as it is.
///
/// `op` owns its transaction, so by the time its error reaches here the
/// transaction has been dropped and the retry starts from nothing: it
/// re-reads everything under fresh locks. Nothing is held across the
/// pause: a 40P01 or 40001 is an ERROR, which aborts the transaction on the
/// server and releases its locks there and then, before any ROLLBACK
/// arrives. The drop only queues that ROLLBACK; sqlx sends it when it
/// cleans the connection up, before anyone reuses it.
pub async fn retry_once<T, E, F, Fut>(
    operation: &'static str,
    mut op: F,
    retryable: impl Fn(&E) -> Option<&'static str>,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    match op().await {
        Err(e) => match retryable(&e) {
            Some(sqlstate) => {
                pause(operation, sqlstate).await;
                op().await
            }
            None => Err(e),
        },
        done => done,
    }
}

/// Log the retry and wait a jittered moment, so the transaction that won
/// gets to finish before the loser runs again. Value-free: the operation's
/// fixed name and the SQLSTATE, never the error's text.
pub async fn pause(operation: &'static str, sqlstate: &'static str) {
    tracing::warn!(
        operation,
        sqlstate,
        "retrying a transaction after a deadlock or serialization failure"
    );
    tokio::time::sleep(Duration::from_millis(rand::random_range(10..=50))).await;
}

/// The SQLSTATE of a retryable [`DbError`] anywhere in `e`'s chain.
pub fn retryable(e: &anyhow::Error) -> Option<&'static str> {
    // `DbError` owns its code; the two retryable ones are named here so the
    // log line can carry a `&'static str`.
    match DbError::find(e).filter(|d| d.is_retryable())?.code() {
        Some("40P01") => Some("40P01"),
        Some("40001") => Some("40001"),
        _ => None,
    }
}

/// [`retryable`] for an error already at the HTTP boundary: only an
/// internal error can carry a database failure.
pub fn retryable_app(e: &AppError) -> Option<&'static str> {
    match e {
        AppError::Internal(e) => retryable(e),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A test error: `Some(code)` is retryable.
    #[derive(Debug, PartialEq)]
    struct Failed(Option<&'static str>);

    fn code(e: &Failed) -> Option<&'static str> {
        e.0
    }

    /// Run `retry_once` over `outcomes` (one per attempt, in order) and
    /// return its result and how many attempts it made.
    async fn run(outcomes: &[Result<u32, Failed>]) -> (Result<u32, Failed>, usize) {
        let calls = Cell::new(0);
        let result = retry_once(
            "test",
            || {
                let n = calls.get();
                calls.set(n + 1);
                let outcome = match &outcomes[n] {
                    Ok(v) => Ok(*v),
                    Err(Failed(c)) => Err(Failed(*c)),
                };
                async move { outcome }
            },
            code,
        )
        .await;
        (result, calls.get())
    }

    #[tokio::test]
    async fn a_retryable_failure_runs_the_op_once_more() {
        assert_eq!(run(&[Err(Failed(Some("40P01"))), Ok(7)]).await, (Ok(7), 2));
    }

    #[tokio::test]
    async fn a_second_failure_is_returned_without_a_third_attempt() {
        let (result, calls) = run(&[
            Err(Failed(Some("40P01"))),
            Err(Failed(Some("40001"))),
            Ok(7),
        ])
        .await;
        assert_eq!((result, calls), (Err(Failed(Some("40001"))), 2));
    }

    #[tokio::test]
    async fn other_failures_are_not_retried() {
        assert_eq!(
            run(&[Err(Failed(None)), Ok(7)]).await,
            (Err(Failed(None)), 1)
        );
    }

    #[tokio::test]
    async fn success_is_never_retried() {
        assert_eq!(run(&[Ok(1), Ok(2)]).await, (Ok(1), 1));
    }

    fn database(code: &'static str) -> anyhow::Error {
        #[derive(Debug)]
        struct Raised(&'static str);
        impl std::fmt::Display for Raised {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("raised")
            }
        }
        impl std::error::Error for Raised {}
        impl sqlx::error::DatabaseError for Raised {
            fn message(&self) -> &str {
                "raised"
            }
            fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
                Some(self.0.into())
            }
            fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
                self
            }
            fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
                self
            }
            fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
                self
            }
            fn kind(&self) -> sqlx::error::ErrorKind {
                sqlx::error::ErrorKind::Other
            }
        }
        let raw = sqlx::Error::Database(Box::new(Raised(code)));
        anyhow::Error::new(DbError::new("locking parcel", &raw)).context("publishing")
    }

    /// The predicate finds the code under added context, through an
    /// `AppError::Internal`, and only for 40P01 and 40001.
    #[test]
    fn the_predicate_reads_the_sqlstate_through_the_chain() {
        assert_eq!(retryable(&database("40P01")), Some("40P01"));
        assert_eq!(retryable(&database("40001")), Some("40001"));
        for other in ["23514", "23505", "40002", "P0001"] {
            assert_eq!(retryable(&database(other)), None, "{other}");
        }
        assert_eq!(retryable(&anyhow::anyhow!("not a database error")), None);
        assert_eq!(
            retryable_app(&AppError::Internal(database("40P01"))),
            Some("40P01")
        );
        assert_eq!(retryable_app(&AppError::Conflict("stale".into())), None);
    }
}
