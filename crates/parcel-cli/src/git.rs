//! What `publish` can tell about where it runs: the git repository of the
//! current directory, for a new parcel's `project:<name>` tag and every
//! version's source (repository, branch, commit).
//!
//! It is the *current directory's* repository, not the published files':
//! agents write pages into a scratch directory outside the repository but run
//! inside it. Any trouble (git missing, not a repository, a hung git past the
//! time budget) means no repository: publishing never fails because of it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use parcel_types::tags::normalize_tag;

/// The whole probe's budget (every git call together).
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// What the probe found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repo {
    /// The repository's name: the last path segment of `remote.origin.url`
    /// minus `.git`, else the main worktree's folder name. Not yet a tag.
    pub name: Option<String>,
    /// `remote.origin.url` exactly as configured. It may hold a credential:
    /// it leaves the CLI only through `SourceInfo::sanitized`.
    pub origin: Option<String>,
    /// The checked-out branch (absent when HEAD is detached).
    pub branch: Option<String>,
    /// HEAD's commit, abbreviated to 12 characters.
    pub commit: Option<String>,
}

/// Probe `cwd`'s repository with the `git` on `PATH`.
pub fn probe(cwd: &Path) -> Option<Repo> {
    probe_with(Path::new("git"), cwd, TIMEOUT)
}

/// Gave up: git could not run, or the budget ran out (nothing found is
/// trusted after that).
struct GaveUp;

fn probe_with(git: &Path, cwd: &Path, budget: Duration) -> Option<Repo> {
    let deadline = Instant::now() + budget;
    let run = |args: &[&str]| run_git(git, cwd, args, deadline);

    let located = run(&["rev-parse", "--show-toplevel", "--git-common-dir"]).ok()??;
    let mut lines = located.lines();
    let top = PathBuf::from(lines.next()?);
    let common = lines.next().map(|dir| resolve_dir(cwd, &top, dir));

    let origin = run(&["config", "--get", "remote.origin.url"]).ok()?;
    let branch = run(&["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()?
        .filter(|b| b != "HEAD");
    let commit = run(&["rev-parse", "--short=12", "HEAD"]).ok()?;
    let name = origin
        .as_deref()
        .and_then(name_from_remote)
        .or_else(|| common.as_deref().and_then(main_worktree_name));
    Some(Repo {
        name,
        origin,
        branch,
        commit,
    })
}

/// `git -C <cwd> <args>`'s trimmed stdout: `Ok(None)` when git fails or
/// prints nothing, `Err` when it can't run or outlives `deadline` (it is
/// killed then). The output is read on a thread and handed back over a
/// channel, so the wait has a timeout without any new dependency.
fn run_git(
    git: &Path,
    cwd: &Path,
    args: &[&str],
    deadline: Instant,
) -> Result<Option<String>, GaveUp> {
    let mut child = Command::new(git)
        .arg("-C")
        .arg(cwd)
        .args(args)
        // Never wait on a credential prompt or take the index lock.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GaveUp)?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.read_to_end(&mut out).map(|_| out);
        let _ = tx.send(read);
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    let out = match rx.recv_timeout(remaining) {
        Ok(Ok(out)) => out,
        _ => return Err(give_up(&mut child)),
    };
    // Its stdout is closed, so git is exiting; still, never wait past the budget.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => return Err(give_up(&mut child)),
        }
    };
    let text = String::from_utf8_lossy(&out).trim().to_string();
    Ok((status.success() && !text.is_empty()).then_some(text))
}

fn give_up(child: &mut Child) -> GaveUp {
    let _ = child.kill();
    let _ = child.wait();
    GaveUp
}

/// `--git-common-dir` may be relative (to the directory git ran in, though
/// some versions answer relative to the top level): the first that exists.
fn resolve_dir(cwd: &Path, top: &Path, dir: &str) -> PathBuf {
    let dir = Path::new(dir);
    if dir.is_absolute() {
        return dir.to_path_buf();
    }
    [cwd.join(dir), top.join(dir)]
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| cwd.join(dir))
}

