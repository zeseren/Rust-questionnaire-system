# Repository Guidelines

> 语言约定：源码注释（`//!` / `///`）、错误消息、测试与 `docs/` 一律使用中文；新增内容保持中文，本文件同样以中文维护。

## Project Overview

当前开发前提：项目尚无正式发布版本，不需要考虑历史答卷兼容。后续设计和实现可以直接调整当前模型、DSL 与数据结构，不为旧格式、历史答卷或旧版本增加兼容分支、迁移方案和回归负担；既有文档中的历史兼容描述是当时的实现记录，不作为当前开发约束，仍按当前契约保留必要的回归验证。

`questionnaire-api` 是问卷调查系统的 Rust HTTP JSON API，`frontend/` 是连接真实接口的 Vue 3 前端 MVP。后端 `share_url` 指向公开 JSON 接口，前端填写页面为 `/s/:id`。

业务范围：发布者用六类题型（单选、多选、单行文本、多行文本、数字、评分）创建问卷，发布不可变版本，匿名收集答卷或按账号跨版本限交一次，然后查看分版本统计并导出 CSV；管理员管理用户与问卷并留下审计记录。

项目根目录是三个成员的 Cargo workspace：

- `backend/`（包名 `questionnaire-api`）：HTTP API 与业务用例。
- `logic-core/`（包名 `questionnaire-logic`）：纯领域 DSL 核心，包含表达式、文档规则、流程与 pest 文本文法；无 HTTP/DB/文件系统依赖，可编译到 wasm32。
- `logic-wasm/`（包名 `logic-wasm`）：把领域核心以 JSON 字符串信封导出给前端。

`frontend/` 是独立的 Vue 3 应用，规则引擎直接调用入库的 wasm 产物，不再保留 TypeScript 镜像实现。使用与阅读说明见 `README.md`、`docs/backend-guide.md`、`docs/frontend-guide.md`，DSL 规范见 `docs/survey-dsl-plan.md`，需求与架构演进/验证记录见 `docs/requirements-analysis.md`（第 9 节）。

## Architecture & Data Flow

严格三层：HTTP 边界 → 业务用例 → 数据访问 → PostgreSQL。

