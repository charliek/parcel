#!/usr/bin/env bash
# The one-crypto-provider guard (CLAUDE.md, "Toolchain"): no crate in the
# graph, under any feature, edge kind or target, depends on `ring` (a second
# rustls/jsonwebtoken backend next to aws-lc-rs panics at the first TLS
# connect or JWT sign/verify) or `rsa` (RUSTSEC-2023-0071, no fix).
# `make lint` and CI run it; it fails on any output.
#
# `cargo tree -i <crate>` answers three ways: the inverse tree (the crate is
# in the graph: fail), nothing on stdout with exit 0 (in Cargo.lock but
# nothing compiles it: ring's optional edges), or exit 101 with "did not
# match any packages" (not in Cargo.lock at all: rsa). Only those last two
# pass; any other error fails rather than passing as "empty". A positive
# control (aws-lc-rs must be found) proves the listing is the real graph.
set -uo pipefail
cargo=${CARGO:-cargo}
flags=(--locked --workspace --all-features -e all --target all)
err=$(mktemp)
trap 'rm -f "$err"' EXIT
failed=0

absent() { # crate
  local out rc
  out=$("$cargo" tree -i "$1" "${flags[@]}" 2>"$err")
  rc=$?
  if [ -z "$out" ] && { [ $rc -eq 0 ] || grep -q 'did not match any packages' "$err"; }; then
    echo "ok: no crate depends on $1"
  else
    echo "error: $1 is in the dependency graph (or cargo tree failed, exit $rc):" >&2
    printf '%s\n' "$out" >&2
    cat "$err" >&2
    failed=1
  fi
}

if ! "$cargo" tree -i aws-lc-rs "${flags[@]}" 2>"$err" | grep -q '^aws-lc-rs'; then
  echo "error: positive control failed: aws-lc-rs not found, so this listing proves nothing" >&2
  cat "$err" >&2
  exit 1
fi
absent ring
absent rsa
exit $failed
