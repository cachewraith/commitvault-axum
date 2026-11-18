# CommitVault backend — working notes for Claude

Rust + Axum REST API for CommitVault. This directory is its **own Git repository**
(`origin` → `cachewraith/commitvault-axum`); the frontend lives in `../frontend` and is a separate
repository. Never mix the two, never create a repository in the parent directory, and never push
without being asked.

## Commands

```bash
docker compose -f ../docker-compose.yml up -d postgres   # dev database on localhost:5433
cargo run                      # migrate + serve on http://127.0.0.1:3000 (reads ./.env)
cargo run -- openapi > openapi.json                       # export the OpenAPI contract
cargo test                     # needs DATABASE_URL; each test gets its own database
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo sqlx prepare             # after changing any query!/query_as! — commit .sqlx/
cargo sqlx migrate add <name>  # new migration (never edit an applied one)
```

`sqlx-cli` is installed with `cargo install sqlx-cli --no-default-features --features rustls,postgres`.

## Layout

```
src/
  main.rs            CLI: serve | migrate | openapi | healthcheck
  lib.rs             AppState, build_app(): middleware stack, sessions, CORS, docs
  openapi.rs         route table + OpenAPI document (utoipa-axum)
  config/            environment → Config (validated; secrets redacted in Debug)
  database/          pool + embedded migrations
  errors/            AppError → JSON error envelope (the only place errors become HTTP)
  middleware/        require_auth, csrf_protect, security headers, client IP
  auth/              login (+ emailed code, trusted devices)/logout/me/password/reset, Argon2id,
                     PgSessionStore, rate limiter, bootstrap
  mail.rs            outgoing email (SMTP via lettre; Disabled without SMTP_HOST; Memory in tests)
  shared/            pagination, response envelopes, extractors, validation rules,
                     rich-text sanitiser, serde helpers, SQL fragments, embedded refs
  modules/<feature>/ projects, environments, branches, commits, deployments, releases, notes,
                     tags, dashboard, search
migrations/          SQL migrations (embedded into the binary)
tests/api/           integration tests (one binary; `common.rs` is the harness)
.sqlx/               offline query metadata — committed
```

Each feature module has the same files and the same dependency direction:

| File | Responsibility |
| --- | --- |
| `routes.rs` | `OpenApiRouter` registering handlers with `routes!` |
| `handlers.rs` | HTTP only: extract → call service → wrap in `Data`/`Paginated`. `#[utoipa::path]` lives here |
| `service.rs` | Business rules, transactions, mapping of query params to filters |
| `repository.rs` | All SQL. Nothing else in the module touches `sqlx` |
| `models.rs` | Table rows used on the write path, and enums |
| `dto.rs` | Request types (`Deserialize + Validate + ToSchema`) and response types |

Handlers never contain business logic; services never build SQL; repositories never return HTTP errors.

## Conventions

- **Responses**: `Data<T>` (`{"data": …}`), `Paginated<T>` (`{"data": […], "meta": {…}}`), `204` for
  deletes. Errors are always `AppError` → `{"error": {"code", "message", "details?"}}`.
- **Read vs write models**: reads go straight from SQL into the response DTO (`#[derive(sqlx::FromRow)]`,
  related data embedded with `jsonb_build_object`/`jsonb_agg` sub-selects and `#[sqlx(json)]`), so a page
  is always one round trip. Writes load the row (`find_for_update`), mutate the model in Rust, then
  `update` — which is how PATCH supports "omit = keep, `null` = clear" without dynamic SQL.
- **Queries**: static statements use `sqlx::query!`/`query_as!` (compile-time checked, offline metadata in
  `.sqlx/`). Filtered lists use `QueryBuilder` with bind parameters. The only interpolated SQL is built
  from constants (`shared/sql.rs`, `TagTarget::table`) and is wrapped in `AssertSqlSafe` with a comment.
  Never interpolate request data into SQL.
- **PATCH DTOs**: nullable fields are `Option<Option<T>>` with `serde_helpers::patch`/`patch_text`.
- **Text input** is trimmed at deserialisation (`serde_helpers::trimmed*`), then validated with
  `validator` (shared rules in `shared/validation.rs`). Use `ValidJson` / `ValidQuery`, never bare
  `Json` / `Query`.
- **Search** terms go through `text::like_contains` / `like_prefix` (escapes `%`, `_`, `\`).
- **Lists are never unbounded**: every list takes `page`/`per_page` (max 100) or has a documented cap.
- **Rich text** (`notes.content_json`, `releases.release_notes`) must pass
  `shared::richtext::sanitize_document`. If the editor gains a node or mark, add it there *and* in
  `frontend/src/lib/components/editor`, with a test.
- **New endpoint checklist**: handler with `#[utoipa::path]` → `routes.rs` → service → repository →
  integration test in `tests/api/` → `cargo sqlx prepare` → re-export `openapi.json` → run
  `bun run gen:api` in the frontend → update `../docs/API.md` and the operation count in
  `tests/api/openapi.rs`.

## Domain rules that must not be broken

- A deployment is a **recorded event**. Never infer that a commit is absent from an environment because
  no record exists, and never output "ahead"/"behind": there is no Git ancestry data.
  (`deployments::service::COMPARISON_NOTICE`, tests in `tests/api/deployments.rs`.)
- CommitVault never runs Git, SSH, or any stored command. Deployment notes are plain text documentation.
- `UNIQUE(project_id, commit_hash)`; commits, deployments and release commits can only reference rows of
  their own project (composite foreign keys). Deleting a commit/environment with deployment history, or a
  branch with commits, needs `?cascade=true`.
- A commit references an existing branch (`branch_id`); recording a commit never creates a branch. A new
  project gets a branch named after its `default_branch`.
- Single user: there is no registration endpoint and no per-record ownership. Every feature route sits
  behind `require_auth`; only `/health` and the sign-in endpoints (`/auth/{csrf,options,login,login/verify,
  login/resend,logout,password/forgot,password/reset}`) are public.
- A *pending* session (password ok, emailed code outstanding) holds `pending_user_id`, never `user_id`.
  One-time codes and device tokens are stored hashed and never logged. Integration tests run with
  `Mailer::memory()`; `TestApp::login()` completes the code step, `TestApp::without_email()` disables it.

## Security checklist for changes

Authentication and CSRF are enforced by middleware — new routes get both automatically when added through
`modules::router()`. Do not log request bodies, cookies or secrets (`Secret` redacts itself). Do not return
`sqlx` errors to clients (convert through `AppError`). Keep CORS limited to `FRONTEND_ORIGIN`. Any stored
URL must pass `validation::http_url`. See `../docs/SECURITY.md`.