- `main.rs` 只调用 `questionnaire_api::run()`（`lib.rs`）。`run()` 是组合根：dotenv → tracing（读 `RUST_LOG`，默认 `questionnaire_api=info`）→ `Config::from_env()` → sea-orm 连接池（最多 10 条连接）→ `migration::Migrator::up` **每次启动都执行** → argv 子命令（默认 `serve`，另有 `migrate`、`create-admin`）→ `TcpListener` → `axum::serve(...).with_graceful_shutdown(ctrl_c)`。
- 全局只有一份 `AppState { db: DatabaseConnection, config: Arc<Config>, limits: Arc<Limits> }`（`lib.rs`），通过 `.with_state(state)` 挂载一次。
- 各业务模块暴露 `pub(crate) fn router() -> OpenApiRouter<AppState>`，用 `utoipa_axum::routes!` 从 handler 的 `#[utoipa::path]` 同时登记路由与文档，不另写 `ApiDoc` 列表。`lib.rs::router()` 合并五个业务模块与 `/health`，再用 `split_for_parts()` 取出路由和文档；文档只在组装时生成，`/api/openapi.json` 返回其副本。随后配置 404/405、`DefaultBodyLimit::max(domain::MAX_SUBMISSION_BYTES)`、`security::guard` 与共享状态；不提供 Swagger UI。结构不同的 DTO 使用不同 Schema 名称，尤其是发布者的 `SurveyPage` 与治理的 `AdminSurveyPage`。
- handler 保持很薄：先抽取（`State` → `AuthUser` → `HeaderMap` → `ApiPath`/`ApiQuery`/`ApiJson`），再调用接收 `&AppState` 与所有者 id 的业务函数，返回 `AppResult<Json<T>>` 或 `AppResult<(StatusCode, Json<T>)>`。handler 与它的业务函数写在同一个文件里；只有跨模块的数据访问规则放进 `repository.rs`。
- `logic-core/src/domain.rs` 是纯领域代码（无 HTTP、无 DB）；`backend/src/lib.rs` 用 `pub use questionnaire_logic::domain;` 再导出，因此业务代码与测试里的 `questionnaire_api::domain::*` 路径不变。它包含 `Document`、`Question`、`QuestionKind`、`Answer`、`validate_draft`、`validate_publish`、`validate_answers`、`renew_ids` 以及数字规范化。`ValidationError { question_id: Option<Uuid>, message }` 以 HTTP 422 暴露。
- 持久化：SeaORM entity 位于 `entities.rs`（表 `users`、`sessions`、`surveys`、`versions`、`responses`、`audits`），共享的归属查询与锁助手位于 `repository.rs`。写路径 `state.db.begin()` 后先锁发布者 `user`（Share / NoKeyUpdate），**再**锁 `survey`（Update），改完 `tx.commit()`。只读的列表与统计路径使用 `RepeatableRead` + `ReadOnly`。
- `admin.rs`：治理问卷列表与用户/审计列表都用 `RepeatableRead` + `ReadOnly` 只读快照。治理问卷列表通过 JSONB 投影只取 `title`，并复用 `SurveyStatus` 与 `effective_status`，不逐行反序列化整份 `Document`。
- 建表：`backend/src/schema.sql` 是唯一的 DDL 来源——原生 PostgreSQL，用 `include_str!` 嵌入并由 `migration.rs` 里那一个迁移执行。改表结构只改这里，不要再写第二份。
- 错误：统一用 `AppError { status, code: &'static str, message, question_id, error_id }`（`error.rs`），响应体为 `{"error":{"code":"...","message":"...","question_id":null}}`；`From<DbErr|sqlx::Error|serde_json::Error>` 一律转成 500 且不泄露 ORM 文本。`internal()` 生成 16 位十六进制 `error_id`（同时作为 `X-Error-Id` 返回），并用 `#[track_caller]` 记录 `error_id` / `error_kind` / `location`；别名 `AppResult<T>`。
- 安全：只用 Cookie Session（不透明 32 字节十六进制 token，数据库存的是 SHA-256 十六进制 `sessions.id_hash`）；Argon2id 密码哈希在 `spawn_blocking` 中执行。`security::guard` 要求每个非 GET/HEAD/OPTIONS 请求的 `Origin` 精确等于 `PUBLIC_ORIGIN` **且**带 `X-CSRF-Protection: 1`，同时执行两个固定窗口限流（`/api/auth/` 写请求、公开答卷 POST），并设置 `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff`。按设计不加 CORS 层。
- Cookie 由 `cookie::Cookie::split_parse` / `Cookie::build` 解析与构造：遍历所有 Cookie 头并拒绝重复 session，不通过 CookieJar 覆盖同名项，也不做百分号解码。Argon2 0.6 为每次哈希自动生成随机盐；session 的 32 字节随机值来自 `getrandom::fill`。

### 请求追踪示例

`POST /api/surveys/{id}/publish`：`surveys::publish` → `publish_survey(&state, owner_id, id)` → `repository::lock_owned_survey`（user Share → survey Update，并校验归属）→ `repository::expire_survey` → `ensure_editable` + `ensure_revision`（草稿修订过期返回 409）→ `Document::deserialize` → `document.validate_publish()` → `mode` 非空检查（未设置返回 409）→ 幂等重放判断：草稿内容与 `current_version_id` 指向的版本完全一致时返回 `200 OK` 和当前版本；否则插入 `versions` 行（`number = max + 1`）并更新 `surveys.current_version_id` → `tx.commit()` → `201 Created`。

校验在幂等重放判断**之前**执行：无论这版内容是否与当前版本相同，都必须先满足当前发布规则。

### 公开填写

- `GET /api/public/surveys/{id}`（`submissions::read_public_survey`）使用 `RepeatableRead` + `ReadOnly` 只读快照，一次事务内检查回收状态、发布者状态与登录限次资格，不持有 `FOR UPDATE` 写锁，也不与回收写入争抢 `users` / `surveys` 行锁。只有 `login_once` 模式才认证并解析 Cookie，匿名与未设置模式的路径**绝不**解释 Cookie。
- `POST /api/public/surveys/{id}/responses` 走写路径，通过 `lock_public_survey` 遵循既有的「发布者 `FOR SHARE` → 问卷 `FOR UPDATE`」锁序，并在同一事务内重新验证 session（等待锁期间可能发生退出、改密或禁用）。锁定前只读取归属与模式以决定是否认证；锁后发现 `owner_id` 或 `mode` 已变化则返回 `collection_changed`。

