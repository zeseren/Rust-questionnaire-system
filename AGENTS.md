# Repository Guidelines

> 语言约定：源码注释（`//!` / `///`）、错误消息、测试与 `docs/` 一律使用中文；新增内容保持中文，本文件同样以中文维护。

## Project Overview

当前开发前提：项目尚无正式发布版本，不需要考虑历史答卷兼容。后续设计和实现可以直接调整当前模型、DSL 与数据结构，不为旧格式、历史答卷或旧版本增加兼容分支、迁移方案和回归负担。

`questionnaire-api` 是动态问卷调查系统的 Rust HTTP JSON API，`frontend/` 是连接真实接口的 Vue 3 前端应用。公开填写页面路由为 `/s/:id`（由前端 `shareLink` 拼接），公开读题面与元信息接口分别为 `/api/public/surveys/{id}` 与 `/api/public/surveys/{id}/meta`。

业务范围：发布者使用六类题型（单选、多选、单行文本、多行文本、数字、评分）与 DSL 规则（计算变量、显隐、动态必填、自定义约束、前向页面分支）编纂问卷，发布不可变版本，按公开匿名或同账号跨版本限交一次模式收集答卷，查看分版本三态统计并导出 CSV；管理员治理用户与问卷并记录同事务审计日志。

项目根目录是三个成员的 Cargo workspace，配合独立的 `frontend/` 目录：

- `backend/`（包名 `questionnaire-api`）：HTTP API、鉴权、事务与业务用例。
- `logic-core/`（包名 `questionnaire-logic`）：纯领域 DSL 与流程核心，包含文档模型、表达式编译求值、变量依赖排序、前向流程状态机与 pest 文本文法；无 HTTP/DB/文件系统依赖，可编译到 `wasm32-unknown-unknown`。
- `logic-wasm/`（包名 `logic-wasm`）：将 `logic-core` 以 JSON 字符串信封导出给前端。
- `frontend/`（包名 `questionnaire-web`）：Vue 3 + TypeScript + Vite 应用，通过入库的 `src/wasm/pkg/` 产物直接调用 Rust 规则引擎，无 TypeScript 镜像实现。

阅读导航见 `README.md`、`docs/backend-guide.md`、`docs/frontend-guide.md`、`docs/survey-dsl-plan.md` 与 `docs/requirements-analysis.md`。

## Architecture & Data Flow

后端严格分层：HTTP 边界 → 业务用例 → 领域核心 / 数据访问 → PostgreSQL 18。

