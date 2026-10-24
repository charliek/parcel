//! Where a version was published from: the repository, branch and commit,
//! the machine and the client. Shown to the parcel's owner only.
//!
//! A remote URL can carry a credential (`https://user:token@host/…`), so
//! `repo` is reduced to `host[:port]/path`: userinfo, query and fragment are
//! always dropped, as is a trailing `.git`. Accepted remotes are
//! `http(s)://[userinfo@]host[:port]/path`, `ssh://[user@]host[:port]/path`
//! and scp-like `user@host:path`. A local path or a `file://` URL is no
//! remote: `repo` is left out (paths can name people and machines). Anything
//! else, or a value still holding `@` or `:` after normalization (literal or
//! percent-encoded), a control character, or more than 200 characters, is
//! not accepted: the CLI drops the field ([`SourceInfo::sanitized`]) and the
//! server refuses the publish with a message naming only the field
//! ([`SourceInfo::validated`]), so the value is never stored, echoed or
//! logged.
//!
//! The two compose: what the CLI sends is what the server accepts, and the
//! server stores from it what it would have from the raw value. So the
//! sanitized `repo` keeps a scheme (`https://github.com/o/r`; `ssh://` for
//! the scp-like form) rather than being the stored `host/path`, which reads
//! the same as a relative local path (`gitserver/o/r`) and would be dropped
//! as one.

use serde::{Deserialize, Deserializer, Serialize};

/// Longest value of any source field, after normalization.
pub const MAX_SOURCE_CHARS: usize = 200;
/// Inputs longer than this aren't looked at.
const MAX_RAW_CHARS: usize = 2048;

/// A version's provenance. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SourceInfo {
    /// The repository's remote as `host[:port]/path` (no credentials, no `.git`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Commit id (usually abbreviated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The machine's hostname.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// What published it: `parcel-cli/<version>`, `mcp`, `web`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// Why a source was refused. The message names the field, never its value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("source must be an object of strings")]
    NotAnObject,
    #[error("source.{0} must be a string")]
    NotAString(&'static str),
    #[error("source.repo is not an accepted remote")]
    Repo,
    #[error("source.{0} must be at most {MAX_SOURCE_CHARS} printable characters")]
    Field(&'static str),
}

/// One field's verdict: kept (normalized), omitted, or refused.
type Checked = Result<Option<String>, SourceError>;

impl SourceInfo {
    /// The server's check: every field normalized (`repo` to its stored
    /// `host[:port]/path`), or the first one that isn't acceptable named in
    /// the error. `None` when nothing is left.
    pub fn validated(&self) -> Result<Option<SourceInfo>, SourceError> {
        self.map(|r| r.repo, std::convert::identity)
    }

    /// The client's version, to send: fields that aren't acceptable are
    /// dropped, and `repo` is a credential-free URL
    /// (`<scheme>://host[:port]/path`) that [`validated`](Self::validated)
    /// reduces to the same stored form as the raw value. `None` when nothing
    /// is left.
    pub fn sanitized(&self) -> Option<SourceInfo> {
        self.map(Remote::url, |checked| {
            Ok::<_, std::convert::Infallible>(checked.unwrap_or(None))
        })
        .unwrap_or_default()
    }

    fn map<E>(
        &self,
        repo_form: fn(Remote) -> String,
        f: impl Fn(Checked) -> Result<Option<String>, E>,
    ) -> Result<Option<SourceInfo>, E> {
        let repo = self.repo.as_deref().map_or(Ok(None), |raw| {
            remote(raw)
                .map(|r| r.map(repo_form))
                .map_err(|()| SourceError::Repo)
        });
        let out = SourceInfo {
            repo: f(repo)?,
            branch: f(check_plain("branch", &self.branch))?,
            commit: f(check_plain("commit", &self.commit))?,
            host: f(check_plain("host", &self.host))?,
            client: f(check_plain("client", &self.client))?,
        };
        Ok((out != SourceInfo::default()).then_some(out))
    }
}

fn check_plain(name: &'static str, raw: &Option<String>) -> Checked {
    let value = raw.as_deref().unwrap_or_default().trim();
    if value.is_empty() {
        Ok(None)
    } else if value.chars().any(char::is_control) || value.chars().count() > MAX_SOURCE_CHARS {
        Err(SourceError::Field(name))
    } else {
        Ok(Some(value.to_string()))
    }
}

/// A remote reduced to `host[:port]/path`; `Ok(None)` for a local path or
/// `file://` URL (no remote); [`SourceError::Repo`] for anything not accepted.
/// The tests' view of [`remote`].
#[cfg(test)]
fn normalize_repo(raw: &str) -> Result<Option<String>, SourceError> {
    remote(raw)
        .map(|r| r.map(|r| r.repo))
        .map_err(|()| SourceError::Repo)
}

/// An accepted remote: how it is reached and its stored form.
struct Remote {
    /// `http`, `https` or `ssh` (the scp-like form is ssh).
    scheme: &'static str,
    /// `host[:port]/path`.
    repo: String,
}

impl Remote {
    fn url(self) -> String {
        format!("{}://{}", self.scheme, self.repo)
    }
}

fn remote(raw: &str) -> Result<Option<Remote>, ()> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.chars().any(char::is_control) || s.chars().count() > MAX_RAW_CHARS {
        return Err(());
    }
    let remote = if let Some((scheme, rest)) = s.split_once("://") {
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "file" => return Ok(None),
            "http" => "http",
            "https" => "https",
            "ssh" => "ssh",
            _ => return Err(()),
        };
        Remote {
            scheme,
            repo: url_remote(rest)?,
        }
    } else {
        match scp_remote(s) {
            Some(repo) => Remote {
                scheme: "ssh",
                repo: repo?,
            },
            // No scheme and not `user@host:path`: a local path.
            None => return Ok(None),
        }
    };
    // `%40` and `%3A` are `@` and `:` to whatever decodes them: a credential
    // separator in disguise, anywhere before the (dropped) query. `%25` is
    // refused too, since a double encoding (`%2540`) hides one.
    let before_query = s.split(['?', '#']).next().unwrap_or_default();
    let lower = before_query.to_ascii_lowercase();
    if ["%40", "%3a", "%25"].iter().any(|e| lower.contains(e)) {
        return Err(());
    }
    Ok(Some(remote))
}