## Key Directories

- `backend/src/` — HTTP 与业务源码，扁平结构（每个功能域一个文件，不分子目录）；DSL 实现已不在其中。
- `backend/tests/` — 独立回归目标：`domain.rs`、`logic.rs`、`flow.rs`、`logic_text.rs`；业务源码中不放测试。
- `logic-core/src/` — `questionnaire-logic` 的 DSL 六文件（`domain.rs`、`identity.rs`、`logic.rs`、`logic_document.rs`、`logic_text.rs` + `logic_text.pest`、`flow.rs`）；纯领域逻辑，对外只导出 `domain`（聚合再导出）与 `identity`。
- `logic-wasm/src/lib.rs` + `logic-wasm/tests/smoke.rs` — wasm 边界（入参出参都是 JSON 字符串信封）；冒烟测试在原生目标调用同一份函数体。
- `frontend/src/wasm/` — `index.ts` 包装 + 入库的 `pkg/` 产物五件套（胶水、导入表、`.wasm` 与两份类型声明）；不装 Rust 工具链也能直接使用。
- `fixtures/survey-logic/` — 后端测试与前端（经 wasm 引擎）共用的人工预期样例：`expressions.json`、`documents.json`、`flows.json`、`submissions.json`、`texts.json` 与 `input-contracts.json`。
- `frontend/` — Vue 3、TypeScript、Vite、Vue Router、Pinia；`prototype.html`、`fluent-prototype.html`、`editorial-prototype.html` 是独立风格原型（`fluent-prototype.html` 对应当前视觉），`src/` 才是真实应用。
- `docs/` — 中文文档：`backend-guide.md`、`frontend-guide.md`、`survey-dsl-plan.md`、`requirements-analysis.md`。
- 仓库根 — `Cargo.toml` / `Cargo.lock`（workspace 与锁文件）、`compose.yaml`（可选的本机 PostgreSQL 18）、`.env.example`。

模块表（推荐的阅读顺序）：

| 文件 | 职责 |
|---|---|
| `lib.rs` | `run()`、`router()`、`AppState`、`/health`、OpenAPI 合并 |
| `main.rs` | 二进制入口，无逻辑 |
| `config.rs` | 环境变量与启动前快速校验 |
| `error.rs` | `AppError` / `AppResult` 与错误信封 |
| `http.rs` | `ApiJson` / `ApiQuery` / `ApiPath` 抽取器、`Pagination` |
| `security.rs` | CSRF、限流与安全响应头中间件 |
| `auth.rs` | `AuthUser` 抽取器、Argon2、会话与 Cookie 装配 |
| `logic-core/src/domain.rs` | `questionnaire-logic` 的问卷文档模型与校验；后端经 `backend/src/lib.rs` 的再导出接入 |
| `logic-core/src/identity.rs` | 配置、作答及选项身份的严格反序列化：只接受非零、小写连字符 UUID |
| `logic-core/src/logic.rs` | 类型化表达式编译与求值、三值逻辑、数值及资源预算 |
| `logic-core/src/logic_document.rs` | 变量依赖排序、题目规则、身份引用重写 |
| `logic-core/src/flow.rs` | 前向流程编译、会话、改答清理与提交路径重建 |
| `logic-core/src/logic_text.rs` | 解析资源预检、pest Pratt 优先级、身份绑定与统一 AST 构造，再调用文档编译器；`format_logic_text` 的唯一实现 |
| `logic-core/src/logic_text.pest` | 无递归词法入口与完整 DSL 文法；语法匹配与业务语义分开 |
| `logic-wasm/src/lib.rs` | JSON 字符串信封边界：`parse_logic_text` / `format_logic_text` / `compile_document_logic` / `compile_expression` / `renew_document_ids` / `evaluate_expression` / `evaluate_document_logic` / `FlowSession`（仅 wasm32 导出） |
| `entities.rs` | 6 个 SeaORM entity 模块 |
| `repository.rs` | 归属查询与行锁规则 |
| `migration.rs` + `schema.sql` | 迁移入口 + 唯一权威 DDL |
| `surveys.rs` | 发布者侧问卷生命周期、草稿、发布/关闭/归档 |
| `submissions.rs` | 公开填写面与幂等提交 |
| `reports.rs` | 答卷列表/详情、统计、CSV 导出 |
| `admin.rs` | 治理（用户/问卷）与审计写入 |

