# fittune-api

Rust backend for FitTune — strength workouts, endurance activities, an exercise library and
training analytics. The web/mobile client lives in the separate
[`fittune-app`](https://github.com/wichtowski/fittune-app) repository and talks to this service
only through the HTTP contract in [`docs/API.md`](docs/API.md).

## Stack

- **axum** + **tokio** HTTP server with graceful shutdown, request ids, tracing, CORS, timeouts
- **PostgreSQL 16** via **sqlx** (runtime-checked queries, embedded migrations applied on start)
- **Argon2id** password hashing and opaque, hashed-at-rest bearer sessions
- Single static binary in a distroless, non-root container

## Layout

```text
src/
├── main.rs          - CLI: serve | migrate | healthcheck | create-admin | grant-admin
├── app.rs           - router assembly and middleware stack
├── config.rs        - FITTUNE_* environment configuration
├── error.rs         - ApiError → JSON error responses, field-level validation errors
├── auth/            - registration, login, sessions, `Auth` extractor
├── invites/         - admin-issued invite codes for invite-only registration
├── users/           - profile, password change, account deletion, admin directory
├── exercises/       - catalog + custom exercises, per-exercise history and records
├── routines/        - reusable workout plans
├── workouts/        - workout documents (client ids, revision-checked upserts)
├── activities/      - runs, rides, walks, ...
└── stats/           - overview, timeline, muscle distribution, personal records
migrations/          - schema and seeded exercise catalog
tests/api/           - HTTP integration tests against a real database
```

Each domain module keeps the same shape: `model.rs` (types and validation), `repo.rs` (SQL),
`routes.rs` (handlers).

## Local development

Prerequisites: Rust 1.94 (pinned in `rust-toolchain.toml`) and Docker Compose (or any Postgres 16).

```bash
cp .env.example .env
make up        # Postgres on localhost:5432
make run       # API on http://localhost:4733, applies migrations
```

```bash
make test      # unit + integration tests (uses DATABASE_URL from .env)
make lint      # rustfmt --check and clippy -D warnings
make help      # everything else
```

Integration tests use `#[sqlx::test]`: every test gets its own freshly migrated database created
through `DATABASE_URL`, so the role needs `CREATEDB`.

`.env.example` sets `FITTUNE_REGISTRATION=open`, so local sign-up works without invites.
Admins manage the shared exercise catalog, list users and issue invite codes:

```bash
make create-admin USERNAME=root EMAIL=root@example.com   # new admin, prompts for the password
make grant-admin LOGIN=oskyy                             # promote an existing account
```

## Configuration

| Variable | Default | Purpose |
|----------|---------|---------|
| `FITTUNE_DATABASE_URL` | — (required) | Postgres connection string |
| `FITTUNE_DB_MAX_CONNECTIONS` | `10` | Pool size |
| `FITTUNE_BIND_ADDR` | `0.0.0.0:4733` | Listen address |
| `FITTUNE_CORS_ORIGINS` | `http://localhost:5173` | Comma-separated allowed browser origins |
| `FITTUNE_SESSION_TTL_HOURS` | `720` | Idle lifetime of a session |
| `FITTUNE_LOG_FORMAT` | `pretty` | `pretty` or `json` |
| `FITTUNE_LOG` | `fittune_api=info,tower_http=info,sqlx=warn,info` | tracing filter |
| `FITTUNE_APP_VERSION` | `dev` | Reported by `/health`; set to the release tag on deploy |
| `FITTUNE_REGISTRATION` | `invite_only` | `invite_only` (sign-up needs an admin's invite code) or `open` (local development only) |
| `FITTUNE_CLIENT_IP_HEADER` | (unset) | Header with the client IP set by a trusted proxy, used to rate-limit invite guessing; `X-Real-IP` in production |

## Deployment

Deployment follows the EchoTrade VPS pattern: the static frontend is deployed separately, and this
repository ships a Docker Compose stack to the VPS over SSH.

- `api-fittune.oskarwichtowski.com` → `fittune-api:4733`, routed by the global platform edge
- Nothing in this stack publishes host ports. `fittune-api` sits on the external `fittune_edge_net`
  (shared with the platform-edge Caddy, create it once with `docker network create fittune_edge_net`);
  Postgres is only on the internal `data_net`.
- Postgres data lives in the named `pgdata` volume and survives redeploys.

GitHub Actions:

| Workflow | Trigger | What it does |
|----------|---------|--------------|
| `ci.yml` | PRs, pushes to `main` | fmt, clippy, unit + integration tests against Postgres |
| `create-release-tag.yml` | merge to `main` / manual | creates the next `x.y.z` tag |
| `deploy-release.yml` | manual, on a tag | validates the tag, then runs `deploy-backend.yml` |
| `deploy-backend.yml` | reusable / manual | packages the repo, copies it and `.env` to the VPS, `docker compose up -d --build --wait` |
| `backend-control.yml` | manual | `start` / `stop` / `restart` the API container (Postgres keeps running) |

Required secrets (same names as EchoTrade): `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`,
`DEPLOY_SSH_PASSPHRASE`, `DEPLOY_PATH` (e.g. `/opt/fittune`), and `ENV_PRODUCTION` — the
production `.env` body. Repository variable: `DEPLOY_ARCHIVE` (e.g. `fittune-api.tar.gz`).

Minimal `ENV_PRODUCTION`:

```dotenv
POSTGRES_USER=fittune
POSTGRES_PASSWORD=<long-random-url-safe-password>
POSTGRES_DB=fittune
FITTUNE_CORS_ORIGINS=https://fittune.oskarwichtowski.com
```

`FITTUNE_DATABASE_URL` is derived from the `POSTGRES_*` values in `docker-compose.prod.yml`, and
`FITTUNE_APP_VERSION` is injected from the release tag.

Useful commands on the VPS (from `DEPLOY_PATH`): `make prod-ps`, `make prod-logs`.

### Invite-only registration and the first admin

Production registration is invite-only by default: `POST /api/v1/auth/register` needs a code an admin issued, and a fresh installation with no admin stays closed.
Nobody is ever promoted automatically, so an operator creates the first admin on the VPS.
Run these from `DEPLOY_PATH` in an interactive SSH session.

If the admin already has an account, promote it:

```bash
make prod-grant-admin LOGIN=<username-or-email>
```

On a fresh installation, create the admin instead:

```bash
make prod-create-admin USERNAME=<username> EMAIL=<email>
```

The command prompts for the password twice without echoing it, and applies the same password policy and hashing as sign-up.
Never pass the password as an argument or through an environment variable, and never paste it into issues, PRs or chat.

Verify after deploying (record only the non-sensitive outcomes, for example in the release notes):

1. Sign in to the app as the admin and open Profile → Invites.
2. Create an invite and check it is listed as active; the code is shown only once.
3. In a private window, try to sign up without a code and check it is rejected with "An invite code is required".
4. Sign up with the code, then check the invite is listed as used and the new account is not an admin.

Recovery:

- Lost admin password: create another admin with `make prod-create-admin`, sign in, and change or remove the old account.
- A code leaked: revoke it in Profile → Invites; accounts it already created stay and can be deleted by their owner.
- Locked out by the rate limit (10 wrong codes per client in 15 minutes): wait for the window to pass, or restart the API container (`backend-control.yml`), which clears the in-memory counters.
