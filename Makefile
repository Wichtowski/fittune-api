# fittune-api — Rust backend

COMPOSE_PROD = docker compose --env-file .env -f docker-compose.prod.yml

.PHONY: help up down run check fmt lint test build docker-build backend-compose grant-admin prod-ps prod-logs

help:
	@echo "fittune-api targets:"
	@echo "  up              — start local Postgres (docker compose)"
	@echo "  down            — stop local infrastructure"
	@echo "  run             — run the API against .env (applies migrations on start)"
	@echo "  check           — cargo check"
	@echo "  fmt             — cargo fmt"
	@echo "  lint            — rustfmt --check + clippy -D warnings"
	@echo "  test            — unit + integration tests (needs DATABASE_URL, see .env.example)"
	@echo "  build           — release build"
	@echo "  docker-build    — build the production image"
	@echo "  backend-compose — build and run Postgres + API with Docker Compose"
	@echo "  grant-admin     — promote a user: make grant-admin LOGIN=<username-or-email>"
	@echo "  prod-ps         — (VPS) show production containers"
	@echo "  prod-logs       — (VPS) follow production API logs"

up:
	docker compose up -d fittune-postgres
	@echo "Postgres: localhost:$${FITTUNE_POSTGRES_PORT:-5432} (db/user/pass: fittune)"

down:
	docker compose down

run:
	cargo run -- serve

check:
	cargo check --all-targets

fmt:
	cargo fmt

lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

test:
	cargo test --all-targets

build:
	cargo build --release --locked

docker-build:
	docker build -t fittune-api:local .

backend-compose:
	docker compose up --build fittune-postgres fittune-api

grant-admin:
	@test -n "$(LOGIN)" || (echo "usage: make grant-admin LOGIN=<username-or-email>" && exit 1)
	cargo run -- grant-admin "$(LOGIN)"

prod-ps:
	$(COMPOSE_PROD) ps

prod-logs:
	$(COMPOSE_PROD) logs -f --tail=200 fittune-api
