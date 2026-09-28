# fittune-api — Rust backend

COMPOSE_PROD = docker compose --env-file .env -f docker-compose.prod.yml
# Local fixtures only exist in builds with the dev-fixtures feature, never in the release image
SEED_DEV = cargo run --features dev-fixtures -- seed-dev $(if $(BASE_DATE),--base-date $(BASE_DATE))
FIXTURES_DATABASE_URL ?= $(or $(FITTUNE_FIXTURES_DATABASE_URL),$(shell sed -n 's/^FITTUNE_FIXTURES_DATABASE_URL=//p' .env 2>/dev/null))

.PHONY: help up down run seed seed-reset reset run-fixtures check fmt lint test build docker-build backend-compose create-admin grant-admin prod-create-admin prod-grant-admin prod-ps prod-logs

help:
	@echo "fittune-api targets:"
	@echo "  up              — start local Postgres and RustFS (docker compose)"
	@echo "  down            — stop local infrastructure"
	@echo "  run             — run the API against .env (applies migrations on start)"
	@echo "  seed            — create/update the local fixture database (FITTUNE_FIXTURES_DATABASE_URL); BASE_DATE=YYYY-MM-DD optional"
	@echo "  seed-reset      — delete the fixture accounts and their data, then seed again"
	@echo "  reset           — drop and recreate the whole fixture database, migrate, seed"
	@echo "  run-fixtures    — run the API against the fixture database"
	@echo "  check           — cargo check"
	@echo "  fmt             — cargo fmt"
	@echo "  lint            — rustfmt --check + clippy -D warnings"
	@echo "  test            — unit + integration tests (needs DATABASE_URL, see .env.example)"
	@echo "  build           — release build"
	@echo "  docker-build    — build the production image"
	@echo "  backend-compose — build and run Postgres + API with Docker Compose"
	@echo "  create-admin    - new admin, prompts for the password: make create-admin USERNAME=<u> EMAIL=<e>"
	@echo "  grant-admin     - promote a user: make grant-admin LOGIN=<username-or-email>"
	@echo "  prod-create-admin / prod-grant-admin - the same on the VPS"
	@echo "  prod-ps         — (VPS) show production containers"
	@echo "  prod-logs       — (VPS) follow production API logs"

up:
	docker compose up -d fittune-postgres fittune-photos-bucket
	@echo "Postgres: localhost:$${FITTUNE_POSTGRES_PORT:-5432} (db/user/pass: fittune)"

down:
	docker compose down

run:
	cargo run -- serve

seed:
	$(SEED_DEV)

seed-reset:
	$(SEED_DEV) --reset

reset:
	$(SEED_DEV) --drop

run-fixtures:
	@test -n "$(FIXTURES_DATABASE_URL)" || (echo "set FITTUNE_FIXTURES_DATABASE_URL in .env, see .env.example" && exit 1)
	FITTUNE_DATABASE_URL="$(FIXTURES_DATABASE_URL)" cargo run -- serve

check:
	cargo check --all-targets --all-features

fmt:
	cargo fmt

lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all-targets --all-features

build:
	cargo build --release --locked

docker-build:
	docker build -t fittune-api:local .

backend-compose:
	docker compose up --build fittune-postgres fittune-api

create-admin:
	@test -n "$(USERNAME)" -a -n "$(EMAIL)" || (echo "usage: make create-admin USERNAME=<username> EMAIL=<email>" && exit 1)
	cargo run -- create-admin "$(USERNAME)" "$(EMAIL)"

grant-admin:
	@test -n "$(LOGIN)" || (echo "usage: make grant-admin LOGIN=<username-or-email>" && exit 1)
	cargo run -- grant-admin "$(LOGIN)"

# -it keeps a terminal attached so the password prompt does not echo
prod-create-admin:
	@test -n "$(USERNAME)" -a -n "$(EMAIL)" || (echo "usage: make prod-create-admin USERNAME=<username> EMAIL=<email>" && exit 1)
	$(COMPOSE_PROD) exec -it fittune-api /usr/local/bin/fittune-api create-admin "$(USERNAME)" "$(EMAIL)"

prod-grant-admin:
	@test -n "$(LOGIN)" || (echo "usage: make prod-grant-admin LOGIN=<username-or-email>" && exit 1)
	$(COMPOSE_PROD) exec fittune-api /usr/local/bin/fittune-api grant-admin "$(LOGIN)"

prod-ps:
	$(COMPOSE_PROD) ps

prod-logs:
	$(COMPOSE_PROD) logs -f --tail=200 fittune-api
