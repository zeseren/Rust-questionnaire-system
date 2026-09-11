# Repository Guidelines

> Language convention: source comments, error messages, tests and `docs/` are written in Chinese. This file is in English, but new comments/messages/docs must stay Chinese.

## Project Overview

`questionnaire-api` is a Rust HTTP JSON API for a questionnaire/survey system (learning project; **no frontend in this repo** — share "links" are JSON endpoints, not pages).

Domain: publishers create surveys with 6 question types (single/multiple choice, text, textarea, number, rating), publish immutable versions, collect answers anonymously or one-per-account, then view per-version statistics and export CSV. Admins govern users/surveys with an audit trail.

Layout: a single-member Cargo workspace (`backend/`, package `questionnaire-api`, lib + thin bin). Docs: `README.md` (operator runbook), `docs/backend-guide.md` (code-reading tour + REST walkthrough), `docs/requirements-analysis.md` (requirements baseline + §11 verification ledger).

## Architecture & Data Flow

Strict three layers: HTTP boundary → business use case → data access → PostgreSQL.

- `main.rs` only calls `questionnaire_api::run()` (`lib.rs`). `run()` is the composition root: dotenv → tracing (`RUST_LOG`, default `questionnaire_api=info`) → `Config::from_env()` → sea-orm pool (max 10 conns) → `migration::Migrator::up` **on every boot** → argv subcommand (`serve` default | `migrate` | `create-admin`) → `TcpListener` → `axum::serve(...).with_graceful_shutdown(ctrl_c)`.
- One shared `AppState { db: DatabaseConnection, config: Arc<Config>, limits: Arc<Limits> }` (`lib.rs`), attached once via `.with_state(state)`.
- Every feature module exposes `pub(crate) fn router() -> Router<AppState>` plus a utoipa `ApiDoc`; `lib.rs::router()` merges `auth`, `surveys`, `submissions`, `reports`, `admin`, then adds `/health`, `/api/openapi.json`, 404/405 fallbacks, `DefaultBodyLimit::max(1 MiB)`, and the `security::guard` middleware. OpenAPI is served as JSON at `/api/openapi.json` (no Swagger UI).
- Handlers are thin: extract (`State` → `AuthUser` → `HeaderMap` → `ApiPath`/`ApiQuery`/`ApiJson`), call a business fn that takes `&AppState` + owner id, return `AppResult<Json<T>>` or `AppResult<(StatusCode, Json<T>)>`. Handler lives next to its business fn in the same file; only cross-module data-access rules go in `repository.rs`.
- `domain.rs` is pure (no HTTP, no DB): `Document`, `Question`, `QuestionKind`, `Answer`, `validate_draft`, `validate_publish`, `validate_answers`, `renew_ids`, plus number canonicalization. `ValidationError { question_id: Option<Uuid>, message }` surfaces as HTTP 422.
- Persistence: SeaORM entities in `entities.rs` (tables `users`, `sessions`, `surveys`, `versions`, `responses`, `audits`) + shared ownership/lock helpers in `repository.rs`. Write paths open `state.db.begin()`, lock publisher `user` (Share / NoKeyUpdate) **then** `survey` (Update), mutate, `tx.commit()`. Read-only list/statistics paths use `RepeatableRead` + `ReadOnly`.
- Schema: `backend/src/schema.sql` is the only DDL artifact — raw PostgreSQL, embedded with `include_str!` and executed by the single migration in `migration.rs`. Add schema changes there, not in a second place.
- Errors: one struct `AppError { status, code: &'static str, message, question_id }` (`error.rs`); body `{"error":{"code":"...","message":"...","question_id":null}}`; `From<DbErr|sqlx::Error|serde_json::Error>` → 500 without leaking ORM text; alias `AppResult<T>`.
- Security: cookie-only sessions (opaque 32-byte hex token; DB stores hex SHA-256 in `sessions.id_hash`), Argon2id passwords executed in `spawn_blocking`. `security::guard` requires `Origin` exactly equal to `PUBLIC_ORIGIN` **and** `X-CSRF-Protection: 1` for every non-GET/HEAD/OPTIONS request, enforces two fixed-window rate limits (`/api/auth/` writes; public response POSTs), and sets `Cache-Control: no-store` + `X-Content-Type-Options: nosniff`. No CORS layer, by design.

Request trace example — `POST /api/surveys/{id}/publish`: `surveys::publish` → `publish_survey(&state, owner_id, id)` → `repository::lock_owned_survey` (user Share → survey Update, ownership check) → `repository::expire_survey` → `ensure_revision` (409 on stale draft revision) → `Document::validate_publish()` (domain rules) → insert `versions` row (`number = max + 1`) + update `surveys.current_version_id` → `tx.commit()` → 201.

## Key Directories

