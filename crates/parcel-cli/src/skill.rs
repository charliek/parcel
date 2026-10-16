//! The bundled agent skill (`skills/parcel/` in the repo), embedded at build
//! time so `parcel skill install` works from a release binary.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// One file of the skill, relative to the skill directory.
pub struct SkillFile {
    pub path: &'static str,
    pub contents: &'static str,
}

/// Every file of the skill. `tests::embeds_every_repo_file` fails if a file is
/// added to `skills/parcel/` without being listed here.
pub const SKILL_FILES: &[SkillFile] = &[
    SkillFile {
        path: "SKILL.md",
        contents: include_str!("../../../skills/parcel/SKILL.md"),
    },
    SkillFile {
        path: "references/authoring.md",
        contents: include_str!("../../../skills/parcel/references/authoring.md"),
    },
    SkillFile {
        path: "references/cli.md",
        contents: include_str!("../../../skills/parcel/references/cli.md"),
    },
    SkillFile {
        path: "references/troubleshooting.md",
        contents: include_str!("../../../skills/parcel/references/troubleshooting.md"),
    },
];

pub fn skill_md() -> &'static str {
    SKILL_FILES[0].contents
}

/// `$HOME`, else the OS home directory. `HOME` first so tests (and users)
/// can redirect it.
pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .context("could not determine your home directory; set HOME or pass --dir")
}

/// `<root>/.claude/skills/parcel` — the one location Claude Code, craze
/// (native agent) and opencode all scan.
pub fn claude_skill_dir(root: &Path) -> PathBuf {
    root.join(".claude").join("skills").join("parcel")
}

/// `<root>/.agents/skills/parcel` — the vendor-neutral location.
pub fn agents_skill_dir(root: &Path) -> PathBuf {
    root.join(".agents").join("skills").join("parcel")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Wrote,
    Unchanged,
}

/// Write the skill into each directory. Nothing is written if any existing
/// file differs from the bundled one and `force` is false.
pub fn install(dirs: &[PathBuf], force: bool) -> Result<Vec<(PathBuf, Outcome)>> {
    let mut plan = Vec::new();
    let mut conflicts = Vec::new();
    for dir in dirs {
        for file in SKILL_FILES {
            let target = dir.join(file.path);
            let outcome = match fs::read(&target) {
                Ok(existing) if existing == file.contents.as_bytes() => Outcome::Unchanged,
                Ok(_) => {
                    if !force {
                        conflicts.push(target.clone());
                    }
                    Outcome::Wrote
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Outcome::Wrote,
                Err(e) => return Err(e).with_context(|| format!("reading {}", target.display())),
            };
            plan.push((target, file.contents, outcome));
        }
    }
    if !conflicts.is_empty() {
        let list: Vec<String> = conflicts
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect();
        bail!(
            "these files differ from the bundled skill (local edits, or an older version):\n{}\n\
             pass --force to overwrite them",
            list.join("\n")
        );
    }
    let mut done = Vec::new();
    for (target, contents, outcome) in plan {
        if outcome == Outcome::Wrote {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&target, contents)
                .with_context(|| format!("writing {}", target.display()))?;
        }
        done.push((target, outcome));
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_skill_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/parcel")
    }

    #[test]
    fn embedded_bytes_match_the_repo_files() {
        for file in SKILL_FILES {
            let on_disk = fs::read_to_string(repo_skill_dir().join(file.path)).unwrap();
            assert_eq!(file.contents, on_disk, "{} is stale", file.path);
        }
    }

    #[test]
    fn embeds_every_repo_file() {
        let root = repo_skill_dir();
        let mut on_disk: Vec<String> = walkdir::WalkDir::new(&root)
            .into_iter()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        on_disk.sort();
        let mut embedded: Vec<String> = SKILL_FILES.iter().map(|f| f.path.to_string()).collect();
        embedded.sort();
        assert_eq!(embedded, on_disk);
    }

    #[test]
    fn skill_has_frontmatter() {
        let md = skill_md();
        assert!(
            md.starts_with("---\nname: parcel\ndescription: "),
            "{}",
            &md[..60]
        );
    }

    #[test]
    fn installs_and_refuses_to_clobber_edits() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_skill_dir(home.path());
        let done = install(std::slice::from_ref(&dir), false).unwrap();
        assert_eq!(done.len(), SKILL_FILES.len());
        assert!(done.iter().all(|(_, o)| *o == Outcome::Wrote));
        assert_eq!(
            fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            skill_md()
        );

        // Re-running is a no-op.
        let done = install(std::slice::from_ref(&dir), false).unwrap();
        assert!(done.iter().all(|(_, o)| *o == Outcome::Unchanged));

        // A local edit blocks the install unless forced.
        fs::write(dir.join("SKILL.md"), "edited").unwrap();
        let err = install(std::slice::from_ref(&dir), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--force"), "{err}");
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "edited");
        install(std::slice::from_ref(&dir), true).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            skill_md()
        );
    }
}