## Development Commands

以下命令从仓库根目录运行（PowerShell），按实际需要选择，不是每次改动或提交都要执行的清单。

```powershell
Copy-Item .env.example .env          # 然后修改 DATABASE_URL
docker compose up -d db              # 可选的本机 PostgreSQL 18；需要 $env:POSTGRES_PASSWORD
cargo run --locked -p questionnaire-api -- migrate          # 执行迁移后退出
cargo run --locked -p questionnaire-api                     # 默认 serve，监听 127.0.0.1:3000
cargo run --locked -p questionnaire-api -- create-admin     # 读取 ADMIN_USERNAME / ADMIN_PASSWORD
Invoke-RestMethod http://localhost:3000/health              # {"status":"ready"}
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --release --locked -p questionnaire-api          # 单 codegen 单元 + fat LTO + strip
Set-Location frontend; pnpm run wasm:build   # 改动 logic-core/ 或 logic-wasm/ 后重建 wasm 产物并同步到 src/wasm/pkg
```

- `pnpm run wasm:build` 需要 Rust 工具链 + `wasm32-unknown-unknown` target + wasm-pack 0.15；只在改动 `logic-core/` 或 `logic-wasm/` 时需要执行。产物五件套（胶水、导入表、`.wasm`、两份类型声明）提交在 `frontend/src/wasm/pkg/`，不装 Rust 的前端开发者直接用入库产物。
- `--locked` 是文档规定的默认做法；根 `Cargo.lock` 是版本权威来源。
- 服务需要可达的 PostgreSQL；配置或数据库错误会中止启动（没有内存回退，也不自动建库）。迁移在启动时总会先跑，包括子命令分发之前。
- 编译速度相关 profile 在根 `Cargo.toml`；取舍背后的数据在 README 第 7.1 节与 `docs/requirements-analysis.md` 第 9.4 节。

### Toolchain and profile: pick per need

两条路径共用 profile。默认使用 stable + LLVM；Cranelift 仅供开发迭代，正式验证与发布仍用 stable + LLVM。是否需要验证由本次改动决定。

| 需求 | 命令 | 代价 / 收益 |
|---|---|---|
| 默认开发构建与运行 | `cargo build\|run --locked -p questionnaire-api` | stable + LLVM；自己的代码 `opt-level = 0`、依赖 `opt-level = 3`，保留 debuginfo。需要断点或想评估依赖运行速度时用它。 |
| 快速逻辑迭代 | `cargo +nightly cranelift -p questionnaire-api` | 别名定义在 `.cargo/config.toml`；需要 nightly + `rustc-codegen-cranelift-preview`（本机已安装）。实测单 crate 增量 8—12 秒，LLVM 为 17—28 秒（约 2—3 倍）；全量冷构建没有收益，因为耗时主要由依赖编译决定。生成的机器码比 LLVM 慢，**不要发布它**。 |
| 在 Cranelift 下运行 / 测试 | `cargo +nightly run --locked -Zcodegen-backend --config .cargo/cranelift.toml -p questionnaire-api`<br>`cargo +nightly test -Zcodegen-backend --config .cargo/cranelift.toml --locked -p questionnaire-api --test domain` | 别名只包装 `build`；`.cargo/cranelift.toml` 只为 dev 与 test profile 设置 `codegen-backend`。 |
| Rust 检查门禁 | 受影响的 `cargo test` + `cargo clippy --locked --workspace --all-targets -- -D warnings` + 改动文件 `rustfmt` | 改动 Rust 代码时的必跑门禁（使用 stable + LLVM）；纯前端或纯文档改动不触发。 |
| 发布产物 | `cargo build --release --locked -p questionnaire-api` | `opt-level = 3` + `codegen-units = 1` + `lto = "fat"` + `strip = "symbols"` + `panic = "abort"`；本机冷构建约 5 分钟，产物 6.87 MB。不要用 Cranelift 做 release。 |