/// `[userinfo@]host[:port]/path[?query][#fragment]` (after the scheme).
fn url_remote(rest: &str) -> Result<String, ()> {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let path = tail.split(['?', '#']).next().unwrap_or("");
    finish(&host(host_port)?, path)
}

/// `user@host:path` (git's scp-like syntax): an `@` before the first `/`,
/// then a host and a `:`. `None` when the value isn't of this shape.
fn scp_remote(s: &str) -> Option<Result<String, ()>> {
    let head_end = s.find('/').unwrap_or(s.len());
    // The first `@`: one later on (in a password, or the path) then fails
    // the host check or `finish`, so it is refused rather than guessed at.
    let at = s[..head_end].find('@')?;
    let after = &s[at + 1..];
    let (host_part, path) = if after.starts_with('[') {
        let close = after.find(']')?;
        let (h, rest) = after.split_at(close + 1);
        (h, rest.strip_prefix(':')?)
    } else {
        let colon = after.find(':')?;
        if after[..colon].contains('/') {
            return None;
        }
        (&after[..colon], &after[colon + 1..])
    };
    // As with a URL, a query or fragment is dropped (it can carry a token).
    let path = path.split(['?', '#']).next().unwrap_or_default();
    Some(host(host_part).and_then(|h| finish(&h, path)))
}

/// A host with an optional numeric port, lowercased. IPv6 stays bracketed.
fn host(host_port: &str) -> Result<String, ()> {
    let (host, port) = if let Some(v6) = host_port.strip_prefix('[') {
        let (inner, rest) = v6.split_once(']').ok_or(())?;
        if inner.is_empty()
            || !inner
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        {
            return Err(());
        }
        let port = match rest {
            "" => None,
            _ => Some(rest.strip_prefix(':').ok_or(())?),
        };
        (format!("[{inner}]"), port)
    } else {
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        };
        if host.is_empty()
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(());
        }
        (host.to_string(), port)
    };
    let host = host.to_ascii_lowercase();
    match port {
        None => Ok(host),
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(format!("{host}:{p}"))
        }
        Some(_) => Err(()),
    }
}

fn finish(host: &str, path: &str) -> Result<String, ()> {
    let mut path = path.trim_matches('/');
    // Every trailing `.git`, so normalizing a normalized value changes nothing.
    while let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.trim_end_matches('/');
    }
    if path.is_empty() || path.contains(['@', ':']) {
        return Err(());
    }
    let repo = format!("{host}/{path}");
    if repo.chars().count() > MAX_SOURCE_CHARS {
        return Err(());
    }
    Ok(repo)
}

