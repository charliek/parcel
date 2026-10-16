#!/usr/bin/env bash
# Run a production-shaped parcel-server for the Playwright suite: one process
# serving the built SPA (PARCEL_STATIC_DIR) and the content origin, in dev
# auth mode, on a throwaway database. Prints the base URL and stays in the
# foreground; Ctrl-C to stop.
#
#   scripts/e2e-stack.sh [port]            # default 18110
#   PARCEL_E2E_BASE_URL=http://localhost:18110 make e2e
set -euo pipefail

port="${1:-18110}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
db=parcel_e2e

docker exec -i parcel-postgres psql -U postgres -v ON_ERROR_STOP=1 >/dev/null <<SQL
DROP DATABASE IF EXISTS $db WITH (FORCE);
CREATE DATABASE $db;
SQL

(cd "$root/frontend" && bun run build >/dev/null)
(cd "$root" && mise exec -- cargo build -q -p parcel-server)

blobs="$(mktemp -d)"
trap 'rm -rf "$blobs"' EXIT
echo "parcel e2e stack: http://localhost:${port} (content: http://127.0.0.1:${port})"
cd "$blobs"
env -i PATH="$PATH" HOME="$HOME" \
  APP_ENV=development RUST_LOG=info \
  BIND_ADDR="127.0.0.1:${port}" \
  DATABASE_URL="postgres://postgres:localdev@localhost:5439/${db}" \
  PARCEL_APP_ORIGIN="http://localhost:${port}" \
  PARCEL_CONTENT_ORIGIN="http://127.0.0.1:${port}" \
  PARCEL_CONTENT_SIGNING_KEY="e2e-signing-key-0123456789abcdef0123456789" \
  PARCEL_STATIC_DIR="$root/frontend/dist" \
  AUTH_MODE=dev DEV_AUTH_EMAIL=e2e@example.com DEV_AUTH_NAME="E2E User" \
  BLOB_BACKEND=fs BLOB_FS_ROOT="$blobs" \
  "$root/target/debug/parcel-server" serve