- 不新增 `rust-toolchain.toml`：stable 保持默认，按需检查与发布都走 LLVM，并遵守已声明的 MSRV。
- 某个 `target/` 目录里第一次用 Cranelift 会重建整棵依赖树（实测 222 个 crate、6 分 07 秒）；之后 cargo 保留两套产物，切回 stable 几乎瞬间完成（实测重编 0 个 crate、约 2 秒）。`cargo clean`、全新克隆或换 `--target-dir` 都要重新付这份成本。
- `.cargo/config.toml` 把 `TMP`/`TEMP` 固定到工作区相对的 `target/`：临时目录不可写或不存在时，MSVC `cl.exe` 编译 C 依赖 `ring` 会报 `error D8050`。
- `.cargo/config.toml` 还为 `wasm32-unknown-unknown` 设置 `--cfg getrandom_backend="wasm_js"`，否则 `uuid` v4 依赖的 getrandom 无法编到该目标。
- 没有 Makefile / justfile / 根脚本 / CI，直接用 cargo 调用。唯一涉及 Rust 的 npm 脚本是 `frontend/` 的 `wasm:build`（见上）。

## Code Conventions & Common Patterns

- **中文输出**：`//!` / `///` 文档、错误消息、测试与 `docs/` 都是中文。每个文件以一行 `//!` 开头说明它守护的不变量（例如 `repository.rs`：先锁 user 再锁 survey）。
- **可见性**：模块私有；跨模块的条目用 `pub(crate)`。对外公开的只有再导出 `pub use questionnaire_logic::domain;` 和 `pub async fn run()`。
- **错误**：不要临时拼元组，统一用 `AppError::new|validation|field|conflict|not_found|unauthorized|forbidden|internal`。错误码是稳定的 snake_case（`csrf_rejected`、`revision_conflict`、`idempotency_conflict`、`rate_limited`、`not_ready` 等）。绝不向客户端或日志暴露 ORM/SQL 文本。`anyhow` 只用在启动阶段（`.context(...)`）。
- **DTO**：请求用 `#[derive(Deserialize, ToSchema)] #[serde(deny_unknown_fields)]`；枚举用 `#[serde(rename_all = "snake_case")]`；`QuestionKind` 与统计使用内部标签 `#[serde(tag = "type")]`。响应用 DTO + `impl From<entity::Model>`，绝不直接序列化 SeaORM 模型。
- **抽取器**：用 `http.rs` 的 `ApiJson` / `ApiQuery` / `ApiPath` newtype，让解析失败变成 `AppError`（400/413）。列表统一走 `Pagination::bounds()`（page ≥ 1，page_size ≤ 100，默认 20）。
- **持久化**：SeaORM builder 风格；只有 PostgreSQL 特有需求才写原生 SQL，用 `Expr::cust_with_values` / sqlx `query_as` 并**绑定参数**（绝不字符串拼接）。`begin()` 之后一直在 `&tx` 上操作。读 user 用 `LockType::Share`，写 user 用 `NoKeyUpdate`，survey 用 `Update`。
- **并发**：只有一种锁序——先发布者 user，再 survey——集中在 `repository.rs`（`lock_user`、`lock_survey`、`lock_owned_survey`）。填写资格检查走 `ensure_collecting` / `expire_survey`；不要发明第二种锁序，也不要在事务外重复检查。
- **密钥**：密码用 Argon2id（`Argon2::default()`，每次哈希独立盐）并在 `spawn_blocking` 内执行；session token 与提交幂等键用 SHA-256 十六进制（`hash_secret`）。携带密钥的结构体绝不派生 `Debug`。
- **ID 与时间**：新 id 用 `Uuid::new_v4()`；时间用 `chrono::Utc::now()` / `DateTime<Utc>`；API 时间戳是 RFC3339，时间范围是半开区间 `[from, to)`。
- **数字**：前后端统一使用标准 JSON 数字与有限 `f64`（JS `number`）；`1` 与 `1.0`、正零与负零视为同一答案，负零归一为 `0.0`。
- **响应头**：新增响应头时要保留现有的 `no-store` / `nosniff` 行为。

### 边界与反过度防御原则（禁止损伤性能与引入冗余）

核心准则：**信守边界（Parse, don't re-validate）**——只在最外层边界（HTTP 抽取器、WASM 字符串信封、数据库约束）做一次性解析与校验，内部模块直接信任已确立的不变量，绝不层层重复设防。

