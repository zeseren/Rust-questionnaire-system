# Repository Guidelines

> Language convention: source comments, error messages, tests and `docs/` are written in Chinese. This file is in English, but new comments/messages/docs must stay Chinese.

## Project Overview

当前开发前提（2026-10-07）：项目尚无正式发布版本，目前不需要考虑历史答卷兼容。后续设计和实现可以直接调整当前模型、DSL 与数据结构，不为旧格式、历史答卷或旧版本增加兼容分支、迁移方案和回归负担。既有文档中的历史兼容描述是此前的实现记录，不作为当前开发约束；仍按当前契约保留必要的回归验证。

`questionnaire-api` 是问卷调查系统的 Rust HTTP JSON API；`frontend/` 现有连接真实接口的 Vue 3 前端 MVP。后端 `share_url` 仍指向 JSON 接口，前端填写页面为 `/s/:id`。

Domain: publishers create surveys with 6 question types (single/multiple choice, text, textarea, number, rating), publish immutable versions, collect answers anonymously or one-per-account, then view per-version statistics and export CSV. Admins govern users/surveys with an audit trail.

项目是三个成员的 Cargo workspace：`backend/`（包名 `questionnaire-api`，HTTP API 与业务用例）、`logic-core/`（包名 `questionnaire-logic`，纯领域 DSL 核心：表达式、文档规则、流程与 pest 文本文法；无 HTTP/DB/文件系统依赖，可编到 wasm32）、`logic-wasm/`（包名 `logic-wasm`，把核心以 JSON 字符串信封导出给前端）。前端位于独立的 `frontend/`，规则引擎直接调用入库的 wasm 产物，不再保留 TypeScript 镜像实现。使用与阅读说明见 `README.md`、`docs/backend-guide.md`、`docs/frontend-guide.md`；需求及分阶段验证记录见 `docs/requirements-analysis.md` 第 11—24 节。

## Architecture & Data Flow

Strict three layers: HTTP boundary → business use case → data access → PostgreSQL.

- `main.rs` only calls `questionnaire_api::run()` (`lib.rs`). `run()` is the composition root: dotenv → tracing (`RUST_LOG`, default `questionnaire_api=info`) → `Config::from_env()` → sea-orm pool (max 10 conns) → `migration::Migrator::up` **on every boot** → argv subcommand (`serve` default | `migrate` | `create-admin`) → `TcpListener` → `axum::serve(...).with_graceful_shutdown(ctrl_c)`.
- One shared `AppState { db: DatabaseConnection, config: Arc<Config>, limits: Arc<Limits> }` (`lib.rs`), attached once via `.with_state(state)`.
- 各业务模块暴露 `pub(crate) fn router() -> OpenApiRouter<AppState>`，通过 `utoipa_axum::routes!` 从 handler 的 `#[utoipa::path]` 同时登记路由与文档，不另写 `ApiDoc` 列表。`lib.rs::router()` 合并五个业务模块及 `/health`，用 `split_for_parts()` 取得路由和文档；文档只在组装时生成，`/api/openapi.json` 返回其副本。随后配置 404/405、`DefaultBodyLimit::max(domain::MAX_SUBMISSION_BYTES)`、`security::guard` 与共享状态；不提供 Swagger UI。不同结构的 DTO 使用不同 Schema 名称，尤其是发布者 `SurveyPage` 与治理 `AdminSurveyPage`。
- Handlers are thin: extract (`State` → `AuthUser` → `HeaderMap` → `ApiPath`/`ApiQuery`/`ApiJson`), call a business fn that takes `&AppState` + owner id, return `AppResult<Json<T>>` or `AppResult<(StatusCode, Json<T>)>`. Handler lives next to its business fn in the same file; only cross-module data-access rules go in `repository.rs`.
- `logic-core/src/domain.rs` is pure (no HTTP, no DB); `backend/src/lib.rs` 用 `pub use questionnaire_logic::domain;` 再导出，因此业务代码与测试里的 `questionnaire_api::domain::*` 路径不变。它包含 `Document`, `Question`, `QuestionKind`, `Answer`, `validate_draft`, `validate_publish`, `validate_answers`, `renew_ids`, 发布期提交预算（`estimated_submission_bytes` / `ensure_submission_budget`，与 HTTP body limit 共用 `MAX_SUBMISSION_BYTES`）, plus number canonicalization. `ValidationError { question_id: Option<Uuid>, message }` surfaces as HTTP 422.
- Persistence: SeaORM entities in `entities.rs` (tables `users`, `sessions`, `surveys`, `versions`, `responses`, `audits`) + shared ownership/lock helpers in `repository.rs`. Write paths open `state.db.begin()`, lock publisher `user` (Share / NoKeyUpdate) **then** `survey` (Update), mutate, `tx.commit()`. Read-only list/statistics paths use `RepeatableRead` + `ReadOnly`.
- Schema: `backend/src/schema.sql` is the only DDL artifact — raw PostgreSQL, embedded with `include_str!` and executed by the single migration in `migration.rs`. Add schema changes there, not in a second place.
- Errors: one struct `AppError { status, code: &'static str, message, question_id, error_id }` (`error.rs`); body `{"error":{"code":"...","message":"...","question_id":null}}`; `From<DbErr|sqlx::Error|serde_json::Error>` → 500 without leaking ORM text. `internal()` generates a 16-hex `error_id` (also returned as `X-Error-Id`) and logs `error_id` / `error_kind` / `location` via `#[track_caller]`; alias `AppResult<T>`.
- Security: cookie-only sessions (opaque 32-byte hex token; DB stores hex SHA-256 in `sessions.id_hash`), Argon2id passwords executed in `spawn_blocking`. `security::guard` requires `Origin` exactly equal to `PUBLIC_ORIGIN` **and** `X-CSRF-Protection: 1` for every non-GET/HEAD/OPTIONS request, enforces two fixed-window rate limits (`/api/auth/` writes; public response POSTs), and sets `Cache-Control: no-store` + `X-Content-Type-Options: nosniff`. No CORS layer, by design.
- Cookie 由 `cookie::Cookie::split_parse` / `Cookie::build` 解析与构造；遍历所有 Cookie 头并拒绝重复 session，不通过 CookieJar 覆盖同名项，也不做百分号解码。Argon2 0.6 自动生成每次哈希的随机盐；Session 的 32 字节随机值来自 `getrandom::fill`。

