#!/usr/bin/env bash
# Bump parcel's release version.
#
# parcel is a Cargo workspace: every crate (parcel-types, parcel-server,
# parcel-cli) inherits `version.workspace = true`, so the canonical version is
# `[workspace.package].version` in the root Cargo.toml. This script bumps that
# field, regenerates Cargo.lock so the three workspace entries match, and bumps
# the Claude Code plugin manifest (.claude-plugin/plugin.json), which carries
# the same version and is checked by release-cli.yml's version-check job.
#
# Contract (see cc-plugins release-workflows references/update-version/README.md):
#   - one arg: semver string, no `v` prefix
#   - idempotent
#   - no network (--offline)
#   - verifies its own work
#   - does not `git add` (the release skill stages + commits)
#
# Portable to macOS: no `sed -i` (its argument differs between BSD and GNU);
# files are rewritten through a temp file instead.

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <X.Y.Z>   e.g. $0 0.1.1" >&2
  exit 2
fi
V="$1"

if [[ ! "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
  echo "error: '$V' is not semver (X.Y.Z or X.Y.Z-suffix)" >&2
  exit 2
fi

if [[ ! -f Cargo.toml || ! -f .claude-plugin/plugin.json ]]; then
  echo "error: run this from the repo root (Cargo.toml and .claude-plugin/plugin.json not found)" >&2
  exit 1
fi

tmp="$(mktemp)"
plugin_tmp="$(mktemp)"
trap 'rm -f "$tmp" "$plugin_tmp"' EXIT

# The version line inside [workspace.package], and nothing else: other
# sections ([workspace.dependencies] inline tables, a future [package]) may
# have their own `version =` keys.
workspace_version() {
  awk '
    /^\[/ { in_section = ($0 == "[workspace.package]") }
    in_section && /^version[[:space:]]*=/ {
      sub(/^version[[:space:]]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' Cargo.toml
}

# 1. Bump [workspace.package].version. Variable whitespace around `=` so both
#    vanilla cargo (`version = "0.1.0"`) and column-aligned layouts match.
count="$(awk '
  /^\[/ { in_section = ($0 == "[workspace.package]") }
  in_section && /^version[[:space:]]*=/ { n++ }
  END { print n + 0 }
' Cargo.toml)"
if [[ "$count" != "1" ]]; then
  echo "error: expected exactly one version line in [workspace.package] of Cargo.toml, found ${count}" >&2
  exit 1
fi
awk -v v="$V" '
  /^\[/ { in_section = ($0 == "[workspace.package]") }
  in_section && /^version[[:space:]]*=/ { print "version = \"" v "\""; next }
  { print }
' Cargo.toml >"$tmp"
cat "$tmp" >Cargo.toml

# 2. Verify Cargo.toml saw the bump. A silent no-match is the most common
#    failure mode here; catch it before blaming the lockfile.
actual="$(workspace_version)"
if [[ "$actual" != "$V" ]]; then
  echo "error: Cargo.toml's [workspace.package].version is '${actual:-<missing>}' after the bump, expected '$V'." >&2
  exit 1
fi
echo "Cargo.toml"

# 3. Regenerate Cargo.lock so the workspace entries match. --offline is safe:
#    only internal version strings change, not the dependency tree. Never
#    delete or fully regenerate the lockfile — it deliberately pins a yanked
#    `zip` that utoipa-swagger-ui needs.
#
#    Resolve cargo via mise when the repo's toolchain is pinned there
#    (.mise.toml is the source of truth), else fall back to whatever cargo is on
#    PATH (CI runners install Rust via actions-rust-lang and don't need mise).
if command -v mise >/dev/null 2>&1 && [[ -f .mise.toml ]]; then
  cargo=(mise exec -- cargo)
elif command -v cargo >/dev/null 2>&1; then
  cargo=(cargo)
else
  echo "error: cargo not found and 'mise exec' unavailable." >&2
  echo "       Install Rust (via rustup) or run inside a shell where" >&2
  echo "       \`mise exec -- cargo --version\` or \`cargo --version\` works." >&2
  exit 1
fi
"${cargo[@]}" update --workspace --offline >/dev/null

# 4. Verify every workspace crate's lockfile entry saw the bump.
for crate in parcel-types parcel-server parcel-cli; do
  locked="$(awk -v name="$crate" '
    $0 == "name = \"" name "\"" { found = 1; next }
    found && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
  ' Cargo.lock)"
  if [[ "$locked" != "$V" ]]; then
    echo "error: Cargo.lock has ${crate} at '${locked:-<missing>}', expected '$V'" >&2
    exit 1
  fi
done
echo "Cargo.lock"

# 5. The Claude Code plugin manifest carries the same version. jq rewrites it
#    through a temp file; the top-level .version is read back so a manifest
#    that lost the field fails the release here instead of shipping stale.
jq --arg v "$V" '.version = $v' .claude-plugin/plugin.json >"$plugin_tmp"
# mktemp makes the file 0600; the manifest is a tracked 0644 file.
chmod 644 "$plugin_tmp"
mv "$plugin_tmp" .claude-plugin/plugin.json

plugin_actual="$(jq -r '.version' .claude-plugin/plugin.json)"
if [[ "$plugin_actual" != "$V" ]]; then
  echo "error: .claude-plugin/plugin.json .version is '${plugin_actual}' after the bump, expected '$V'" >&2
  exit 1
fi
echo ".claude-plugin/plugin.json"

echo "Bumped parcel to $V"