- `backend/src/` — all source, flat (one file per feature area, no subdirectories).
- `backend/tests/` — integration tests (`domain.rs` is currently the entire suite).
- `docs/` — Chinese design docs (`backend-guide.md`, `requirements-analysis.md`).
- repo root — `Cargo.toml` / `Cargo.lock` (workspace + lockfile), `compose.yaml` (optional Postgres 18 only), `.env.example`.

Module map (useful reading order):

| File | Responsibility |
|---|---|
| `lib.rs` | `run()`, `router()`, `AppState`, `/health`, OpenAPI merge |
| `main.rs` | binary entry, no logic |
| `config.rs` | env vars + fail-fast validation |
| `error.rs` | `AppError` / `AppResult`, error envelope |
| `http.rs` | `ApiJson` / `ApiQuery` / `ApiPath` extractors, `Pagination` |
| `security.rs` | CSRF + rate-limit + security-header middleware |
| `auth.rs` | `AuthUser` extractor, Argon2, sessions, cookie plumbing |
| `domain.rs` | pure survey document model + validation |
| `entities.rs` | 6 SeaORM entity modules |
| `repository.rs` | ownership lookups + row locking rules |
| `migration.rs` + `schema.sql` | migration entry + authoritative DDL |
| `surveys.rs` | owner survey lifecycle, drafts, publish/close/archive |
| `submissions.rs` | public fill surface + idempotent submit |
| `reports.rs` | response list/detail, statistics, CSV export |
| `admin.rs` | governance (users/surveys) + audit writes |

## Development Commands

Run from the repo root (README uses PowerShell).

```powershell
Copy-Item .env.example .env          # then set DATABASE_URL
docker compose up -d db              # optional local Postgres 18; requires $env:POSTGRES_PASSWORD
cargo run --locked -p questionnaire-api -- migrate          # apply migrations, exit
cargo run --locked -p questionnaire-api                     # default `serve` on 127.0.0.1:3000
cargo run --locked -p questionnaire-api -- create-admin     # reads ADMIN_USERNAME / ADMIN_PASSWORD
Invoke-RestMethod http://localhost:3000/health              # {"status":"ready"}
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

- `--locked` is the documented norm; the root `Cargo.lock` is authoritative.
- The server needs a reachable PostgreSQL; config/DB errors abort startup (no in-memory fallback, no DB auto-create). Migrations always run at startup, including before subcommand dispatch.
- No Makefile / justfile / scripts / CI — call cargo directly.

## Code Conventions & Common Patterns

- **Chinese output**: `//!`/`///` docs, error messages, tests and docs are Chinese. Start each file with a `//!` line stating the invariant it defends (e.g. `repository.rs`: lock user before survey).
- **Visibility**: modules are private; items crossing modules are `pub(crate)`. Only `pub mod domain` and `pub async fn run()` are public.
- **Errors**: never ad-hoc tuples; use `AppError::new|validation|field|conflict|not_found|unauthorized|forbidden|internal`. Codes are stable snake_case (`csrf_rejected`, `revision_conflict`, `idempotency_conflict`, `rate_limited`, `not_ready`, …). Never expose ORM/SQL text to clients or logs. `anyhow` only at bootstrap (`.context(...)`).
- **DTOs**: requests `#[derive(Deserialize, ToSchema)] #[serde(deny_unknown_fields)]`; enums `#[serde(rename_all = "snake_case")]`; `QuestionKind` and statistics use internally tagged `#[serde(tag = "type")]`. Responses are DTOs with `impl From<entity::Model>` — never serialize SeaORM models directly.
- **Extractors**: use the `ApiJson` / `ApiQuery` / `ApiPath` newtypes (`http.rs`) so parse rejections become `AppError` (400/413). Lists go through `Pagination::bounds()` (page ≥ 1, page_size ≤ 100, default 20).
- **Persistence**: SeaORM builder style; raw SQL only for PostgreSQL-specific needs via `Expr::cust_with_values` / sqlx `query_as` with **bound parameters** (never string interpolation). After `begin()` stay on `&tx`. `LockType::Share` for user reads, `NoKeyUpdate` for user writes, `Update` for surveys.
- **Concurrency**: single lock order — publisher user, then survey — centralized in `repository.rs` (`lock_user`, `lock_survey`, `lock_owned_survey`). Collection eligibility goes through `ensure_collecting` / `expire_survey`; do not invent a second ordering or re-check outside the transaction.
- **Crypto/secrets**: passwords Argon2id (`Argon2::default()`, per-hash salt) inside `spawn_blocking`; session tokens and submission idempotency keys hashed with SHA-256 hex (`hash_secret`). Never derive `Debug` for structs carrying secrets.
- **IDs & time**: `Uuid::new_v4()` for new ids; `chrono::Utc::now()` / `DateTime<Utc>`; API timestamps are RFC3339 and time ranges are half-open `[from, to)`.
- **Numbers**: `serde_json` is compiled with `arbitrary_precision` on purpose (JSONB number fidelity); do not remove that feature.
- **Headers**: if you add response headers, keep the existing `no-store` / `nosniff` behavior intact.