Request trace example — `POST /api/surveys/{id}/publish`: `surveys::publish` → `publish_survey(&state, owner_id, id)` → `repository::lock_owned_survey` (user Share → survey Update, ownership check) → `repository::expire_survey` → `ensure_revision` (409 on stale draft revision) → `Document::validate_publish()` + `ensure_submission_budget` (422 when the worst-case fill exceeds `MAX_SUBMISSION_BYTES`) → idempotent replay: if the draft equals the current version, return 200 with it; otherwise insert `versions` row (`number = max + 1`) + update `surveys.current_version_id` → `tx.commit()` → 201.

## Key Directories

- `backend/src/` — HTTP 与业务源码，flat (one file per feature area, no subdirectories)；DSL 实现已不在其中。
- `backend/tests/` — 独立回归目标：`domain.rs`、`logic.rs`、`flow.rs`、`logic_text.rs`；业务源码中不放测试。
- `logic-core/src/` — `questionnaire-logic`：DSL 六文件（`domain.rs`、`identity.rs`、`logic.rs`、`logic_document.rs`、`logic_text.rs` + `logic_text.pest`、`flow.rs`），纯领域逻辑，对外只导出 `domain`（聚合再导出）与 `identity`。
- `logic-wasm/src/lib.rs` + `logic-wasm/tests/smoke.rs` — wasm 边界（入参出参都是 JSON 字符串信封）；冒烟测试在原生目标调用同一份函数体。
- `frontend/src/wasm/` — `index.ts` 包装 + 入库的 `pkg/` 产物五件套（胶水、导入表、`.wasm` 与两份类型声明）；不装 Rust 工具链也能直接使用。
- `fixtures/survey-logic/` — 后端测试与前端（经 wasm 引擎）共用人工预期的表达式、文档、流程、最终提交、文本及输入契约样例。
- `frontend/` — Vue 3、TypeScript、Vite、Vue Router、Pinia；`prototype.html` 与 `fluent-prototype.html` 保留为独立风格原型（后者对应当前 Fluent 2 视觉），`src/` 为真实应用。
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
| `logic-core/src/domain.rs` | `questionnaire-logic`：pure survey document model + validation；`backend/src/lib.rs` 用 `pub use questionnaire_logic::domain;` 接入 |
| `logic-core/src/identity.rs` | 配置与作答题目 UUID 的严格反序列化；与答案值的规范化分开 |
| `logic-core/src/logic.rs` | 类型化表达式编译与求值、三值逻辑、数值及资源预算 |
| `logic-core/src/logic_document.rs` | 变量依赖排序、题目规则、身份引用重写 |
| `logic-core/src/flow.rs` | 前向流程编译、会话、改答清理与提交路径重建 |
| `logic-core/src/logic_text.rs` | 解析资源预检、pest Pratt 优先级、身份绑定与统一 AST 构造，再调用文档编译器；`format_logic_text` 的唯一实现 |
| `logic-core/src/logic_text.pest` | 无递归词法入口及完整 DSL 文法；语法匹配与业务语义分开 |
| `logic-wasm/src/lib.rs` | JSON 字符串信封边界：`parse_logic_text` / `format_logic_text` / `compile_document_logic` / `compile_expression` / `renew_document_ids` / `evaluate_expression` / `evaluate_document_logic` / `FlowSession`（仅 wasm32 导出） |
| `entities.rs` | 6 SeaORM entity modules |
| `repository.rs` | ownership lookups + row locking rules |
| `migration.rs` + `schema.sql` | migration entry + authoritative DDL |
| `surveys.rs` | owner survey lifecycle, drafts, publish/close/archive |
| `submissions.rs` | public fill surface + idempotent submit |
| `reports.rs` | response list/detail, statistics, CSV export |
| `admin.rs` | governance (users/surveys) + audit writes |

