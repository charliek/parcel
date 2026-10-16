# Run via mise so the pinned toolchain is on PATH, e.g. `mise exec -- make check`.
CARGO ?= cargo
IMAGE ?= us-central1-docker.pkg.dev/projects-413401/stridelabs/parcel
TAG ?= dev

.DEFAULT_GOAL := help

# ---- infra ---------------------------------------------------------------

.PHONY: up
up:  ## Start Postgres + the GCS emulator (required before test/functional)
	docker compose up -d

.PHONY: down
down:  ## Stop local infrastructure
	docker compose down

# ---- build / run -----------------------------------------------------------

.PHONY: build
build:  ## Build the server and CLI (debug)
	$(CARGO) build --workspace

.PHONY: serve
serve:  ## Run the server with .env (prox runs this as `backend`)
	$(CARGO) run -p parcel-server -- serve

.PHONY: openapi
openapi:  ## Regenerate the committed OpenAPI document
	$(CARGO) run -q -p parcel-server -- openapi > crates/parcel-server/openapi.json

.PHONY: install-cli
install-cli:  ## Install the parcel CLI from source into ~/.cargo/bin
	$(CARGO) install --locked --path crates/parcel-cli

# ---- quality ---------------------------------------------------------------

.PHONY: check
check: fmt-check lint test spectral functional frontend-check  ## Everything CI runs

.PHONY: fmt
fmt:  ## Format Rust code
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check:
	$(CARGO) fmt --all -- --check

.PHONY: lint
lint:  ## Clippy, warnings as errors (default and production feature sets)
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings
	$(CARGO) clippy -p parcel-server --no-default-features -- -D warnings

.PHONY: test
test:  ## Rust unit + integration tests (REQUIRES `make up`; fails loudly, never skips)
	$(CARGO) test --workspace

.PHONY: spectral
spectral:  ## Lint the committed OpenAPI document
	bunx @stoplight/spectral-cli@6.16.2 lint crates/parcel-server/openapi.json --fail-severity=warn

.PHONY: functional
functional:  ## Python functional suite against the real binaries (REQUIRES `make up`)
	uv run --group test pytest tests/functional

.PHONY: frontend-check
frontend-check:  ## Frontend lint, typecheck, unit tests and build
	cd frontend && bun install --frozen-lockfile && bun run lint && bunx tsc -b && bun test && bun run build

.PHONY: e2e
e2e:  ## Playwright e2e (needs the dev-auth stack: see frontend/README.md)
	cd frontend && bun run e2e

# ---- release ---------------------------------------------------------------

.PHONY: image
image:  ## Build the production image
	docker build -t $(IMAGE):$(TAG) .

.PHONY: image-push
image-push: image  ## Build and push the production image (TAG=vYYYY.MM.DD)
	docker push $(IMAGE):$(TAG)

.PHONY: help
help:  ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'
