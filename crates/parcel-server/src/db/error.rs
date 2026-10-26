//! Value-free database errors: what a failed query becomes before it can
//! reach a log line or an error chain.
//!
//! PostgreSQL's messages quote what they reject (`invalid input syntax for
//! type uuid: "…"`, a rejected setting's value), and constraint violations
//! carry row values in DETAIL. sqlx's `Display` passes the message through,
//! and every unexpected error on an HTTP, MCP or content path is logged in
//! full by a library that renders the whole `anyhow` chain (invariant 5). So
//! a [`DbError`] keeps only what is safe to log and useful to decide on —
//! the call site's fixed context, [`error_summary`]'s SQLSTATE and fixed
//! description, the SQLSTATE itself and the violated constraint's name —
//! and drops the `sqlx::Error`. The constraint name is server-supplied text
//! (a schema identifier from a real violation, but `RAISE … USING
//! CONSTRAINT` can set anything), so it is kept for comparison and never
//! rendered. Unlike
//! `IndexError`, it doesn't even keep it for `Debug`.
//!
//! Built to lift into stridelabs-postgres: it knows nothing of parcel's
//! config, tables or types.

use std::fmt;

use stridelabs_postgres::error_summary;

/// A database error with its message, DETAIL and HINT dropped. `Display`
/// and `Debug` are both `"{context}: {summary}"`; there is no `source()`.
pub struct DbError {
    context: &'static str,
    summary: String,
    code: Option<String>,
    constraint: Option<String>,
}

impl DbError {
    /// Keep what is safe to log about `error`, under `context` (a fixed
    /// description of the operation: never a value).
    pub fn new(context: &'static str, error: &sqlx::Error) -> Self {
        let database = error.as_database_error();
        Self {
            context,
            summary: error_summary(error).to_string(),
            // Validated as `error_summary` does, so `code()` never hands out
            // anything but a SQLSTATE.
            code: database
                .and_then(|d| d.code())
                .filter(|c| is_sqlstate(c))
                .map(|c| c.into_owned()),
            constraint: database.and_then(|d| d.constraint()).map(str::to_owned),
        }
    }

    /// The first `DbError` in `error`'s chain, however much context was
    /// added on top of it.
    pub fn find(error: &anyhow::Error) -> Option<&DbError> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<DbError>())
    }

    /// The SQLSTATE, when the server sent a well-formed one.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// The name of the constraint the statement violated, when there is one.
    /// Server-supplied: compare it with known names, never log it.
    pub fn constraint(&self) -> Option<&str> {
        self.constraint.as_deref()
    }

    /// SQLSTATE 23505.
    pub fn is_unique_violation(&self) -> bool {
        self.code() == Some("23505")
    }

    /// A deadlock (40P01) or serialization failure (40001): the transaction
    /// was rolled back and running it again from the start may succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self.code(), Some("40P01" | "40001"))
    }
}

fn is_sqlstate(code: &str) -> bool {
    code.len() == 5
        && code
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.summary)
    }
}

/// The same as `Display`, so `?err` is as safe as `%err`.
impl fmt::Debug for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// No `source()`: the sqlx error is gone, so nothing below this one can be
/// printed by a chain walker.
impl std::error::Error for DbError {}

/// `.db_context("…")` for sqlx results: the value-free counterpart of
/// `anyhow::Context::context`, so repository functions keep returning
/// `anyhow::Result`.
pub trait DbContext<T> {
    /// Replace the sqlx error with a [`DbError`] under `context`.
    fn db_context(self, context: &'static str) -> anyhow::Result<T>;
}

impl<T> DbContext<T> for Result<T, sqlx::Error> {
    fn db_context(self, context: &'static str) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::Error::new(DbError::new(context, &e)))
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as _;

    use sqlx::error::{DatabaseError, ErrorKind};

    use super::*;

    const CANARY: &str = "db-error-canary";

    /// A server error as sqlx reports it, every text field a canary.
    #[derive(Debug)]
    struct Fake {
        code: &'static str,
        constraint: Option<&'static str>,
    }

    impl fmt::Display for Fake {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{CANARY} message, DETAIL {CANARY}, HINT {CANARY}")
        }
    }

    impl std::error::Error for Fake {}