## Development Commands

以下命令从仓库根目录运行（PowerShell），按实际需要选择，不是每次改动或提交都要执行的清单。

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
cargo build --release --locked -p questionnaire-api          # single codegen unit + fat LTO + strip
Set-Location frontend; pnpm run wasm:build   # 改动 logic-core/ 或 logic-wasm/ 后重建 wasm 产物并同步到 src/wasm/pkg
```

- `pnpm run wasm:build` 需要 Rust 工具链 + `wasm32-unknown-unknown` target + wasm-pack 0.15；只有改动 `logic-core/` 或 `logic-wasm/` 时才需要执行。产物（胶水、导入表、`.wasm`、两份类型声明）提交在 `frontend/src/wasm/pkg/`，不装 Rust 的前端开发者直接用入库产物。

- `--locked` is the documented norm; the root `Cargo.lock` is authoritative.
- The server needs a reachable PostgreSQL; config/DB errors abort startup (no in-memory fallback, no DB auto-create). Migrations always run at startup, including before subcommand dispatch.
- Compile-speed profiles live in the root `Cargo.toml`; the numbers behind the choices are in README §7.1 and `docs/requirements-analysis.md` §16.

### Toolchain and profile: pick per need

两条路径共用 profile。默认使用 stable + LLVM；Cranelift 仅供开发迭代，正式验证与发布仍用 stable + LLVM。是否需要验证由本次改动决定。

| Need | Command | Cost / benefit |
|---|---|---|
| Default dev build & run | `cargo build\|run --locked -p questionnaire-api` | stable + LLVM; own code `opt-level = 0`, deps `opt-level = 3`, debuginfo kept. Use it when you need breakpoints or want to judge dependency runtime speed. |
| Fast logic iteration | `cargo +nightly cranelift -p questionnaire-api` | alias defined in `.cargo/config.toml`; needs nightly + `rustc-codegen-cranelift-preview` (installed on this machine). Measured: one-crate incremental 8—12 s vs 17—28 s with LLVM (~2—3×); a full cold build gains nothing (dependency compilation dominates). Generated machine code is slower than LLVM — never ship it. |
| Run / test under Cranelift | `cargo +nightly run --locked -Zcodegen-backend --config .cargo/cranelift.toml -p questionnaire-api`<br>`cargo +nightly test -Zcodegen-backend --config .cargo/cranelift.toml --locked -p questionnaire-api --test domain` | the alias only wraps `build`; `.cargo/cranelift.toml` sets `codegen-backend` for the dev and test profiles only. |
| Rust 检查（按改动选择） | 对应的 `cargo test --locked -p questionnaire-api --test <目标>`；格式与 Clippy 按需执行 | 仅覆盖受影响的 Rust 代码，使用 stable + LLVM；不是提交门禁，前端或文档改动不触发。 |
| Release artifact | `cargo build --release --locked -p questionnaire-api` | `opt-level = 3` + `codegen-units = 1` + `lto = "fat"` + `strip = "symbols"` + `panic = "abort"`; ≈5 min cold, 6.87 MB here. Never build release with Cranelift. |

- 不新增 `rust-toolchain.toml`：stable 保持默认，按需检查与发布均使用 LLVM，并遵守已声明的 MSRV。
- The first Cranelift build in a given `target/` rebuilds the whole dependency tree (measured 222 crates, 6m07s); afterwards cargo keeps both artifact sets and switching back is instant (measured 0 crates, ~2 s). `cargo clean`, a fresh clone or a different `--target-dir` pays that cost again.
- `TMP`/`TEMP` are pinned to the workspace-relative `target/` in `.cargo/config.toml`: with an unwritable or non-existent temp dir MSVC `cl.exe` fails with `error D8050` while compiling the C dependency `ring`.
- No Makefile / justfile / root scripts / CI — call cargo directly. The only npm script that touches Rust is `frontend/` 的 `wasm:build`（见上）。

## Code Conventions & Common Patterns

- **Chinese output**: `//!`/`///` docs, error messages, tests and docs are Chinese. Start each file with a `//!` line stating the invariant it defends (e.g. `repository.rs`: lock user before survey).
- **Visibility**: modules are private; items crossing modules are `pub(crate)`. Only the re-export `pub use questionnaire_logic::domain;` and `pub async fn run()` are public.
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
- `logic-core/src/domain.rs` — 问卷模型与校验，也是纯领域 DSL/流程接口的公共出口；实现模块保持私有，后端经 `backend/src/lib.rs` 的再导出使用。
- `backend/src/http.rs`, `backend/src/config.rs`, `backend/src/security.rs` — boundary concerns (extractors, env contract, request gating).
- `backend/Cargo.toml` — MSRV/edition/features; `Cargo.lock` — exact pins.
- `Cargo.toml` (root) `[profile.*]` + `.cargo/config.toml` / `.cargo/cranelift.toml` — compile-speed tuning (dev/release), the opt-in Cranelift alias, and the child-process `TMP`/`TEMP` pin.
- `backend/tests/` — 纯领域回归，按文档、表达式/规则、流程、文本入口分文件。
- `compose.yaml`, `.env.example` — optional DB and the env contract.
- `README.md`、`docs/backend-guide.md`、`docs/frontend-guide.md`、`docs/requirements-analysis.md` — 运行说明、前后端代码指南与分阶段验证记录。前端 MVP 已实现，性能目标仍未经过压测；以当前代码和验证记录为准。

