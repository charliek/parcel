# Deploying parcel (Cloud Run + Neon + GCS)

> Status: **prepared, not yet executed.** Everything here is written against
> the fleet's existing GCP conventions (project `projects-413401`,
> `us-central1`, Artifact Registry `stridelabs`, per-env runtime service
> accounts, `{env}-parcel-{purpose}` secrets). Nothing has been created in
> GCP, Neon or DNS yet.

## Shape

One Cloud Run service per environment answers **two domains**:

| Env | App (SPA + API) | Content (sandboxed user files) |
|---|---|---|
| staging | `parcel-staging.stridelabs.ai` | `parcelcontent-staging.stridelabs.ai` |
| production | `parcel.stridelabs.ai` | `parcelcontent.stridelabs.ai` |

The server routes by `Host` (`PARCEL_CONTENT_ORIGIN`). Content responses carry
`Content-Security-Policy: sandbox …`, which gives them an opaque origin, so the
content domain can be a sibling subdomain without being able to touch
`*.stridelabs.ai` cookies. A separate registrable domain (e.g.
`stridelabsusercontent.com`) would add defence in depth and is a one-line
change to `CONTENT_DOMAIN`.

## One-time setup (per environment)

Shown for production; substitute `staging` / `parcel-staging` names.

1. **Runtime service account** — no project-level roles.
   ```bash
   gcloud iam service-accounts create parcel-prod --project projects-413401
   ```
2. **Bucket** — private, uniform access, the runtime SA as object admin. The
   server authenticates through the metadata server, so no key file.
   ```bash
   gcloud storage buckets create gs://parcel-prod-blobs --location us-central1 \
     --uniform-bucket-level-access --public-access-prevention
   gcloud storage buckets add-iam-policy-binding gs://parcel-prod-blobs \
     --member serviceAccount:parcel-prod@projects-413401.iam.gserviceaccount.com \
     --role roles/storage.objectAdmin
   ```
3. **Neon** — a `parcel` project (or branch) with database `parcel`. Keep both
   the pooled URL (service) and the direct URL (migrate job).
4. **Secrets** — each with its own accessor binding (never project-wide):
   ```bash
   for s in database-url database-url-direct content-signing-key; do
     gcloud secrets create prod-parcel-$s --replication-policy automatic
     gcloud secrets add-iam-policy-binding prod-parcel-$s \
       --member serviceAccount:parcel-prod@projects-413401.iam.gserviceaccount.com \
       --role roles/secretmanager.secretAccessor
   done
   printf %s "$POOLED_URL"  | gcloud secrets versions add prod-parcel-database-url --data-file=-
   printf %s "$DIRECT_URL"  | gcloud secrets versions add prod-parcel-database-url-direct --data-file=-
   openssl rand -hex 32 | tr -d '\n' | gcloud secrets versions add prod-parcel-content-signing-key --data-file=-
   ```
   Rotating the signing key invalidates outstanding content links (≤ 12 h old);
   viewers just reload.
5. **Domains** — map both to the service, then add DNS-only (grey-cloud)
   Cloudflare CNAMEs to `ghs.googlehosted.com`:
   ```bash
   gcloud beta run domain-mappings create --service parcel --domain parcel.stridelabs.ai --region us-central1
   gcloud beta run domain-mappings create --service parcel --domain parcelcontent.stridelabs.ai --region us-central1
   ```
6. **Keep capability URLs out of Cloud Run's request log.** The server
   redacts `/v/<token>` (files) and `/d/<token>` (page data) in its own logs,
   but Cloud Run's platform request log records every full URL, and a content
   URL is a bearer capability until it expires. Exclude those requests from
   the `_Default` sink (and any other sink that exports logs):
   ```bash
   gcloud logging sinks update _Default --project projects-413401 \
     --add-exclusion='name=parcel-content-capabilities,filter=resource.type="cloud_run_revision" AND resource.labels.service_name=~"^parcel" AND (httpRequest.requestUrl:"/v/" OR httpRequest.requestUrl:"/d/")'
   ```
7. **slauth** — the `parcel` client and app are on slauth's
   `feature/add-parcel-app` branch (redirects for all three envs). Merge and
   deploy slauth before parcel's first login in that environment, and hand out
   the `parcel-…` invite code (or `scripts/admin-cli.sh grants add --app parcel`).

## Each deploy

```bash
make image-push TAG=v2026.10.16          # build + push us-central1-docker.pkg.dev/.../parcel:<tag>
deploy/deploy.sh staging v2026.10.16     # migrate job (direct URL), then the service
deploy/deploy.sh production v2026.10.16
```

`deploy.sh` resolves the tag to a digest, runs `parcel-server migrate` as a
Cloud Run job, replaces the service from `cloud-run-service.yaml`
(`RUN_MIGRATIONS=false` there), grants `allUsers` invoker, and probes both
domains. Once the repo is on GitHub, `.github/workflows/release.yml` builds
images on date tags and `deploy.yml` runs the same steps with Workload
Identity Federation (the tapper pattern: `GCP_WIF_PROVIDER`, `GCP_CI_SA`).
