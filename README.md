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
make up        # Postgres on localhost:5432 and private RustFS on localhost:9000
make run       # API on http://api-fittune.local:4733, applies migrations
```

The API listens on `0.0.0.0:4733` (`FITTUNE_BIND_ADDR`), and the app runs on `http://fittune.local:4734`.
Fixed, uncommon ports plus named hosts avoid clashes with other local apps on 3000, 5173 or 8080.
Add both names to `/etc/hosts` once (needs `sudo`):

```text
127.0.0.1  fittune.local api-fittune.local
```

`FITTUNE_CORS_ORIGINS` in `.env.example` allows `http://fittune.local:4734` (and `localhost:4734`); copy that line into an existing `.env` created before the change.
`localhost:4733` keeps working for curl and tests, the host name only matters for the browser.

```bash
make test      # unit + integration tests (uses DATABASE_URL from .env)
make lint      # rustfmt --check and clippy -D warnings
make help      # everything else
```

Integration tests use `#[sqlx::test]`: every test gets its own freshly migrated database created
through `DATABASE_URL`, so the role needs `CREATEDB`.

`make up` also starts RustFS on `localhost:9000` and creates the private
`fittune-progress-photos` bucket. The API uses the S3-compatible endpoint with path-style
addressing. Progress photos are decoded, resized and re-encoded as JPEG before upload; the
original file and its EXIF metadata are never stored. A thumbnail and full-size image are kept
in RustFS, and only authenticated API endpoints can read them. Use separate, random
`FITTUNE_PHOTOS_ACCESS_KEY` and `FITTUNE_PHOTOS_SECRET_KEY` values outside local development.

`.env.example` sets `FITTUNE_REGISTRATION=open`, so local sign-up works without invites.
Admins manage the shared exercise catalog, list users and issue invite codes:

```bash
make create-admin USERNAME=root EMAIL=root@example.com   # new admin, prompts for the password
make grant-admin LOGIN=oskyy                             # promote an existing account
```

### Development fixtures

One command gives a local database with realistic data, so the app does not start from an empty account:

```bash
make up && make seed   # creates fittune_dev, applies migrations, seeds it
make run-fixtures      # API on http://api-fittune.local:4733 against fittune_dev
```

Fixtures go into their own database, `FITTUNE_FIXTURES_DATABASE_URL` (`fittune_dev` in `.env.example`), never the one `make run` uses and never the `DATABASE_URL` that tests create their databases from.
The command only exists in builds with the `dev-fixtures` cargo feature, so the release binary and the production image cannot seed anything.
It also refuses to run unless `FITTUNE_ENV=development`, the database is on localhost and its name ends in `_dev`.
Nothing seeds automatically: not migrations, not startup, not deployment.

Every account uses the dev-only password `FitTune#Dev1`:

| Account | What it shows |
|---------|---------------|
| `admin@fittune.test` | Admin screens: users, and invites that are active, used, expired and revoked |
| `demo@fittune.test` | 12 weeks of push/pull/legs plus a session earlier today, with progressive overload, warm-up, drop and failure sets, a deload week, records, 4 routines, a gym and a home place, custom exercises (weight, reps and duration tracking), runs, rides, swims and hikes |
| `casual@fittune.test` | lb/mi units, 6 light weeks with cardio finishers, and a workout in progress "on another device" |
| `newbie@fittune.test` | A new account with no places, routines or history |

The data goes through the real HTTP handlers with real sessions, so it passes the same validation and authorization as the app.
The admin is created like `create-admin`, and the other accounts sign up with invites it issues.
Only the expired invite is back-dated in SQL, because the API only takes expiry in days from now.

- **Rerunning** (`make seed`) updates the fixtures in place without duplicating anything.
  Workouts, activities and places have ids derived from fixed keys; routines and custom exercises are matched by name.
  A fixture workout you changed in the app has a newer revision and is kept, just as the API treats any other device.
- **Dates** count back from today, so recent-history screens stay populated; `make seed BASE_DATE=2026-09-01` (or `FITTUNE_FIXTURES_BASE_DATE`) pins them.
- **`make seed-reset`** deletes only the fixture accounts (through account deletion, so their data and photos go too), then seeds again. Other accounts in the database are untouched.
- **`make reset`** drops and recreates the whole fixture database, migrates and seeds it.

An integration test seeds a fresh `sqlx::test` database twice and resets it, so the fixtures fail in CI rather than on a laptop when the schema or an endpoint changes.

### Open Food Facts products

FitHealth's barcode scan and product search use a copy of [Open Food Facts](https://world.openfoodfacts.org) in `off_products` (ODbL: the app credits Open Food Facts wherever the data is shown).
It is imported once from OFF's CSV export and never fetched at runtime.
Products with a barcode and a name are kept, with or without nutrition; search only offers the ones with nutrition.

Import the export into the local fixture database (about 5 minutes for the ~13 GB file, 4 M products):

```bash
FITTUNE_DATABASE_URL=postgres://fittune:fittune@localhost:5432/fittune_dev \
  cargo run --release -- import-off /path/to/en.openfoodfacts.org.products.csv
```

The import replaces the table in one transaction, so running it again with a newer export is safe.
Production gets the finished table instead of re-running the import:

```bash
# local: dump only the imported rows
docker exec fittune-api-fittune-postgres-1 \
  pg_dump -U fittune -d fittune_dev --data-only --table=off_products -Fc -Z 9 > off_products.dump
scp off_products.dump vps:/tmp/

# VPS: the API release with the off_products migration must be running first
cd /srv/fittune/api   # where docker-compose.prod.yml lives
docker compose -f docker-compose.prod.yml exec -T fittune-postgres \
  sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "TRUNCATE off_products" &&
         pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" --data-only --single-transaction' < /tmp/off_products.dump
rm /tmp/off_products.dump
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
| `OPENAI_API_KEY` | (unset) | Optional server key from the dedicated GitHub secret; AI OCR is disabled when absent |
| `FITTUNE_OCR_ENDPOINT` | (unset) | Private RapidOCR sidecar URL; local OCR/manual entry remain available when absent |
| `FITTUNE_APP_VERSION` | `dev` | Reported by `/health`; set to the release tag on deploy |
| `FITTUNE_PHOTOS_ENDPOINT` | — | RustFS S3 endpoint; photo API is unavailable when unset |
| `FITTUNE_PHOTOS_BUCKET` | — | Private bucket, created by Compose |
| `FITTUNE_PHOTOS_ACCESS_KEY` / `FITTUNE_PHOTOS_SECRET_KEY` | — | RustFS credentials; required with the endpoint |
| `FITTUNE_REGISTRATION` | `invite_only` | `invite_only` (sign-up needs an admin's invite code) or `open` (local development only) |
| `FITTUNE_CLIENT_IP_HEADER` | (unset) | Header with the client IP set by a trusted proxy, used to rate-limit invite guessing; `X-Real-IP` in production |

## Deployment

The static frontend is deployed separately. For this API, GitHub Actions builds the Docker image,
pushes it to GHCR as `ghcr.io/wichtowski/fittune-api:<release tag>`, and runs the Compose stack on
the VPS over SSH. The VPS never builds anything and holds no source code: `DEPLOY_PATH` contains
only `docker-compose.prod.yml`, the `Makefile` (for the `prod-*` targets) and `.env`.

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
| `deploy-backend.yml` | reusable / manual | validates the Compose config, builds and pushes the image to GHCR, copies the Compose file, `Makefile` and `.env` to the VPS, pulls the image and runs `docker compose up -d --wait` |
| `backend-control.yml` | manual | `start` / `stop` / `restart` the API container (Postgres keeps running) |

Required secrets (same names as EchoTrade): `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`,
`DEPLOY_SSH_PASSPHRASE`, `DEPLOY_PATH` (e.g. `/opt/fittune`), and `ENV_PRODUCTION` — the
production `.env` body. The GHCR push and the VPS pull use the workflow's own `GITHUB_TOKEN`, so
no registry secret is needed; the VPS login is removed again when the deploy ends. The image package
is private and linked to this repository.

Add `FITTUNE_PHOTOS_BUCKET`, `FITTUNE_PHOTOS_ACCESS_KEY`, and
`FITTUNE_PHOTOS_SECRET_KEY` to `ENV_PRODUCTION` before deploying this release. RustFS is
only on the internal Compose network; do not add a public port or bucket policy. Compose keeps
its data in the `rustfsdata` named volume. Back up that volume **together with** Postgres and
the deployment `.env` secrets, and restore the matching pair. For a consistent cold backup,
stop the stack with `docker compose --env-file .env -f docker-compose.prod.yml down` (which
preserves named volumes), archive the Compose project's `pgdata` and `rustfsdata` volumes, then start
the stack again. Keep encrypted copies off the VPS and test a restore before relying on them.

The shared platform-edge Caddyfile currently caps FitTune API requests at 1 MB and blocks
`blob:` previews. Apply the reviewed changes in
[`docs/platform-edge-progress-photos.patch`](docs/platform-edge-progress-photos.patch) to the
`platform-edge` repository before deploying the app. The patch permits 11 MB at the proxy
(multipart overhead included); the API itself still rejects image files above 10 MB.

Minimal `ENV_PRODUCTION`:

```dotenv
POSTGRES_USER=fittune
POSTGRES_PASSWORD=<long-random-url-safe-password>
POSTGRES_DB=fittune
FITTUNE_CORS_ORIGINS=https://fittune.oskarwichtowski.com
```

`FITTUNE_DATABASE_URL` is derived from the `POSTGRES_*` values in `docker-compose.prod.yml`, and
`FITTUNE_APP_VERSION` is injected from the release tag and also selects the image to run.

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

## Nutrition label OCR

The shared Admin panel contains invites, users, catalogue management and the global OCR model setting.
The default is `gpt-6-luna`; only admins can select an allowed model, and changes are persisted with the actor and timestamp.
The deploy workflow passes the dedicated `OPENAI_API_KEY` GitHub secret into the API runtime environment; it is never part of a frontend bundle or Docker build argument.
See [fixture validation and runtime limits](tests/label-ocr/README.md) before enabling production OCR.
AI allows two requests per user per minute, twenty admitted attempts per UTC day and two concurrent requests globally.
The persisted daily global ceiling is two hundred attempts; provider failures count because they may incur charges.
Server OCR allows ten requests per user per minute and one concurrent inference globally.
Both paths allow one extraction per user across engines, reject overload without queueing, and return `Retry-After` for manual retry.