    impl DatabaseError for Fake {
        fn message(&self) -> &str {
            CANARY
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
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
        fn constraint(&self) -> Option<&str> {
            self.constraint
        }
        fn table(&self) -> Option<&str> {
            Some(CANARY)
        }
        fn kind(&self) -> ErrorKind {
            match self.code {
                "23505" => ErrorKind::UniqueViolation,
                _ => ErrorKind::Other,
            }
        }
    }

    fn database(code: &'static str, constraint: Option<&'static str>) -> sqlx::Error {
        sqlx::Error::Database(Box::new(Fake { code, constraint }))
    }

    /// `raw` as a repository function hands it up: a `DbError` under
    /// "inserting parcel", with the caller's context on top.
    fn chained(raw: sqlx::Error) -> anyhow::Error {
        Err::<(), _>(raw)
            .db_context("inserting parcel")
            .map_err(|e| e.context("publishing"))
            .unwrap_err()
    }

    /// No rendering of the error, alone or in a chain with context on top,
    /// carries the server's text; the context and SQLSTATE survive.
    #[test]
    fn renderings_carry_no_message_text() {
        let raw = database("23505", Some("parcels_pkey"));
        assert!(
            raw.to_string().contains(CANARY),
            "the fake leaks as sqlx would"
        );
        let error = DbError::new("inserting parcel", &raw);
        assert_eq!(
            error.to_string(),
            "inserting parcel: database error SQLSTATE 23505 (unique violation)"
        );
        assert_eq!(format!("{error:?}"), error.to_string());
        assert!(error.source().is_none());

        let chained = chained(raw);
        for rendered in [
            format!("{chained}"),
            format!("{chained:#}"),
            format!("{chained:?}"),
            format!("{chained:#?}"),
        ] {
            assert!(!rendered.contains(CANARY), "{rendered}");
            assert!(
                !rendered.contains("error returned from database"),
                "{rendered}"
            );
        }
        assert!(format!("{chained:?}").contains("inserting parcel: database error SQLSTATE 23505"));
        assert_eq!(chained.chain().count(), 2, "the DbError ends the chain");
    }

    #[test]
    fn code_and_constraint_are_kept_and_found_through_context() {
        let chained = chained(database("23505", Some("parcels_slug_idx")));
        let found = DbError::find(&chained).expect("found under the context");
        assert!(found.to_string().starts_with("inserting parcel: "));
        assert_eq!(found.code(), Some("23505"));
        assert_eq!(found.constraint(), Some("parcels_slug_idx"));
        assert!(found.is_unique_violation());
        assert!(!found.is_retryable());

        assert!(DbError::find(&anyhow::anyhow!("not a database error")).is_none());
    }

    #[test]
    fn retryable_is_deadlock_or_serialization_failure() {
        for (code, retryable, unique) in [
            ("40P01", true, false),
            ("40001", true, false),
            ("40002", false, false),
            ("23505", false, true),
            ("23514", false, false),
            ("P0001", false, false),
        ] {
            let error = DbError::new("x", &database(code, None));
            assert_eq!(error.code(), Some(code));
            assert_eq!(error.is_retryable(), retryable, "{code}");
            assert_eq!(error.is_unique_violation(), unique, "{code}");
            assert_eq!(error.constraint(), None);
        }
    }

    /// A code that isn't a SQLSTATE is dropped rather than passed on.
    #[test]
    fn malformed_codes_are_dropped() {
        for code in [CANARY, "4000", "40p01", "40P011", ""] {
            let error = DbError::new("x", &database(code, None));
            assert_eq!(error.code(), None, "{code}");
            assert!(!error.is_retryable());
            assert_eq!(error.to_string(), "x: database error (no SQLSTATE)");
        }
    }

    /// Errors that aren't from the server keep a fixed class name and no
    /// code, whatever their payload says.
    #[test]
    fn client_errors_render_as_fixed_names() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, CANARY);
        for (raw, summary) in [
            (sqlx::Error::Io(io), "I/O error (ConnectionReset)"),
            (sqlx::Error::Protocol(CANARY.into()), "protocol error"),
            (sqlx::Error::RowNotFound, "no row returned"),
            (
                sqlx::Error::PoolTimedOut,
                "timed out waiting for a pool connection",
            ),
        ] {
            let error = DbError::new("loading parcel", &raw);
            assert_eq!(error.to_string(), format!("loading parcel: {summary}"));
            assert_eq!(error.code(), None);
            assert_eq!(error.constraint(), None);
        }
    }
}