/// Deserialize an optional `source` in request metadata. Every error is
/// value-free: serde's own messages quote a mistyped value (`invalid type:
/// string "https://user:token@…"`), and this one may hold a credential.
/// Unknown fields are ignored (a newer client's extra field is dropped,
/// never stored).
pub(crate) fn deserialize_opt<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<SourceInfo>, D::Error> {
    use serde::de::Error;
    let value = serde_json::Value::deserialize(d)?;
    let object = match value {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::Object(object) => object,
        _ => return Err(D::Error::custom(SourceError::NotAnObject)),
    };
    let field = |name: &'static str| match object.get(name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(D::Error::custom(SourceError::NotAString(name))),
    };
    Ok(Some(SourceInfo {
        repo: field("repo")?,
        branch: field("branch")?,
        commit: field("commit")?,
        host: field("host")?,
        client: field("client")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Remotes and their stored form.
    const ACCEPTED: &[(&str, &str)] = &[
        (
            "https://github.com/charliek/parcel.git",
            "github.com/charliek/parcel",
        ),
        (
            "https://github.com/charliek/parcel",
            "github.com/charliek/parcel",
        ),
        ("https://user:tok@github.com/o/r", "github.com/o/r"),
        (
            "https://x-access-token:ghp_abc@github.com/o/r.git?x=1#f",
            "github.com/o/r",
        ),
        ("https://u:p@ss@github.com/o/r", "github.com/o/r"),
        ("https://host/o/r?q=%40%3A#%40", "host/o/r"),
        ("http://git.local:8080/o/r/", "git.local:8080/o/r"),
        ("HTTPS://GitHub.COM/Owner/Repo", "github.com/Owner/Repo"),
        ("ssh://git@github.com:22/o/r.git", "github.com:22/o/r"),
        ("ssh://github.com/o/r", "github.com/o/r"),
        ("ssh://gitserver/o/r", "gitserver/o/r"),
        ("git@github.com:o/r.git", "github.com/o/r"),
        ("git:pw@github.com:o/r", "github.com/o/r"),
        ("git@github.com:/abs/r.git", "github.com/abs/r"),
        (
            "git@github.com:o/r.git?token=CANARY#SECRET",
            "github.com/o/r",
        ),
        ("git@github.com:o/r#SECRET", "github.com/o/r"),
        ("git@gitserver:o/r", "gitserver/o/r"),
        ("https://[::1]:8080/o/r", "[::1]:8080/o/r"),
        ("git@[fe80::1]:o/r", "[fe80::1]/o/r"),
        ("  https://github.com/o/r  ", "github.com/o/r"),
        ("https://h/o/r.git.git", "h/o/r"),
        ("https://h/o/r.git/.git/", "h/o/r"),
    ];

    /// No remote: `repo` is left out.
    const LOCAL: &[&str] = &[
        "",
        "/home/ada/src/parcel",
        "../parcel",
        "~/src/parcel",
        "parcel",
        "file:///home/ada/parcel",
        "FILE://x",
        "C:\\Users\\ada\\parcel",
        "github.com:o/r",
        "user:token@host/repo",
        "/home/a%40b/repo",
        "gitserver/o/r",
    ];

    /// Not accepted: refused by the server, dropped by the client.
    fn refused() -> Vec<String> {
        let mut out: Vec<String> = [
            "git://github.com/o/r",
            "ftp://host/o/r",
            "https://host:password/o/r",
            "https://github.com",
            "https://github.com/",
            "https://github.com/.git",
            "https://host/o/r@x",
            "https://host/o:r",
            "https://ho st/o/r",
            "https://user@/o/r",
            "https://[::1/o/r",
            "https://[zz]/o/r",
            "git@host:",
            "git@host:o@r",
            "git@host:?x",
            "https://host/o/r\u{7}",
            "https://host/o/r\nx",
            // Percent-encoded `@` and `:`, any case, wherever they'd act.
            "https://host/o/x%40CANARY/r",
            "https://host/o/x%3aCANARY",
            "https://host/o/x%3ACANARY",
            "ssh://host/o%3ACANARY%40h/r",
            "git@host:o/CANARY%40r",
            "git@host:o/r%3a",
            "https://u%3ACANARY@host/o/r",
            "git%3ACANARY@host:o/r",
            "https://host/o/x%2540CANARY",
        ]
        .map(String::from)
        .into();
        out.push(format!("https://host/{}", "a".repeat(200)));
        out.push(format!("https://host/{}", "a".repeat(3000)));
        out
    }

    #[test]
    fn remotes_reduce_to_host_and_path() {
        for (raw, expected) in ACCEPTED {
            assert_eq!(
                normalize_repo(raw),
                Ok(Some(expected.to_string())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn local_paths_are_no_remote() {
        for raw in LOCAL {
            assert_eq!(normalize_repo(raw), Ok(None), "{raw:?}");
        }
    }

    #[test]
    fn other_values_are_refused() {
        for raw in refused() {
            assert_eq!(normalize_repo(&raw), Err(SourceError::Repo), "{raw:?}");
        }
    }

    fn source(repo: &str) -> SourceInfo {
        SourceInfo {
            repo: Some(repo.into()),
            branch: Some(" main ".into()),
            commit: Some("abc123".into()),
            host: Some("laptop".into()),
            client: Some("parcel-cli/0.2.0".into()),
        }
    }

    #[test]
    fn the_server_refuses_naming_only_the_field() {
        let ok = source("https://ada:canary-7731@github.com/o/r.git")
            .validated()
            .unwrap()
            .unwrap();
        assert_eq!(ok.repo.as_deref(), Some("github.com/o/r"));
        assert_eq!(ok.branch.as_deref(), Some("main"));

        let err = source("https://github.com:canary-7731/o/r")
            .validated()
            .unwrap_err();
        assert_eq!(err, SourceError::Repo);
        assert!(!err.to_string().contains("canary"));

        let mut bad_branch = source("/local/path");
        bad_branch.branch = Some(format!("x{}", "\u{1b}"));
        assert_eq!(bad_branch.validated(), Err(SourceError::Field("branch")));
        bad_branch.branch = Some("b".repeat(201));
        assert_eq!(
            bad_branch.validated().unwrap_err().to_string(),
            "source.branch must be at most 200 printable characters"
        );

        let local = SourceInfo {
            repo: Some("/home/ada/parcel".into()),
            ..SourceInfo::default()
        };
        assert_eq!(local.validated(), Ok(None), "nothing left");
        assert_eq!(SourceInfo::default().validated(), Ok(None));
    }

    #[test]
    fn the_client_drops_what_the_server_would_refuse() {
        let mut s = source("git://host/o/r");
        s.host = Some("h\u{0}".into());
        let kept = s.sanitized().unwrap();
        assert_eq!(kept.repo, None);
        assert_eq!(kept.host, None);
        assert_eq!(kept.commit.as_deref(), Some("abc123"));
        assert_eq!(
            SourceInfo {
                repo: Some("git://x/y".into()),
                ..SourceInfo::default()
            }
            .sanitized(),
            None
        );
    }

    #[test]
    fn the_client_sends_a_credential_free_url() {
        for (raw, sent) in [
            (
                "https://user:tok@github.com/o/r.git?x#y",
                "https://github.com/o/r",
            ),
            ("HTTP://Git.Local:8080/o/r/", "http://git.local:8080/o/r"),
            ("git:pw@gitserver:o/r.git?token=t", "ssh://gitserver/o/r"),
            ("ssh://git@[::1]:22/o/r", "ssh://[::1]:22/o/r"),
        ] {
            let kept = source(raw).sanitized().unwrap();
            assert_eq!(kept.repo.as_deref(), Some(sent), "{raw:?}");
        }
    }

    /// What the client sends, the server accepts and stores as it would the
    /// raw value (or stores nothing where it would refuse it); sanitizing
    /// twice changes nothing.
    #[test]
    fn sanitizing_composes_with_validating() {
        let raws = ACCEPTED
            .iter()
            .map(|(raw, _)| raw.to_string())
            .chain(LOCAL.iter().map(|raw| raw.to_string()))
            .chain(refused());
        for raw in raws {
            for s in [
                source(&raw),
                SourceInfo {
                    repo: Some(raw.clone()),
                    ..SourceInfo::default()
                },
            ] {
                let sent = s.sanitized();
                let validated = sent.as_ref().map_or(Ok(None), SourceInfo::validated);
                let dropped = SourceInfo {
                    repo: None,
                    ..s.clone()
                };
                let expected = s.validated().or_else(|_| dropped.validated());
                assert_eq!(validated, expected, "{raw:?}");
                assert_eq!(
                    sent.as_ref().and_then(SourceInfo::sanitized),
                    sent,
                    "idempotent: {raw:?}"
                );
            }
        }
    }

    #[derive(Debug, Deserialize)]
    struct Holder {
        #[serde(default, deserialize_with = "deserialize_opt")]
        source: Option<SourceInfo>,
    }

    #[test]
    fn deserializing_is_value_free() {
        let canary = "https://u:canary-7731@h/r";
        for json in [
            format!(r#"{{"source": "{canary}"}}"#),
            format!(r#"{{"source": ["{canary}"]}}"#),
            format!(r#"{{"source": {{"repo": ["{canary}"]}}}}"#),
        ] {
            let err = serde_json::from_str::<Holder>(&json)
                .unwrap_err()
                .to_string();
            assert!(!err.contains("canary"), "{err}");
        }
        let h: Holder = serde_json::from_str(r#"{"source": {"repo": "x", "extra": 1}}"#).unwrap();
        assert_eq!(h.source.unwrap().repo.as_deref(), Some("x"));
        let h: Holder = serde_json::from_str(r#"{"source": null}"#).unwrap();
        assert!(h.source.is_none());
        let h: Holder = serde_json::from_str("{}").unwrap();
        assert!(h.source.is_none());
    }
}
