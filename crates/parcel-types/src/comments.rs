//! Comments: feedback pinned to a spot on a parcel's page.
//!
//! Comments are numbered per parcel (`#1`, `#2`, …) so a person or an agent
//! can say "resolve 3 and 5". A top-level comment is a thread: it may carry
//! an anchor and be resolved; replies may do neither.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::paths;

/// Longest comment or reply body, in characters.
pub const MAX_COMMENT_CHARS: usize = 4000;
/// Longest anchor selector, in characters.
pub const MAX_SELECTOR_CHARS: usize = 1000;
/// Longest anchor text snippet, in characters.
pub const MAX_ANCHOR_TEXT_CHARS: usize = 500;

/// Where on the page a comment points, as recorded by the viewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct CommentAnchor {
    /// Published path of the page the comment was made on (`index.html`).
    pub path: String,
    /// CSS selector of the element that was clicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// The element's visible text when the comment was made, used to find
    /// it again after the page changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Click position inside the element's box, as a fraction of its width (0–1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// Click position inside the element's box, as a fraction of its height (0–1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

impl CommentAnchor {
    /// Check the anchor's shape. The server stores it as given, so this is
    /// the whole of its validation.
    pub fn validate(&self) -> Result<(), String> {
        paths::validate_path(&self.path).map_err(|e| format!("anchor.path: {e}"))?;
        if let Some(s) = &self.selector {
            if s.trim().is_empty() || s.chars().count() > MAX_SELECTOR_CHARS {
                return Err(format!(
                    "anchor.selector must be 1–{MAX_SELECTOR_CHARS} characters"
                ));
            }
        }
        for (name, value) in [("selector", &self.selector), ("text", &self.text)] {
            if value
                .as_deref()
                .is_some_and(|v| v.chars().any(char::is_control))
            {
                return Err(format!("anchor.{name} must not contain control characters"));
            }
        }
        if self
            .text
            .as_ref()
            .is_some_and(|t| t.chars().count() > MAX_ANCHOR_TEXT_CHARS)
        {
            return Err(format!(
                "anchor.text must be at most {MAX_ANCHOR_TEXT_CHARS} characters"
            ));
        }
        for (name, value) in [("x", self.x), ("y", self.y)] {
            if value.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
                return Err(format!("anchor.{name} must be between 0 and 1"));
            }
        }
        Ok(())
    }
}

/// Validate a comment or reply body; returns it trimmed. Line breaks and
/// tabs are fine; other control characters are refused, since bodies are
/// printed raw in terminals (`parcel comments list`).
pub fn validate_body(body: &str) -> Result<&str, String> {
    let body = body.trim();
    if body.is_empty() || body.chars().count() > MAX_COMMENT_CHARS {
        return Err(format!("comment must be 1–{MAX_COMMENT_CHARS} characters"));
    }
    if body
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("comment must not contain control characters".into());
    }
    Ok(body)
}

/// Where a comment was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum CommentVia {
    /// In the web viewer.
    Web,
    /// With a `pcl_` token: the CLI or an agent.
    Cli,
}

impl CommentVia {
    pub fn as_str(self) -> &'static str {
        match self {
            CommentVia::Web => "web",
            CommentVia::Cli => "cli",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "web" => Some(CommentVia::Web),
            "cli" => Some(CommentVia::Cli),
            _ => None,
        }
    }
}

/// Who wrote a comment, from the caller's point of view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CommentAuthor {
    /// Display name (the account's name, else its email).
    pub name: String,
    /// The author owns the parcel.
    pub is_owner: bool,
    /// The author is the caller.
    pub is_you: bool,
}

/// A reply in a comment thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CommentReply {
    pub number: i32,
    pub author: CommentAuthor,
    pub body: String,
    pub via: CommentVia,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// A comment thread: the top-level comment and its replies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Comment {
    pub number: i32,
    /// The version the commenter was looking at.
    pub version: i32,
    pub author: CommentAuthor,
    pub body: String,
    /// Absent for a general comment about the whole parcel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommentAnchor>,
    pub via: CommentVia,
    pub resolved: bool,
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub resolved_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    /// Oldest first.
    #[serde(default)]
    pub replies: Vec<CommentReply>,
}

/// `GET /api/v1/parcels/{id}/comments`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CommentList {
    /// Oldest first.
    pub comments: Vec<Comment>,
    /// Unresolved threads on the parcel, whatever the filter.
    pub open: i64,
}

/// Which threads to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum CommentStatus {
    #[default]
    Open,
    Resolved,
    All,
}

/// `POST /api/v1/parcels/{id}/comments` — a new thread, or a reply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct NewComment {
    pub body: String,
    /// The version being commented on. Defaults to the current one; ignored
    /// for replies, which take their thread's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// Where on the page. Not allowed on replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommentAnchor>,
    /// Reply to this thread instead of starting a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i32>,
}

/// `PATCH /api/v1/parcels/{id}/comments/{number}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct UpdateComment {
    /// New text (the author only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Resolve or reopen a thread (the parcel's owner or the thread's author).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
}

/// `GET /api/v1/parcels/{id}/activity` — what the viewer polls to notice
/// new versions and comment changes without refetching everything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelActivity {
    pub current_version: i32,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    /// Unresolved comment threads.
    pub open_comments: i64,
    /// Changes whenever any comment is added, edited, resolved or deleted.
    pub comments_revision: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> CommentAnchor {
        CommentAnchor {
            path: "index.html".into(),
            selector: Some("main > h1".into()),
            text: Some("Hello".into()),
            x: Some(0.5),
            y: Some(0.0),
        }
    }

    #[test]
    fn anchors_validate() {
        assert!(anchor().validate().is_ok());
        let mut bad = anchor();
        bad.path = "../x".into();
        assert!(bad.validate().is_err());
        let mut bad = anchor();
        bad.x = Some(1.5);
        assert!(bad.validate().unwrap_err().contains("anchor.x"));
        let mut bad = anchor();
        bad.selector = Some(" ".into());
        assert!(bad.validate().is_err());
        let mut bad = anchor();
        bad.text = Some("x\u{1b}[2J".into());
        assert!(bad.validate().unwrap_err().contains("anchor.text"));
    }

    #[test]
    fn bodies_are_trimmed_and_bounded() {
        assert_eq!(validate_body("  hi \n").unwrap(), "hi");
        assert!(validate_body("   ").is_err());
        assert!(validate_body(&"x".repeat(MAX_COMMENT_CHARS + 1)).is_err());
        assert_eq!(validate_body("a\r\n\tb").unwrap(), "a\r\n\tb");
        assert!(validate_body("evil \u{1b}]52;c;AAAA\u{7}").is_err());
        assert!(validate_body("c1 \u{9b}2J").is_err());
    }

    #[test]
    fn unknown_anchor_fields_are_rejected() {
        let err = serde_json::from_str::<CommentAnchor>(r#"{"path":"a.html","nope":1}"#);
        assert!(err.is_err());
    }
}