- **组合根**：`main.rs` 仅调用 `questionnaire_api::run()`（`backend/src/lib.rs`）。`run()` 依次执行：加载 `.env` → 初始化 `tracing`（默认 `questionnaire_api=info`）→ `Config::from_env()` → 建立 SeaORM 连接池（最多 10 条连接）→ **每次启动自动执行 `migration::Migrator::up`** → 分发子命令（默认 `serve`，另有 `migrate`、`create-admin`）→ 绑定 `TcpListener` 并启动带 `ctrl_c` 优雅停机的 Axum 服务。
- **共享状态与路由登记**：全局唯一的 `AppState { db: DatabaseConnection, config: Arc<Config>, limits: Arc<Limits>, version_cache: Arc<VersionCache> }`。各业务模块暴露 `pub(crate) fn router() -> OpenApiRouter<AppState>`，使用 `utoipa_axum::routes!` 从 handler 的 `#[utoipa::path]` 同步登记路由与 OpenAPI 文档。`lib.rs::router()` 合并五个业务模块与 `/health`，通过 `split_for_parts()` 生成并预序列化 `/api/openapi.json`（每次请求只克隆字节），并挂载 404/405、默认 1 MiB `DefaultBodyLimit`（创建/保存草稿路由单独放宽至 `8 MiB`，公开提交严格保持 `domain::MAX_SUBMISSION_BYTES`）与 `security::guard`。
- **薄 Handler**：鉴权接口抽取顺序统一为 `State` → `AuthUser` → `ApiPath`/`ApiQuery`/`ApiJson`（公开/认证接口按需省略 `AuthUser`），随后调用同文件内的业务函数；跨模块的归属查询与行锁集中于 `repository.rs`。
- **单一领域核心**：`logic-core/src/domain.rs` 聚合导出 `Document`、`Question`、`QuestionKind`、`Answer`、`validate_draft`（仅检查容量、长度与身份，不跑规则编译）、`validate_publish`、`validate_answers`、`renew_ids` 以及 DSL/流程接口；`backend/src/lib.rs` 通过 `pub use questionnaire_logic::domain;` 再导出。领域校验错误 `ValidationError { question_id: Option<Uuid>, message }` 映射为 HTTP 422。
- **持久化与并发**：6 个 SeaORM 实体位于 `entities.rs`（`users`、`sessions`、`surveys`、`versions`、`responses`、`audits`），唯一建表 DDL 位于 `backend/src/schema.sql`（由 `migration.rs` 嵌入执行）；业务主键与领域实体 ID 采用 UUIDv7（`Uuid::now_v7()`）。`responses` 持久化 `outcome`（`complete` / `screen_out`）与 `question_states` 三态 JSONB；发布者写操作在事务内对目标问卷行加 `FOR UPDATE` 锁并校验归属；公开提交对问卷行加 `FOR SHARE` 锁以允许并发提交；只读列表、公开读题面、统计与导出直接通过连接池执行无锁查询（已发布版本命中进程内有界 LRU `VersionCache`）。
- **统一错误与安全**：`AppError { status, code, message, question_id, error_id }` 序列化为 `{"error":{"code":"...","message":"...","question_id":null}}`；内部 500 错误生成 16 位十六进制 `error_id`（连同错误详情写入日志，并在响应头附带 `X-Error-Id`），绝不向外泄露内部错误文本。Session 采用 32 字节随机十六进制 token（库内仅存 SHA-256 `id_hash`，并由后台周期任务清理过期会话）；密码哈希使用 `spawn_blocking` + Argon2id；`security::guard` 强制校验非读请求的 `Origin == PUBLIC_ORIGIN` 与 `X-CSRF-Protection: 1`，并对登录/注册按每日随机盐盲化 IP、公开提交按（盲化 IP, 问卷 ID）执行固定窗口限流（`TRUST_PROXY` 控制代理头解析，`ALLOW_INSECURE_HTTP` 控制非本机明文 HTTP 放行）。

## Key Directories & Module Table

| 文件 / 目录 | 职责 |
| --- | --- |
| `backend/src/lib.rs`、`main.rs` | 组合根 `run()`、`router()`、`AppState`、`/health`、OpenAPI 合并与二进制入口 |
| `backend/src/config.rs`、`error.rs`、`http.rs` | 环境变量解析、`AppError` 错误信封、`ApiJson` / `ApiQuery` / `ApiPath` 抽取器与 `Pagination` |
| `backend/src/security.rs`、`auth.rs` | 同源 CSRF 防护、固定窗口限流、安全响应头；`AuthUser`、Argon2id、Session 与 Cookie 处理 |
| `backend/src/entities.rs`、`repository.rs`、`migration.rs` + `schema.sql` | SeaORM 实体定义、归属过滤与行锁助手、唯一权威 DDL |
| `backend/src/surveys.rs`、`submissions.rs`、`reports.rs`、`admin.rs` | 发布者问卷生命周期、公开填写与幂等提交、分版本统计与 CSV 导出、管理员治理与审计 |
| `logic-core/src/domain.rs`、`identity.rs` | 问卷文档模型、容量与题型校验；非零连字符 UUID（大小写归一）的严格反序列化边界 |
| `logic-core/src/logic.rs`、`logic_document.rs`、`flow.rs` | 表达式类型检查与三值逻辑求值、计算变量拓扑排序与题目规则、前向流程状态机与路径重建 |
| `logic-core/src/logic_text.rs`、`logic_text.pest` | 词法预算预检、pest Pratt 优先级解析、别名绑定与唯一 `format_logic_text` 实现 |
| `logic-wasm/src/lib.rs`、`frontend/src/wasm/` | WASM JSON 字符串信封边界与前端入库五件套（`pkg/`）及 TS 包装（`index.ts`） |
| `fixtures/survey-logic/`、`logic-core/tests/`、`logic-wasm/tests/`、`backend/src/tests/`、`frontend/tests/` | 双端共读的语义夹具、领域契约集成测试、WASM 信封边界测试、后端模块化单元与 HTTP 边界测试、前端分层测试与浏览器环境桩 |

## Development Commands

从仓库根目录运行（PowerShell）：

