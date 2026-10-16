#!/usr/bin/env bash
# Deploy parcel to Cloud Run by hand (until the repo is on GitHub and
# .github/workflows/deploy.yml takes over).
#
#   deploy/deploy.sh staging|production <image-tag>
#
# Builds nothing: push the image first with `make image-push TAG=<tag>`.
# Requires gcloud auth with run.admin + iam.serviceAccountUser on the runtime SA.
set -euo pipefail

env_name="${1:?usage: deploy.sh staging|production <image-tag>}"
tag="${2:?usage: deploy.sh staging|production <image-tag>}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$env_name" in
  staging|production) ;;
  *) echo "unknown environment: $env_name" >&2; exit 2 ;;
esac

set -a
# shellcheck disable=SC1090
. "$here/env.$env_name"
set +a

repo="${REGION}-docker.pkg.dev/${PROJECT_ID}/stridelabs/parcel"
# Deploy by digest so the revision is immutable even if the tag moves.
digest="$(gcloud artifacts docker images describe "${repo}:${tag}" \
  --format='value(image_summary.digest)')"
export IMAGE="${repo}@${digest}"
echo "deploying ${IMAGE} to ${SERVICE_NAME} (${ENV})"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
envsubst < "$here/cloud-run-migrate-job.yaml" > "$tmp/job.yaml"
envsubst < "$here/cloud-run-service.yaml" > "$tmp/service.yaml"

gcloud run jobs replace "$tmp/job.yaml" --region "$REGION" --project "$PROJECT_ID"
gcloud run jobs execute "${SERVICE_NAME}-migrate" --region "$REGION" --project "$PROJECT_ID" --wait

gcloud run services replace "$tmp/service.yaml" --region "$REGION" --project "$PROJECT_ID"
gcloud run services add-iam-policy-binding "$SERVICE_NAME" --region "$REGION" \
  --project "$PROJECT_ID" --member=allUsers --role=roles/run.invoker --quiet >/dev/null

for domain in "$APP_DOMAIN" "$CONTENT_DOMAIN"; do
  code="$(curl -s -o /dev/null -w '%{http_code}' "https://${domain}/health/live" || true)"
  echo "https://${domain}/health/live -> ${code}"
done
