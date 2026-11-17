# CommitVault — backend

REST API for CommitVault, a personal workspace for recording commits, deployments,
environments, releases and technical notes. Everything is entered by hand: the API never
runs Git, connects to servers or executes stored commands.

**Stack:** Rust · Axum · SQLx · PostgreSQL · Argon2id · OpenAPI (utoipa)
**Frontend:** [commitvault-svelt](https://github.com/cachewraith/commitvault-svelt)

## Run

```bash
cp .env.example .env     # set DATABASE_URL, SESSION_SECRET, BOOTSTRAP_ADMIN_*
cargo run                # http://localhost:3000 — migrations run on startup
```

Needs Rust stable and PostgreSQL 14+. The admin account is created on first start from
`BOOTSTRAP_ADMIN_EMAIL` / `BOOTSTRAP_ADMIN_PASSWORD`; there is no registration.

Generate a session secret with `openssl rand -hex 48`.

## Configuration

| Variable | Default | Notes |
| --- | --- | --- |
| `DATABASE_URL` | — | required |
| `SESSION_SECRET` | — | required, 64+ characters |
| `BOOTSTRAP_ADMIN_EMAIL` / `_PASSWORD` | — | first start only; password 12+ characters |
| `APP_ENV` | `development` | `production` enforces secure cookies and HTTPS origin |
| `APP_HOST` / `APP_PORT` | `127.0.0.1` / `3000` | |
| `FRONTEND_ORIGIN` | `http://localhost:5173` | exact origin, no wildcards |
| `COOKIE_SECURE` | `false` dev, `true` prod | |
| `SESSION_TTL_HOURS` | `168` | idle timeout |
| `TRUST_PROXY` | `false` | set `true` behind a reverse proxy |

## Commands

```bash
cargo test                                                # needs DATABASE_URL
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- openapi > openapi.json                       # export the API contract
cargo sqlx prepare                                        # after changing queries
```

## API

Base path `/api/v1`. The full contract is [`openapi.json`](openapi.json); in development
Swagger UI is served at `/api/docs/`.

Cookie sessions (HttpOnly, SameSite=Strict). Every `POST`, `PUT`, `PATCH` and `DELETE` must
send the token from `GET /auth/csrf` in the `X-CSRF-Token` header.

## Layout

```
src/modules/<feature>/   routes · handlers · service · repository · models · dto
src/auth/                login, sessions, rate limiting
src/shared/              pagination, validation, rich-text allow-list
migrations/              SQL schema
tests/api/               integration tests
```