- **禁止在只读路径加排他写锁或开可写事务**：只读查询（如公开读题面 `GET /api/public/surveys/{id}`、问卷/答卷/治理分页列表、统计与导出）一律使用 `RepeatableRead + ReadOnly` 快照或普通只读查询，绝不复用写路径的 `FOR UPDATE` / `FOR SHARE` 行锁阻塞正常提交。列表查询若只需标题等摘要字段，用 SQL/JSONB 字段投影（如 `cast_json_field("title")`），禁止为取单个字段把整份 `Document` JSONB 反序列化。
- **禁止对已编译/已校验结构回头解析原始 JSON 或重复遍历**：表达式、文档规则与流程一旦经 `CompilationContext` / `DocumentLogic::compile` 编译为类型化 AST（`CompiledExpression` / `Node`），下游分析（如必读依赖 `required_reads`、常量分支检查、求值）必须直接在类型化节点或预建索引（如 `BTreeMap<Uuid, &Question>`）上运行；禁止再回头读取原始 `&Value`、重复 `Uuid::parse_str`、重复跑 `check_raw_budget` 或重复编译同一份文档。
- **禁止用静默兜底掩盖内部不变量违约**：编译期或上游已保证存在的数据（如流程页内题目、已声明变量、已知枚举状态），直接按类型或索引访问；禁止写 `unwrap_or("")`、`unwrap_or_default()`、`else { continue }` 或在 `catch` 中伪造领域语法错误（如 `LogicError('invalid_syntax')`）来吞掉不可能发生的分支。
- **禁止无意义的深拷贝与双次序列化**：
  - Rust / WASM 边界使用泛型 `Envelope<T>` 单遍 `serde_json::to_string` 序列化，禁止先 `serde_json::to_value` 构造中间树再 `.to_string()`。
  - 求值器与状态机优先借用或按需就地更新（如 `FlowEvent::Change` 就地更新输入、无规则报表在统计/CSV 中不分配状态表），禁止为“防意外修改”预先克隆整张变量表或答案表。
  - 前端禁止对 `api()` 刚解码出的新鲜对象、即将传给 `api()` 同步序列化的请求体、或只读消费的草稿/答案表反复调用 `copy()`（`encode` + `decode` 全量 JSON 往返）。
- **禁止保留历史兼容后门与多份真相**：项目无历史版本兼容负担，校验与幂等重放保持单一正常顺序（先校验发布合法性，再判同内容重放）；状态派生（`effective_status`）、枚举解析（`SurveyStatus` / `FillingMode`）与错误文案提取（`message()`）集中在一处复用，不在多处手写副本。

## Important Files

- `backend/src/lib.rs` — 组合根：`run()`、`router()`、`AppState`、健康检查、OpenAPI 合并。先读这里。
- `backend/src/error.rs` — 所有返回共享的错误契约。
- `backend/src/repository.rs` — 每个写路径都要遵守的锁序与归属不变量。
- `logic-core/src/domain.rs` — 问卷模型与校验，也是纯领域 DSL/流程接口的公共出口；实现模块保持私有，后端经 `backend/src/lib.rs` 的再导出使用。
- `backend/src/http.rs`、`backend/src/config.rs`、`backend/src/security.rs` — 边界关注点（抽取器、环境契约、请求准入）。
- `backend/Cargo.toml` — MSRV / edition / 依赖；`Cargo.lock` — 精确版本。
- 根 `Cargo.toml` 的 `[profile.*]` + `.cargo/config.toml` / `.cargo/cranelift.toml` — 编译速度调优（dev/release）、可选的 Cranelift 别名、子进程 `TMP`/`TEMP` 固定与 wasm32 rustflags。
- `backend/tests/` — 纯领域回归，按文档、表达式/规则、流程、文本入口分文件。
- `compose.yaml`、`.env.example` — 可选数据库与环境契约。
- `README.md`、`docs/backend-guide.md`、`docs/frontend-guide.md`、`docs/survey-dsl-plan.md`、`docs/requirements-analysis.md` — 运行说明、前后端代码指南、DSL 实施计划与分阶段验证记录。前端 MVP 已实现，性能目标仍未经过压测；以当前代码和验证记录为准。

## 前端 MVP 约定