```powershell
./start.ps1                                                   # 一键启动本地 PG18（.pgdata:55432）、自动迁移、建默认管理员并拉起前后端
Copy-Item .env.example .env                                   # 首次配置，前端联调需设 PUBLIC_ORIGIN=http://localhost:5173
cargo run --locked -p questionnaire-api                       # 启动后端（自动执行迁移，默认监听 127.0.0.1:3000）
cargo run --locked -p questionnaire-api -- migrate            # 仅执行数据库迁移后退出
cargo run --locked -p questionnaire-api -- create-admin       # 读取进程环境变量 ADMIN_USERNAME / ADMIN_PASSWORD 创建管理员
cargo fmt --all -- --check
cargo test --locked --workspace                               # 纯内存测试，无需数据库或环境变量
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --release --locked -p questionnaire-api
Set-Location frontend; pnpm install --frozen-lockfile
pnpm run dev                                                  # 启动前端开发服（固定 http://localhost:5173）
pnpm run test:regression                                      # 运行前端分层 Node 测试（tests/{contract,unit,integration}）
pnpm run build                                                # vue-tsc 类型检查 + Vite 生产打包
pnpm run format:check                                         # Prettier 格式检查
pnpm run wasm:build                                           # 仅在改动 logic-core/ 或 logic-wasm/ 时重建并同步 WASM 产物
```

## Code Conventions & Anti-Over-Engineering Rules

- **信守边界（Parse, don't re-validate）**：只在最外层边界（HTTP 抽取器、WASM 字符串信封、数据库约束）做一次性解析与校验，内部模块直接信任已确立的不变量。
- **只读快照与 JSONB 投影**：只读路径（公开读题面、分页列表、统计与导出）一律使用 `RepeatableRead + ReadOnly` 快照或普通只读查询，绝不加 `FOR UPDATE` / `FOR SHARE` 行锁；列表只需标题时使用 JSONB 字段投影（`cast_json_field("title")`），禁止反序列化整份 `Document`。
- **禁止回头解析原始 JSON 或静默兜底**：表达式、规则与流程编译为类型化结构后，下游分析直接在类型化节点或预建索引上运行；编译期已保证存在的数据直接按类型/索引访问，禁止用 `unwrap_or_default()` 或伪造错误掩盖内部不变量违约。
- **禁止无意义深拷贝与双次序列化**：WASM 边界使用泛型 `Envelope<T>` 单遍 `serde_json::to_string`；状态机就地更新输入，无规则报表不分配每题状态表；前端不对刚解码或只读消费的对象反复调用 `copy()`。

## 前端约定（Editorial Craft 视觉基线）

- **视觉体系**：采用 **Editorial Craft（纸本与松烟墨）** 风格（全局设计 Token 与基础样式位于 `frontend/src/styles.css`，业务与工作台布局拆分于 `application.css`、`editor.css`）：
  - 宣纸浅色（`#f5f2eb`）与玄墨深色（`#121210`）基底，配 `32px` 极淡稿纸经纬网格背景；`index.html` 首绘脚本与 `stores.ts` 的 `CANVAS_COLORS` 保持严格一致。
  - 三套字体阶梯：`--font-serif`（中文衬线标题、品牌名、问卷题干、指标大字）、`--font-mono`（等宽 `tabular-nums` 数据指标、修订号、回执编号、眉题与 DSL 代码）、`--font-sans`（正文与表单控件）。
  - 语义色盘：朱砂印泥（`#c84b31` / `#e4684e`）作点睛主操作与选中高亮，松石绿（`#1d634d` / `#46b593`）表征采集中与校验通过，靛青（`#294c7a` / `#76a1de`）表征计算变量与分支逻辑，琥珀（`#945d14` / `#e2a54f`）表征草稿与诊断提示，DSL 文本工作台采用松烟墨深色控制台（`#191816` / `#0d0d0c`）。
- **工具与包管理**：固定使用 `pnpm`（`frontend/pnpm-lock.yaml`）；开发与预览端口固定为 `http://localhost:5173`，通过 Vite 同源代理 `/api` 到后端。
- **按需加载**：Element Plus 仅在 `SearchSelect.vue` 按需引入 `ElSelect`/`ElOption`；CodeMirror 6 仅在 `LogicTextEditor.vue` 通过 `defineAsyncComponent` 懒加载 `DslCodeEditor.vue`。

### 交互与操作布局规范（Interaction & Action Placement Rules）