## 前端 MVP 约定

- 视觉基线是 2026-09-30 起的 Fluent 2 方向：控件 4px、卡片 8px、弹窗 12px 圆角，分级阴影，带轻微色调的 Mica 基底与磁贴渐变，移动导航在浏览器支持时使用 `backdrop-filter` 亚克力；主题仍是浅色、深色与跟随系统。以 `frontend/src/styles.css` 令牌和 `frontend/fluent-prototype.html` 为准，早期「直角、纯色、不用渐变/投影/毛玻璃」的说法已作废，不要再据此改样式。
- 在 `frontend/` 运行 `pnpm install --frozen-lockfile`、`pnpm run dev`；必要构建使用 `pnpm run build`。开发和预览固定使用 `http://localhost:5173`，后端 `PUBLIC_ORIGIN` 必须匹配。
- 前端包管理器固定为 pnpm：版本的唯一来源是 `frontend/package.json` 的 `packageManager` 字段，锁文件是 `frontend/pnpm-lock.yaml`；不要用 npm/yarn/bun，不要重新生成 `package-lock.json`，依赖变更后用 pnpm 更新锁文件。
- `/api` 经 Vite 同源代理访问后端，目标由服务器变量 `API_PROXY_TARGET` 配置；保留 Cookie 和 Origin 校验，不将秘密放入浏览器变量。
- DSL 引擎只有一份实现：`logic-core`（`questionnaire-logic`）编到 wasm 后由 `frontend/src/wasm/index.ts` 包装调用，前端不再有表达式/文档/流程/文本的 TS 镜像。`frontend/src/wasm/pkg/` 的五件套产物入库，改动 `logic-core/` 或 `logic-wasm/` 后在 `frontend/` 运行 `pnpm run wasm:build` 重建；只改前端时无需 Rust 工具链。
- 用户要求按 MVP 范围推进，只做必要验证，不主动扩展测试工程或异常场景联调；静态原型无需测试。

## Runtime/Tooling Preferences

