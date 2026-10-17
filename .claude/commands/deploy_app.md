# Deploy Command

Deploy parcel to production Cloud Run via the `deploy.yml` GitHub Actions
workflow (migrate job, then the service; the same steps as
`deploy/deploy.sh`).

Production is the only deploy target for now: there is no staging
environment (`deploy/env.staging` is prepared, but nothing exists in GCP).

## Arguments

- Optional first argument: image tag to deploy (e.g., `v2026.10.16`). If
  omitted, the command prompts.

## Steps

1. **Determine image tag**: If no image tag argument was provided:
   - Look up the 3 most recent `v*` git tags: `git tag -l "v*" --sort=-v:refname | head -3`
   - Present them via AskUserQuestion with the most recent as the first option
     marked "(Recommended)"
   - The user can also type a custom tag via the "Other" option

2. **Verify image exists**: Confirm the tag exists in Artifact Registry
   - Run: `gcloud artifacts docker images list us-central1-docker.pkg.dev/projects-413401/stridelabs/parcel --include-tags --filter="tags:<TAG>" --format=json`
   - `release.yml` builds it when `/deploy:build` pushes a date tag. If the tag
     doesn't exist (or that workflow is still running:
     `gh run list --workflow=release.yml -L 3`), inform the user and suggest
     running `/deploy:build` first. Stop.

3. **Trigger deployment**: Dispatch the workflow
   - `gh workflow run deploy.yml --ref main -f environment=production -f image_tag=<TAG>`
   - Wait ~5 seconds for the run to register.

4. **Monitor the run**: Watch the workflow execution
   - Get the run ID: `gh run list --workflow=deploy.yml -L 1 --json databaseId -q '.[0].databaseId'`
   - Watch it: `gh run watch <run_id> --exit-status`
   - On failure: fetch the failed logs (`gh run view <run_id> --log-failed | tail -200`)
     and surface them. Stop without proceeding to verification.

5. **Verify deployment**: Confirm the new revision is healthy
   - Capture the new revision name + service URL:
     `gcloud run services describe parcel --region=us-central1 --format='value(status.latestReadyRevisionName,status.url)' --project=projects-413401`
   - Hit the smoke endpoints on both origins (one service answers both, routed
     by Host):
     - `curl -sS -o /dev/null -w '%{http_code}\n' https://parcel.stridelabs.ai/health/live`
     - `curl -sS -o /dev/null -w '%{http_code}\n' https://parcel.stridelabs.ai/health/ready`
     - `curl -sS -o /dev/null -w '%{http_code}\n' https://parcelcontent.stridelabs.ai/health/live`
   - All should be `200`. `/health/ready` pings the DB, so a 503 there means
     the migrate job succeeded but the service can't reach Neon's pooler —
     read `gcloud logging read 'resource.type=cloud_run_revision AND resource.labels.service_name=parcel' --limit=50` next.
   - `curl -s https://parcel.stridelabs.ai/api/v1/info` shows the sign-in
     providers the server was configured with.

6. **Report result**: Summarize the deployment
   - Image tag deployed
   - New Cloud Run revision name
   - Service URL + the three health HTTP codes

## Error Handling

- If `gh` CLI is not authenticated, inform the user to run `gh auth login`.
- If the image tag doesn't exist in Artifact Registry, suggest `/deploy:build` first.
- If the `deploy.yml` run fails, link the Actions URL and show
  `--log-failed` output; common causes are:
  - The GitHub → GCP auth step failing: the repo variables `GCP_WIF_PROVIDER`
    / `GCP_CI_SA` (`parcel-ci-prod`) or that account's
    `roles/iam.workloadIdentityUser` binding for `charliek/parcel` (see
    `deploy/README.md`, "CI and deploys").
  - A `secretKeyRef` in `deploy/cloud-run-service.yaml` naming a secret
    without a per-secret accessor binding to `parcel-prod` → the container
    fails at startup with `PERMISSION_DENIED`. Add the binding (never a
    project-wide role) and redeploy.
  - The `parcel-migrate` Cloud Run job failing: read its logs with
    `gcloud logging read 'resource.type=cloud_run_job AND resource.labels.job_name=parcel-migrate' --limit=50`.
    It uses the direct (non-pooled) Neon URL, `prod-parcel-database-url-direct`.
- If `/health/live` returns non-200 after the workflow reports success, suggest
  `gcloud logging read 'resource.type=cloud_run_revision AND resource.labels.service_name=parcel AND severity>=ERROR' --limit=50` and inspect the latest revision.