1. **对象与操作就近绑定**：实体级管理操作（删除草稿、重命名、导出、切换状态等）必须绑定在该实体的入口卡片行（List Item Row）或编辑器顶部固定栏（Sticky Top Header）；严禁塞进分步表单的最后一步或页面流式末尾。
2. **全流程随时可退**：多步向导或长表单中，「保存草稿」「丢弃/删除草稿」「取消返回」属于全局逃生口（Escape Hatches），必须在每一步的顶栏或固定底栏左侧常驻可见，绝不能要求用户翻到流程终点才能删除或取消。
3. **主次与破坏性操作分离**：每个视图只允许 1 个最高视觉权重的主按钮（Primary CTA，置于主操作区右侧）。破坏性操作（如「删除草稿」）必须与主提交按钮在空间上拉开距离（顶栏右上角更多菜单、卡片操作列或固定栏最左端），使用低饱和度幽灵按钮（`.text-button.danger-text`、`.ghost-danger` 等）并在悬停/键盘聚焦时变为朱红 `--rose`，防止误触。
4. **写代码前先过动线自检**：生成任何带状态界面之前，先用三行注释写明 `[入口在哪] -> [用户在第 1 步后悔了怎么删/退] -> [主提交通道在哪]`；每个视图文件顶部第 1–3 行必须保留这三行自检注释。

## Testing & QA

测试按边界分层，禁止在同一个文件里混装传输断言与领域语义：

| 层级 | 位置 | 覆盖内容 |
| --- | --- | --- |
| `logic-core` 单元 | `logic-core/src/*.rs` 内的 `#[cfg(test)]` 模块 | 各模块内部不变量：身份严格解析、表达式、变量拓扑排序、流程状态机与文本 DSL |
| `logic-core` 契约集成 | `logic-core/tests/{domain,logic,flow,logic_text}.rs` | 领域校验、三值逻辑、前向流程与文本 DSL 的跨模块契约，并与 `fixtures/survey-logic/` 共读人工预期 |
| `logic-wasm` 边界 | `logic-wasm/tests/smoke.rs` | JSON 字符串信封的成功/错误形状与错误分类穿透 |
| `backend` 单元与 HTTP | `backend/src/tests/{config,error_and_http,security_and_router,auth_and_admin,surveys_and_submissions,reports}.rs` | 配置解析、错误信封与抽取器、CSRF/盲化 IP 限流/路由、认证与治理、问卷生命周期与提交、报表聚合与分批 CSV 导出 |
| `backend` 真实 DB | `backend/tests/pg_integration.rs` | 由 `TEST_DATABASE_URL` 门控的真实 PostgreSQL 18 迁移、字面标题筛选、`login_once` + `screen_out` 并发唯一约束与三态持久化测试 |
| `frontend` 契约 | `frontend/tests/contract/` | 与 Rust 共读 `fixtures/survey-logic/`（`documents`、`expressions`、`flows`、`input-contracts`、`submissions`、`texts`），断言两端语义一致 |
| `frontend` 单元 | `frontend/tests/unit/` | 18 个模块：`value-contract`、`numbers`、`json`、`format`、`logic-errors`、`answers`、`documents`、`condition-builder`、`flow-tools`、`editor-logic`、`survey-templates`、`logic-text-tools`、`api`、`resource`、`stores`、`router`、`wasm-envelope`、`uuid` |
| `frontend` 集成 | `frontend/tests/integration/` | `editor-workflow`、`flow-inputs`：编辑器工具与 `FlowSession` 输入适配的跨模块流程 |
| 共享测试基建 | `backend/src/tests/support.rs`、`frontend/tests/helpers/` | 后端无连接 DB/原始 HTTP 测试桩 `support.rs`（由 `mod.rs` 聚合）；前端 `fixtures.mjs` 夹具缓存、`documents.mjs` 固定身份构造器、`browser.mjs` 按需安装且 `t.after` 自动清理的浏览器环境桩 |

- 修改 Rust 代码必跑：受影响的 `cargo test`、`cargo clippy --locked --workspace --all-targets -- -D warnings` 与 `rustfmt`。
- 修改前端代码必跑：`pnpm run test:regression`（递归执行 `frontend/tests/` 下的分层测试）、`pnpm run typecheck`（或 `pnpm run build`）与 `prettier`。
- 修改 `logic-core/` 或 `logic-wasm/` 必跑：先在 `frontend/` 执行 `pnpm run wasm:build` 同步 `src/wasm/pkg/`，再运行两端测试验证语义一致。