/// The main worktree's folder: the common git dir's parent (`<main>/.git`),
/// or a bare repository's own name (`repo.git`). The same from every linked
/// worktree, which is what makes the fallback worktree-safe.
fn main_worktree_name(common: &Path) -> Option<String> {
    let common = common
        .canonicalize()
        .unwrap_or_else(|_| common.to_path_buf());
    let named = if common.file_name()? == ".git" {
        common.parent()?.file_name()?
    } else {
        common.file_name()?
    };
    let name = named.to_str()?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

/// The last path segment of a remote URL, minus `.git`. Only the path is
/// looked at (never the authority, which is where a credential sits:
/// `https://user:token@host`), and a segment that still holds `@`, `:` or
/// `%` is refused rather than risk putting a secret in a tag.
pub fn name_from_remote(raw: &str) -> Option<String> {
    let s = raw.trim().split(['?', '#']).next()?;
    let path = if let Some((_, rest)) = s.split_once("://") {
        rest.split_once('/').map_or("", |(_, path)| path)
    } else {
        // scp-like `user@host:path` (or a Windows drive): what follows the
        // first `:` before any `/`.
        let head = &s[..s.find(['/', '\\']).unwrap_or(s.len())];
        match head.find(':') {
            Some(colon) => &s[colon + 1..],
            None => s,
        }
    };
    let mut path = path.trim_end_matches(['/', '\\']);
    while let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.trim_end_matches(['/', '\\']);
    }
    let last = path.rsplit(['/', '\\']).next()?;
    (!last.is_empty() && !last.contains(['@', ':', '%'])).then(|| last.to_string())
}

/// `project:<name>` with the name folded into a tag value: lowercase, any
/// run of characters a tag can't hold turned into one `-`. `None` when
/// nothing usable is left.
pub fn project_tag(name: &str) -> Option<String> {
    const PREFIX: &str = "project:";
    let mut value = String::new();
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '+') {
            value.push(c);
        } else if !value.ends_with('-') {
            value.push('-');
        }
    }
    let value: String = value
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(parcel_types::tags::MAX_TAG_CHARS - PREFIX.len())
        .collect();
    let value = value.trim_end_matches('-');
    normalize_tag(&format!("{PREFIX}{value}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_remote_path_only() {
        for (remote, name) in [
            ("https://github.com/charliek/parcel.git", Some("parcel")),
            ("https://user:tok@github.com/o/Parcel", Some("Parcel")),
            ("git@github.com:o/r.git", Some("r")),
            ("git@github.com:r.git", Some("r")),
            ("ssh://git@host:22/o/r.git/", Some("r")),
            ("https://host/o/r.git?token=x#y", Some("r")),
            ("/srv/git/tools.git", Some("tools")),
            ("../sibling", Some("sibling")),
            ("C:\\src\\proj", Some("proj")),
            // No path: the authority is never a name (it holds the credential).
            ("https://user:token@host", None),
            ("https://host/", None),
            ("https://host/o/user:token@x", None),
            ("https://host/o/a%40b", None),
            ("", None),
        ] {
            assert_eq!(name_from_remote(remote).as_deref(), name, "{remote:?}");
        }
    }

    #[test]
    fn project_tags_fold_names_into_tag_values() {
        for (name, tag) in [
            ("parcel", Some("project:parcel")),
            ("My_Repo", Some("project:my-repo")),
            ("dots.and+plus", Some("project:dots.and+plus")),
            ("-weird--name!", Some("project:weird-name")),
            ("ünï", Some("project:n")),
            ("ü!", None),
            ("", None),
        ] {
            assert_eq!(project_tag(name).as_deref(), tag, "{name:?}");
        }
        let long = project_tag(&"a".repeat(100)).unwrap();
        assert_eq!(long.len(), parcel_types::tags::MAX_TAG_CHARS);
    }

    #[test]
    fn worktree_names_come_from_the_common_dir() {
        assert_eq!(
            main_worktree_name(Path::new("/nowhere/proj/.git")).as_deref(),
            Some("proj")
        );
        assert_eq!(
            main_worktree_name(Path::new("/nowhere/bare.git")).as_deref(),
            Some("bare")
        );
    }

    #[test]
    fn a_missing_git_is_no_repository() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-git");
        assert_eq!(probe_with(&missing, dir.path(), TIMEOUT), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_git_is_given_up_on() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("git");
        std::fs::write(&fake, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        assert_eq!(
            probe_with(&fake, dir.path(), Duration::from_millis(300)),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
