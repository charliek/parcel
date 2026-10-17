# Releasing the parcel CLI

> **Status: written, not yet run.** The repo is on GitHub
> ([charliek/parcel](https://github.com/charliek/parcel)) and CI runs, but no
> release tag has been pushed, so nothing below has executed. The workflow,
> formula template and version script follow strix (a Rust CLI already
> shipped this way) and are ready for the first tag once the
> [one-time setup](#one-time-setup) is done.

The general release framework is `cc-plugins:release-workflows`; this file
documents what's specific to this repo. It covers the `parcel` CLI
(`crates/parcel-cli`) and the two Claude Code plugins in this repo's
marketplace (`parcel`, the CLI skill at the repo root, and `parcel-mcp`, the
MCP server, in `plugins/parcel-mcp/`), which release with it. The server's
container image is deployed separately (see `deploy/`).

## TL;DR

    /release-workflows:release v0.1.1

Everything after that is automatic.

## What happens

1. **`release-workflows:release`** (LLM, local):
   - Verifies branch (`main`) + clean tree + `ci-success` green on HEAD
   - Asks/confirms the version
   - Drafts a `CHANGELOG.md` entry from `git log v<previous>..HEAD`, commits as
     `docs(changelog): vX.Y.Z entry`
   - Runs `scripts/release/update-version.sh X.Y.Z` (see below)
   - Commits as `chore(version): bump to X.Y.Z`, tags `vX.Y.Z` (annotated),
     `git push --follow-tags`

2. **`.github/workflows/release-cli.yml`** (CI, on tag push `v*`):
   - **version-check** — the tag matches `[workspace.package].version` in the
     root `Cargo.toml` **and** both plugin manifests' `version`
     (`.claude-plugin/plugin.json`, `plugins/parcel-mcp/.claude-plugin/plugin.json`)
   - **ci-gate** — polls for a green `ci-success` check on the tagged commit
   - **create-release** — this version's `CHANGELOG.md` section (or generated
     notes when there is no changelog yet) → `gh release create`
   - **build** (matrix, native runners) — `cargo build --locked --release -p parcel-cli`
     for `darwin/amd64`, `darwin/arm64`, `linux/amd64`, `linux/arm64`; tarballs
     `parcel_<os>_<arch>.tar.gz` (one file, `parcel`); `cargo deb -p parcel-cli`
     on the two linux targets → `parcel_<version>_<arch>.deb`; all uploaded to
     the Release
   - **finalize-release** — flips the Release out of draft and marks it latest
     (so the public download URLs resolve before the next jobs hash them)
   - **homebrew** — renders `scripts/release/parcel.rb.tmpl` with the four
     tarball sha256s and pushes `Formula/parcel.rb` to `charliek/homebrew-tap`
   - **apt-dispatch** — `repository_dispatch` (`event_type=publish`,
     `package=parcel`) to `charliek/apt-charliek`, which collects the `.deb`s and
     republishes its index (skipped for prereleases)

Users then install with:

    brew install charliek/tap/parcel
    sudo apt install parcel        # after adding the apt-charliek repository

## Version files this repo owns

`scripts/release/update-version.sh X.Y.Z` bumps, without network access:

- `Cargo.toml` — `[workspace.package].version`, the canonical version. All
  three crates (`parcel-types`, `parcel-server`, `parcel-cli`) inherit it via
  `version.workspace = true`, so the CLI's `--version`, the server's
  `/api/v1/info` version and the `.deb` version move together.
- `Cargo.lock` — `cargo update --workspace --offline`, which rewrites only the
  three workspace entries. Don't delete or regenerate the lockfile: that
  would move every dependency in the release commit.
- `.claude-plugin/plugin.json` and `plugins/parcel-mcp/.claude-plugin/plugin.json`
  — `.version`, so both plugins and the CLI that embeds the skill share a
  version (`parcel-cli`'s `both_plugin_manifests_carry_the_workspace_version`
  test fails between releases if one drifts).

NOT bumped: `frontend/package.json` (the SPA is shipped inside the server image,
not versioned independently) and `.claude-plugin/marketplace.json` (no version
field).

## Claude Code plugins

Users install from this repo (`/plugin marketplace add charliek/parcel`), so a
plugin change ships when it reaches `main`; the version bump at release is
what tells Claude Code to update installed copies. Before a release, run
`claude plugin validate ./plugins/parcel-mcp` and `claude plugin validate .`.
`plugins/parcel-mcp/skills/parcel-mcp/references/authoring.md` is a copy of
the CLI skill's (an installed plugin can't read outside its directory);
`parcel-cli`'s tests fail when they differ. The MCP plugin's `.mcp.json`
pins the OAuth client (`parcel-claude-code`) and callback port 3118, which
slauth registers as `http://localhost:3118/callback`; change both together.

## Packaging details

- **Binary:** `parcel`, Rust with rustls on aws-lc-rs (statically linked),
  trusting the OS store (the platform verifier) plus bundled Mozilla roots;
  no OpenSSL, and the `.deb` depends only on libc.
- **`.deb`:** `[package.metadata.deb]` in `crates/parcel-cli/Cargo.toml` names
  the Debian package `parcel` (not the crate name `parcel-cli`), installs
  `/usr/bin/parcel`, and sets `revision = ""` so the file is
  `parcel_<version>_<arch>.deb`, matching the apt-charliek glob. Verified
  locally with cargo-deb: `Package: parcel`, `Depends: libc6`.
- **Formula:** `charliek/tap/parcel`, class `Parcel`. Always install it
  tap-qualified; the bare name `parcel` may collide with other formulae.
- **Skill:** embedded in the binary (`include_str!` of `skills/parcel/`), so
  `parcel skill install` works from any package; the Claude Code plugin
  (`.claude-plugin/`) installs the same files from the repo.

## One-time setup

Do these once, after the repo is pushed to `github.com/charliek/parcel`:

1. **Push the repo** and keep it **public** — the homebrew job downloads the
   release tarballs anonymously to hash them.
2. **CI with a `ci-success` check.** `ci-gate` waits for a check run named
   `ci-success` on the tagged commit, so the CI workflow must end in a job with
   that name (the release-workflows convention's aggregate job).
3. **Install the release-bot GitHub App** (App ID `3902108`) on
   **charliek/parcel**, **charliek/homebrew-tap** and **charliek/apt-charliek**
   (Contents: read + write). It is the single credential for the formula push
   and the apt dispatch — no PATs.
4. **Repository secrets** on charliek/parcel:

   | Secret | Purpose |
   |---|---|
   | `RELEASE_BOT_CLIENT_ID` | release-bot App client id (used by `actions/create-github-app-token`) |
   | `RELEASE_BOT_APP_KEY` | release-bot App private key (`.pem`) |

   If `RELEASE_BOT_CLIENT_ID` is unset, `homebrew` and `apt-dispatch` log a
   warning and skip instead of failing the release.
5. **apt-charliek:** add a `parcel` entry to `apt-charliek/packages.yaml`
   (source repo `charliek/parcel`, asset glob `parcel_*.deb`), following the
   existing `strix` entry.
6. **Branch protection on `main`:** a ruleset requiring `ci-success`, with the
   release-bot App (type `Integration`) and the Admin role (id `5`,
   `RepositoryRole`) as bypass actors so `/release-workflows:release` can push
   the changelog and version commits plus the tag.
7. **First release:** the tree starts at `0.1.0`. Cut `v0.1.1` (or `v0.1.0`
   after confirming nothing has been tagged) with the release skill; the
   version-bump commit must be a real change, so pick a version other than the
   one already in `Cargo.toml`.

## When things break

| Symptom | Cause | Fix |
|---|---|---|
| `git push` rejected: "Required status check ci-success" | Pusher not in the ruleset bypass list | Add the App and the admin role to `bypass_actors` |
| `create-github-app-token` → `no access to <repo>` | App not installed on that repo | Install the App on the missing repo, re-run the job |
| Tag pushed, `version-check` fails | Tagged a commit that didn't run `update-version.sh` (or `plugin.json` drifted) | Re-bump locally and cut a fresh patch tag (don't move an existing tag) |
| `build` fails at `--locked` | `Cargo.lock` out of date with a manifest change | Run `cargo update --workspace --offline` (never regenerate the lock), commit, re-tag a patch |
| `homebrew` 404s downloading tarballs | Release still a draft, or CDN lag | `finalize-release` handles the draft; the job retries for lag — re-run if it still fails |
| `homebrew`/`apt-dispatch` warned + skipped | `RELEASE_BOT_CLIENT_ID` unset | Set the App secrets; re-run those jobs |
| apt repo doesn't show the new version | `parcel` missing from apt-charliek `packages.yaml`, or the dispatch didn't fire | Add the entry and re-run `apt-dispatch` |