## Important Files

- `backend/src/lib.rs` — composition root: `run()`, `router()`, `AppState`, health check, OpenAPI merge. Read first.
- `backend/src/error.rs` — the error contract everything returns.
- `backend/src/repository.rs` — the locking/ownership invariant every writer obeys.
- `backend/src/domain.rs` — all validation rules; the only module with automated tests.
- `backend/src/http.rs`, `backend/src/config.rs`, `backend/src/security.rs` — boundary concerns (extractors, env contract, request gating).
- `backend/Cargo.toml` — MSRV/edition/features; `Cargo.lock` — exact pins.
- `backend/tests/domain.rs` — the whole test suite.
- `compose.yaml`, `.env.example` — optional DB and the env contract.
- `README.md`, `docs/backend-guide.md`, `docs/requirements-analysis.md` — runbook, architecture walkthrough, requirements + §11 "verified vs unverified" ledger. Note: the requirements doc also describes a planned Vue frontend and performance targets that **do not exist in this repo** — treat README and code as current.

## Runtime/Tooling Preferences

- Rust stable **≥ 1.94**, edition 2024 (verified locally on 1.98.1). No `rust-toolchain.toml`; the installed toolchain is used.
- Cargo workspace, single member `backend`; dependencies live in `backend/Cargo.toml` (no workspace dependency inheritance, no `[features]`, `[dev-dependencies]`, `[profile]` or `[lints]` sections). `publish = false`.
- Key pinned versions: axum 0.8.9, sea-orm / sea-orm-migration 2.0.2, sqlx 0.9.0, tokio 1.53.1, utoipa 5.5.0.
- PostgreSQL 18 is the runtime baseline; there is no SQLite/in-memory mode and no Redis/queue. Compose only supplies the DB — the API always runs via `cargo run`. TLS termination, reverse proxy and container images are explicitly out of scope.
- Configuration is env-only: `.env` is loaded by `dotenvy` from the repo root (process env wins; a missing `.env` is fine).

| Variable | Required | Default | Notes |
|---|---|---|---|
| `DATABASE_URL` | yes | — | must be `postgres://` / `postgresql://` |
| `LISTEN_ADDR` | no | `127.0.0.1:3000` | `IP:port` |
| `PUBLIC_ORIGIN` | no | `http://localhost:3000` | bare origin, no path/trailing slash; `http` only for localhost |
| `COOKIE_SECURE` | no | `false` | must be `true` on HTTPS, `false` on HTTP |
| `SESSION_TTL_SECONDS` | no | `86400` | 1..=2 592 000 |
| `AUTH_REQUESTS_PER_MINUTE` | no | `60` | whole-instance fixed window |
| `SUBMISSIONS_PER_MINUTE` | no | `600` | whole-instance fixed window |
| `RUST_LOG` | no | `questionnaire_api=info` | tracing `EnvFilter` |
| `ADMIN_USERNAME`, `ADMIN_PASSWORD` | only for `create-admin` | — | process env, not in `.env.example` |
| `POSTGRES_PASSWORD` | only for Compose | — | injected into the `db` service |

- Formatting/linting uses stock rustfmt + clippy (no config files); `-D warnings` is the CI-substitute gate.
- No CI workflows, Dockerfile, task runners or scripts exist. Do not assume one.

## Testing & QA

- Framework: built-in `cargo test`; one integration target `backend/tests/domain.rs` (7 synchronous tests) covering pure domain rules only: normalization/idempotence (answer order, `-0.0` → `0`), duplicate/foreign answer identities, required vs optional answers, Unicode scalar limits + NUL rejection, number precision/overflow, and JSONB round-trip.
- `cargo test --locked --workspace` needs **no database and no env vars**. Useful filters: `cargo test -p questionnaire-api --test domain <name_substring>`.
- Style: build input via the existing helpers (`id`, `question`, `choices`, `document`, `answer`); assert on `Result` and `question_id`, never on message wording; parse raw JSON strings with `serde_json::from_str` when digit precision matters.
- Scope expectation: the suite is deliberately limited to pure domain logic. Do not add DB-less tests for HTTP/ORM plumbing — those paths are verified manually against a real Postgres (`docs/requirements-analysis.md` §11.2 records what was run, §11.3 lists what was not). If you change `domain.rs`, extend `tests/domain.rs`; if you change HTTP/auth/SQL behavior, verify by running the service.
- There is no coverage tooling, benchmark, mock framework or dev-dependency section; adding test-only crates requires creating `[dev-dependencies]`.