- 视觉基线是 2026-09-30 起的 Fluent 2 方向：控件 4px、卡片 8px、弹窗 12px 圆角，分级阴影，带轻微色调的 Mica 基底与磁贴渐变，移动导航在浏览器支持时使用 `backdrop-filter` 亚克力；主题仍是浅色、深色与跟随系统。以 `frontend/src/styles.css` 令牌和 `frontend/fluent-prototype.html` 为准，早期「直角、纯色、不用渐变/投影/毛玻璃」的说法已作废，不要再据此改样式。
- 在 `frontend/` 运行 `pnpm install --frozen-lockfile`、`pnpm run dev`；必要构建使用 `pnpm run build`。开发和预览固定使用 `http://localhost:5173`，后端 `PUBLIC_ORIGIN` 必须匹配。
- 前端包管理器固定为 pnpm：版本的唯一来源是 `frontend/package.json` 的 `packageManager` 字段，锁文件是 `frontend/pnpm-lock.yaml`；不要用 npm/yarn/bun，不要重新生成 `package-lock.json`，依赖变更后用 pnpm 更新锁文件。
- `/api` 经 Vite 同源代理访问后端，目标由服务器变量 `API_PROXY_TARGET` 配置；保留 Cookie 和 Origin 校验，不将秘密放入浏览器变量。
- DSL 引擎只有一份实现：`logic-core`（`questionnaire-logic`）编到 wasm 后由 `frontend/src/wasm/index.ts` 包装调用，导出 `parseLogicText`、`formatLogicText`、`compileDocumentLogic`、`compileExpression`、`renewDocumentIds` 与 `FlowSession`；模块顶层 `await loadEngine()`，导入方总能直接调用。前端不再有表达式/文档/流程/文本的 TS 镜像实现，也不再有 `ready`、`numeric` 这类空壳导出。
- `frontend/src/wasm/pkg/` 的五件套产物入库，改动 `logic-core/` 或 `logic-wasm/` 后在 `frontend/` 运行 `pnpm run wasm:build` 重建；只改前端时无需 Rust 工具链。
- 用户要求按 MVP 范围推进，只做必要验证，不主动扩展测试工程或异常场景联调；静态原型无需测试。

## Runtime/Tooling Preferences

- Rust stable **≥ 1.94**，edition 2024（本机在 1.98.1 上验证）。没有 `rust-toolchain.toml`，stable 保持默认激活工具链。nightly 只为可选的 Cranelift 路径安装——见「Toolchain and profile: pick per need」。
- Cargo workspace 含三个成员：`backend`（`questionnaire-api`）、`logic-core`（`questionnaire-logic`）与 `logic-wasm`；每个成员在自己的 `Cargo.toml` 里管理依赖（不做 workspace 依赖继承，没有 `[features]`、`[dev-dependencies]` 或 `[lints]` 段）。三者都设置 `publish = false`；`logic-wasm` 额外带 `[package.metadata.wasm-pack]`（仅构建调优）。构建调优的 `[profile.*]` 是唯一例外，只放在**根** `Cargo.toml`，绝不写进成员清单。
- 当前核心锁定版本：axum 0.8.9、sea-orm / sea-orm-migration 2.0.4、sqlx 0.9.0、tokio 1.53.2、utoipa 6.0.0 / utoipa-axum 0.3.0、cookie 0.18.2、pest / pest_derive 2.9.2、argon2 0.6.0、sha2 0.11.0；准确依赖图以根 `Cargo.lock` 为准。
- PostgreSQL 18 是运行基线；没有 SQLite/内存模式，也没有 Redis/队列。Compose 只提供数据库——API 始终通过 `cargo run` 启动。TLS 终止、反向代理与容器镜像明确不在范围内。
- 配置只来自环境变量：`dotenvy` 从仓库根读取 `.env`（进程环境变量优先；没有 `.env` 也能跑）。