- Rust stable **≥ 1.94**, edition 2024 (verified locally on 1.98.1). No `rust-toolchain.toml`; stable stays the default active toolchain. Nightly is installed only for the opt-in Cranelift path — see "Toolchain and profile: pick per need".
- Cargo workspace, three members: `backend` (`questionnaire-api`), `logic-core` (`questionnaire-logic`) and `logic-wasm`; each member owns its dependencies in its own `Cargo.toml` (no workspace dependency inheritance, no `[features]`, `[dev-dependencies]` or `[lints]` sections). All three set `publish = false`; `logic-wasm` additionally carries `[package.metadata.wasm-pack]` (build-tuning only). Build-tuning `[profile.*]` sections are the only exception and live in the **root** `Cargo.toml`, never in a member manifest.
- 当前核心锁定版本：axum 0.8.9、sea-orm / sea-orm-migration 2.0.4、sqlx 0.9.0、tokio 1.53.2、utoipa 6.0.0 / utoipa-axum 0.3.0、cookie 0.18.2、pest / pest_derive 2.9.2、argon2 0.6.0、sha2 0.11.0；准确依赖图以根 `Cargo.lock` 为准。
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

- Rust 格式与静态检查使用默认 rustfmt / clippy；Clippy 按需运行并使用 `-D warnings`，不与测试、构建捆绑成固定提交门禁。
- No CI workflows, Dockerfile, task runners or scripts exist. Do not assume one.

## Testing & QA

- 先按 [README 第 7 节](README.md#7-开发检查)选择与改动对应的最小检查。没有统一提交门禁；同一份代码已有的通过结果直接复用，commit、改分支或整理文档不触发重跑。纯前端不跑 Rust，纯后端不跑前端；只有共享契约变更才按影响验证两端。
- 仅调整测试时运行受影响的测试即可，不附带生产构建、Clippy 或服务联调。前端构建已包含类型检查，不再单独重复 typecheck；格式化只处理改动文件。
- 使用内置 `cargo test`，四个独立目标：`domain` 覆盖题型、身份、规范化、文本/数字边界与提交预算；`logic` 覆盖表达式及文档依赖；`flow` 覆盖拓扑、改答清理、最终提交及配置身份；`logic_text` 覆盖文本解析。另有 `logic-wasm/tests/smoke.rs` 在原生目标覆盖同一 JSON 信封边界（随 `--workspace` 一起执行）。前端以 Node 内置运行器执行 `frontend/tests/`。具体通过数量和真实服务验证记录在 `docs/requirements-analysis.md`，不把历史数量当作当前测试清单。
- 测试围绕当前业务契约与已确认缺陷；已有覆盖足够时不新增，不按文件、函数或覆盖率配额补测试，不锁定私有结构、元数据排列等实现细节。相同行为优先保留一处回归；共享语义样例仍由后端测试与前端（经 wasm 引擎）两侧各读一次，用于确认边界两端的语义一致。新测试名、断言说明与样例说明使用中文，Rust 测试放 `backend/tests/`（wasm 边界放 `logic-wasm/tests/`）；不新增历史答卷兼容专项测试。
- 文本空白统一按 Unicode `White_Space`（Rust `str::trim`）：U+0085 是空白，U+FEFF 不是；非空白文本保留原文。配置的题目、选项、变量及流程身份与引用只接受非零、小写连字符 UUID；答案中的选项值仍由规范化入口处理。
- 提交答案和改答事件的 `question_id` 同样必须使用规范题目身份；不能把选项答案值的宽松 UUID 解析复用到身份字段。
- `cargo test --locked --workspace` needs **no database and no env vars**. Useful filters: `cargo test -p questionnaire-api --test domain <name_substring>`.
- Style: build input via the existing helpers (`id`, `question`, `choices`, `document`, `answer`); assert on `Result` and `question_id`, never on message wording; parse raw JSON strings with `serde_json::from_str` when digit precision matters.
- 自动化测试保持在纯领域逻辑与必要输入适配范围。领域行为变更优先复用相关测试，只为未覆盖的实际风险补回归。HTTP/auth/SQL 行为变更只核对受影响的真实服务路径，不增设 ORM Mock 或无数据库的接线测试；既有服务验证记录见 `docs/requirements-analysis.md`。
- There is no coverage tooling, benchmark or mock framework, and no `[dev-dependencies]` section in any member manifest; adding test-only crates requires creating one.
