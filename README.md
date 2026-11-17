# CommitVault — backend

REST API for [CommitVault](../README.md), a personal developer workspace for recording commits,
deployments, environments, releases and technical notes.

**Stack:** Rust (stable) · Axum 0.8 · Tokio · SQLx 0.9 · PostgreSQL · tower-sessions · Argon2id ·
utoipa (OpenAPI 3.1)

The API is a modular monolith. It stores what the user enters and nothing else: it never runs Git,
never connects to servers and never executes stored commands.

## Requirements

- Rust stable (1.94+; developed with 1.97)
- PostgreSQL 14+ (the workspace `docker-compose.yml` provides PostgreSQL 17)
- `sqlx-cli` for migrations and query metadata:
  `cargo install sqlx-cli --no-default-features --features rustls,postgres`

## Quick start

```bash
# 1. Database (from the workspace root; listens on localhost:5433)
docker compose -f ../docker-compose.yml up -d postgres

# 2. Configuration
cp .env.example .env        # then set DATABASE_URL, SESSION_SECRET, BOOTSTRAP_ADMIN_*
openssl rand -hex 48        # a suitable SESSION_SECRET

# 3. Run — migrations are applied on startup, the admin account is created on first start
cargo run
```

The API listens on `http://127.0.0.1:3000`:

- `GET /api/v1/health` — liveness + database check
- `GET /api/docs/` — Swagger UI (development only)
- `GET /api/v1/openapi.json` — OpenAPI document (development only)

Sign in with `BOOTSTRAP_ADMIN_EMAIL` / `BOOTSTRAP_ADMIN_PASSWORD`. The bootstrap variables are only
used while the `users` table is empty; afterwards change the password in the application.

## Configuration

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `APP_ENV` | | `development` | `development` or `production` |
| `APP_HOST` / `APP_PORT` | | `127.0.0.1` / `3000` | Listen address |
| `DATABASE_URL` | yes | | PostgreSQL connection string |
| `DATABASE_MAX_CONNECTIONS` | | `10` | Pool size |
| `SESSION_SECRET` | yes | | ≥ 64 characters; signs the session cookie |
| `COOKIE_SECURE` | | `false` (dev) / `true` (prod) | `Secure` cookie + `__Host-` prefix + HSTS |
| `SESSION_TTL_HOURS` | | `168` | Idle session lifetime |
| `BOOTSTRAP_ADMIN_EMAIL` / `BOOTSTRAP_ADMIN_PASSWORD` | first start | | Initial administrator (password ≥ 12 characters) |
| `FRONTEND_ORIGIN` | | `http://localhost:5173` | Exact origin(s) for CORS and the CSRF origin check |
| `TRUST_PROXY` | | `false` | Trust `X-Forwarded-For` (set behind Traefik) |
| `ENABLE_API_DOCS` | | `true` (dev) / `false` (prod) | Swagger UI and `openapi.json` |
| `RUST_LOG` | | `commitvault=info,tower_http=info,sqlx=warn` | Log filter |

With `APP_ENV=production` the server refuses to start unless cookies are secure, the frontend origin
is HTTPS and no placeholder secret is left in place.

## Commands

```bash
cargo run                    # migrate + serve
cargo run -- migrate         # run migrations and exit
cargo run -- openapi         # print the OpenAPI document
cargo test                   # unit + integration tests (needs DATABASE_URL)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

### Database migrations

Migrations live in `migrations/`, are embedded in the binary and run automatically on `serve`.

```bash
cargo sqlx migrate add add_something   # create a new migration file
cargo sqlx migrate run                 # apply (or just start the server)
cargo sqlx prepare                     # refresh .sqlx/ after changing compile-time checked queries
```

`.sqlx/` is committed so the crate builds without a database (`SQLX_OFFLINE=true` is the default
whenever `DATABASE_URL` is unset).

### Tests

```bash
cargo test
```

Integration tests (`tests/api/`) use `#[sqlx::test]`: every test creates its own database from
`DATABASE_URL`, runs the migrations and drives the real router in-process, so sessions, CSRF,
validation and SQL are all exercised. The database user therefore needs `CREATEDB` (the Compose
PostgreSQL user has it).

### OpenAPI contract

The route table and the OpenAPI document are generated from the same handler registrations
(`utoipa-axum`), so they cannot drift. After changing the API:

```bash
cargo run -- openapi > openapi.json      # commit this file
(cd ../frontend && bun run gen:api)      # regenerate the TypeScript types
```

## Docker

```bash
docker build -t commitvault-backend .
```

Multi-stage build → small Debian image running as an unprivileged user. The binary doubles as its
own health probe (`commitvault healthcheck`), so the image needs no curl. See the workspace
[`docs/DEVELOPMENT.md`](../docs/DEVELOPMENT.md) for Compose usage and production deployment.

## Project layout

```
src/
  main.rs          CLI entry point (serve | migrate | openapi | healthcheck)
  lib.rs           application state and middleware stack
  openapi.rs       route table + OpenAPI document
  config/ database/ errors/ middleware/ auth/ shared/
  modules/         projects, environments, commits, deployments, releases, notes, tags,
                   dashboard, search — each: routes, handlers, service, repository, models, dto
migrations/        SQL schema
tests/api/         integration tests
```

Architecture, database design, API reference and the security model are documented in
[`../docs`](../docs).