| 变量 | 必需 | 默认值 | 说明 |
|---|---|---|---|
| `DATABASE_URL` | 是 | — | 必须是 `postgres://` / `postgresql://` |
| `LISTEN_ADDR` | 否 | `127.0.0.1:3000` | `IP:port` |
| `PUBLIC_ORIGIN` | 否 | `http://localhost:3000` | 裸 origin，无路径、无末尾斜杠；只有 localhost 可以用 `http` |
| `COOKIE_SECURE` | 否 | `false` | HTTPS 必须为 `true`，HTTP 必须为 `false` |
| `SESSION_TTL_SECONDS` | 否 | `86400` | 1..=2 592 000 |
| `AUTH_REQUESTS_PER_MINUTE` | 否 | `60` | 整实例固定窗口 |
| `SUBMISSIONS_PER_MINUTE` | 否 | `600` | 整实例固定窗口 |
| `RUST_LOG` | 否 | `questionnaire_api=info` | tracing `EnvFilter` |
| `ADMIN_USERNAME`、`ADMIN_PASSWORD` | 仅 `create-admin` | — | 进程环境变量，不写入 `.env.example` |
| `POSTGRES_PASSWORD` | 仅 Compose | — | 注入 `db` 服务 |

- Rust 格式与静态检查使用默认 rustfmt / clippy；修改 Rust 代码后必须运行 `cargo clippy --locked --workspace --all-targets -- -D warnings` 与改动文件 `rustfmt` 作为交付门禁。
- 没有 CI 工作流、Dockerfile、任务运行器或脚本。不要假设它们存在。

## Testing & QA

- 按 [README 第 7 节](README.md#7-开发检查)执行与改动范围对应的必跑门禁：修改 Rust 代码跑受影响的 `cargo test`、`cargo clippy --locked --workspace --all-targets -- -D warnings` 与 `rustfmt`；修改前端代码跑受影响的 `pnpm run test:regression`、`pnpm run typecheck`（或 `build`）与 `prettier`；共享契约或 `logic-core`/`logic-wasm` 变更先跑 `wasm:build` 再验证两端。
- 同一份代码已通过的门禁结果直接复用，commit、改分支或纯文档整理不触发重跑；纯前端不跑 Rust，纯后端不跑前端。
- 使用内置 `cargo test`，四个独立目标：`domain` 覆盖题型、身份、规范化与文本/数字边界；`logic` 覆盖表达式及文档依赖；`flow` 覆盖拓扑、改答清理、最终提交及配置身份；`logic_text` 覆盖文本解析。另有 `logic-wasm/tests/smoke.rs` 在原生目标覆盖同一 JSON 信封边界（随 `--workspace` 一起执行）。前端以 Node 内置运行器执行 `frontend/tests/`。具体通过数量和真实服务验证记录在 `docs/requirements-analysis.md`，不把历史数量当作当前测试清单。
- 文本答案上限：单行 `text` 最多 200、多行 `textarea` 最多 1000 个 Unicode 标量值；问卷标题最多 200，说明与收集目的各最多 5000；请求体上限为 `MAX_SUBMISSION_BYTES`（1 MiB）。
- 测试围绕当前业务契约与已确认缺陷；已有覆盖足够时不新增，不按文件、函数或覆盖率配额补测试，不锁定私有结构、元数据排列等实现细节。相同行为优先保留一处回归；共享语义样例仍由后端测试与前端（经 wasm 引擎）两侧各读一次，用于确认边界两端的语义一致。新测试名、断言说明与样例说明使用中文，Rust 测试放 `backend/tests/`（wasm 边界放 `logic-wasm/tests/`）；不新增历史答卷兼容专项测试。
- 文本空白统一按 Unicode `White_Space`（Rust `str::trim`）：U+0085 是空白，U+FEFF 不是；非空白文本保留原文。配置、作答及选项身份统一只接受非零、小写连字符 UUID。
- `cargo test --locked --workspace` **不需要数据库，也不需要环境变量**。常用过滤：`cargo test -p questionnaire-api --test domain <名称子串>`。
- 风格：用现有助手构造输入（`id`、`question`、`choices`、`document`、`answer`）；断言看 `Result` 与 `question_id`，不断言消息措辞。
- 自动化测试保持在纯领域逻辑与必要输入适配范围。领域行为变更优先复用相关测试，只为未覆盖的实际风险补回归。HTTP/auth/SQL 行为变更只核对受影响的真实服务路径，不增设 ORM Mock 或无数据库的接线测试。
- 没有覆盖率工具、基准测试或 mock 框架，任何成员清单里也没有 `[dev-dependencies]` 段；要加测试专用 crate 就得先建这个段。
