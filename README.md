# 问序（Wenxu）— 动态问卷调查系统（Rust + Vue 3 + WASM）

面向课程反馈、活动报名与研究调研的 Web 动态问卷系统：发布者使用六类题型、计算变量、条件规则与分支跳题编纂问卷，发布不可变版本并按公开匿名或账号限次模式收集答卷，查看分版本统计与导出 CSV；平台管理员治理用户与问卷并记录审计日志。

后端为 **Rust + Axum + SeaORM + SQLx** 的 HTTP JSON API，领域规则与流程引擎由纯 Rust crate **`logic-core`（`questionnaire-logic`）** 唯一实现，并经 **`logic-wasm`** 编译为 WebAssembly 供 **Vue 3 + TypeScript + Vite** 前端直接调用。界面采用 **Editorial Craft（纸本与松烟墨）** 视觉体系，支持宣纸浅色、玄墨深色与跟随系统。

---

## 1. 核心能力与架构

| 领域 | 已实现能力 |
| --- | --- |
| 账号与权限 | 注册、登录、退出、修改密码（全设备会话失效）；HttpOnly Cookie Session；发布者归属隔离；管理员禁用/启用账号、下架/恢复问卷并写入同事务审计 |
| 问卷编纂 | 单选、多选、单行文本、多行文本、数字、评分六类题型；内置三套带规则模板；乐观锁草稿保存（`revision`）、整卷/单题复制 |
| 规则与跳题 | 可视化条件构建器与文本 DSL 双向无损切换；计算变量、题目显隐（`visible_if`）、动态必填（`required_if`）、答案约束（`validate`）与前向无环页面分支（`flow`） |
| 发布与回收 | 不可变发布版本快照；同内容重复发布幂等返回当前版本；公开匿名或同账号跨版本限填一次（`login_once`）；截止时间、手动关闭/恢复、不可撤销归档 |
| 在线填写与提交 | 桌面/手机预览与内存试填；按规则分步渲染与回退改答自动清理；UUIDv4 提交秘密幂等重放；服务端按历史版本重建有效路径 |
| 报表与导出 | 按版本与 `[from, to)` 时间范围筛选；区分「已答 / 适用未答 / 规则跳过」三态统计；选项占比、评分稀疏分布、数值均值与自然日趋势；UTF-8 CSV 导出（含电子表格公式注入防护） |

### 目录与工作区结构

```text
frontend/           Vue 3 + TypeScript + Vite + Pinia + Vue Router（固定使用 pnpm）
      │  调用 frontend/src/wasm/pkg/ 入库产物
logic-wasm/         WASM 边界：JSON 字符串信封 {"ok":…} / {"error":…}
      │
logic-core/         纯领域 DSL 与流程核心（questionnaire-logic，无 HTTP/DB 依赖）
      │  pub use questionnaire_logic::domain;
backend/            HTTP JSON API、鉴权、事务与持久化（questionnaire-api）
```

- **单一真源**：表达式编译求值、变量拓扑排序、前向流程状态机与文本 DSL 解析/格式化仅在 `logic-core/` 实现一份。前端不再保留 TypeScript 镜像实现。
- **更多文档**：[后端开发指南](docs/backend-guide.md)、[前端开发指南](docs/frontend-guide.md)、[DSL 与流程引擎规范](docs/survey-dsl-plan.md)、[需求与架构规格说明](docs/requirements-analysis.md)。

---

## 2. 环境要求

| 工具 | 版本要求 | 说明 |
| --- | --- | --- |
| Rust / Cargo | **stable ≥ 1.94**（2024 edition） | 根 `Cargo.lock` 锁定依赖版本，命令建议加 `--locked` |
| C/C++ 编译工具 | MSVC Build Tools + Windows SDK（Windows）或 GCC/Clang | 编译原生依赖（如 `ring`）所需 |
| PostgreSQL | **PostgreSQL 18** | 唯一持久化存储，无 SQLite 或内存回退模式 |
| Node.js / pnpm | Node.js `^22.18.0` 或 `>=24.12.0`，**pnpm**（见 `frontend/package.json` 的 `packageManager`） | 仅改前端时直接使用入库的 WASM 产物，无需安装 `wasm-pack` |

---

## 3. 快速启动（本地开发与联调）

### 3.1 一键启动（推荐）

在仓库根目录运行：

```powershell
.\start.ps1
```

脚本会自动完成以下工作：

- 在 `.pgdata/` 下维护一个**测试专用、持久化**的 PostgreSQL 18 实例（`127.0.0.1:55432`，用户 `questionnaire`，数据库 `questionnaire_test`，本地 `trust` 认证），与系统默认 `5432` 实例互不干扰，重启脚本后数据仍然保留；
- 若根目录没有 `.env`，自动生成与测试库匹配的配置（`DATABASE_URL=postgres://questionnaire@127.0.0.1:55432/questionnaire_test`、`PUBLIC_ORIGIN=http://localhost:5173`）；
- 执行数据库迁移，并幂等创建默认管理员 **`admin` / `admin123456`**；
- 并发启动前端 `pnpm --dir frontend run dev` 与后端 `cargo run --locked -p questionnaire-api`，按 `Ctrl+C` 退出时自动清理前端子进程。

| 参数 | 作用 |
| --- | --- |
| `-ResetDb` | 停止并删除 `.pgdata/`，重新初始化干净的测试数据库 |
| `-Stop` | 停止后台测试数据库后退出 |
| `-DbOnly` | 仅确保测试数据库启动、完成迁移与默认管理员初始化后退出（不启动前后端） |

> `initdb`/`pg_ctl`/`pg_isready`/`psql`/`createdb`、`cargo` 与 `pnpm` 需在 `PATH` 中（脚本会给出缺失提示）。`.pgdata/` 已加入 `.gitignore`，不会污染版本库；默认管理员口令仅供本地测试，请勿用于生产环境。

### 3.2 准备 PostgreSQL 数据库

你可以使用本机已安装的 PostgreSQL 18，或通过仓库根目录的 `compose.yaml` 启动容器数据库。

**方式 A：使用本机 PostgreSQL**

创建专用角色 `questionnaire` 与 UTF-8 数据库 `questionnaire`（若已有可用实例与数据库，直接使用其连接串即可）：

```powershell
psql -h 127.0.0.1 -p 5432 -U postgres -d postgres -c "CREATE ROLE questionnaire WITH LOGIN PASSWORD 'change_me';"
createdb -h 127.0.0.1 -p 5432 -U postgres --owner=questionnaire --encoding=UTF8 --template=template0 --locale=C questionnaire
```

**方式 B：使用 Docker Compose（可选）**

```powershell
$env:POSTGRES_PASSWORD = 'change_me'
docker compose up -d db
```

### 3.3 配置环境变量（`.env`）

在仓库根目录复制 `.env.example` 为 `.env`：

```powershell
Copy-Item .env.example .env
```

编辑 `.env`（**与前端联调时，务必将 `PUBLIC_ORIGIN` 设为 `http://localhost:5173`**，否则写请求会因同源 CSRF 校验返回 `403 csrf_rejected`）：

```dotenv
DATABASE_URL=postgres://questionnaire:change_me@127.0.0.1:5432/questionnaire
LISTEN_ADDR=127.0.0.1:3000
PUBLIC_ORIGIN=http://localhost:5173
COOKIE_SECURE=false
SESSION_TTL_SECONDS=86400
AUTH_REQUESTS_PER_MINUTE=60
SUBMISSIONS_PER_MINUTE=600
RUST_LOG=questionnaire_api=info
```

> 进程环境变量优先级高于 `.env` 文件。密码中若包含 `@`、`:`、`/`、`#`、`%` 等特殊字符，需进行 URI 百分号编码。

### 3.4 启动后端 API

在仓库根目录运行：

```powershell
cargo run --locked -p questionnaire-api
```

- 默认执行 `serve` 子命令，监听 `LISTEN_ADDR`（默认 `127.0.0.1:3000`）。
- **每次启动会自动执行数据库迁移**（建表定义唯一位于 `backend/src/schema.sql`）。若只需执行迁移后立即退出，可运行 `cargo run --locked -p questionnaire-api -- migrate`。
- 健康检查与 OpenAPI 文档：
  - `GET http://localhost:3000/health` → `{"status":"ready"}`
  - `GET http://localhost:3000/api/openapi.json` → OpenAPI 3.1 JSON

### 3.5 创建与登录管理员账号

公开注册接口（`/register`）只能创建普通用户（`user`）。管理员账号通过后端 `create-admin` 子命令初始化：

```powershell
$env:ADMIN_USERNAME = 'admin'
$env:ADMIN_PASSWORD = 'your_strong_password'
cargo run --locked -p questionnaire-api -- create-admin
Remove-Item Env:ADMIN_USERNAME, Env:ADMIN_PASSWORD
```

- 使用 `.\start.ps1` 启动时，默认管理员（`admin` / `admin123456`）已自动幂等创建，无需重复执行本节；如需自定义账号或口令，再按上述命令操作。
- **如何登录管理员**：管理员与普通用户使用**同一个前端登录页**（`http://localhost:5173/login`）或登录接口（`POST /api/auth/login`）。
- 登录成功后，左侧导航栏会自动出现 **「平台管理」**（`/admin`）入口，可执行账号禁用/启用、问卷下架/恢复及查看治理审计记录。

### 3.6 启动前端工作台

另开终端进入 `frontend/`：

```powershell
Set-Location frontend
pnpm install --frozen-lockfile
pnpm run dev
```

浏览器访问 **http://localhost:5173** 即可使用完整工作台。

- Vite 开发服务器固定监听 `localhost:5173`（`strictPort: true`），并将 `/api` 同源代理至 `http://127.0.0.1:3000`（可通过 `frontend/.env` 中的 `API_PROXY_TARGET` 覆盖后端地址）。
- 请全程统一使用 `http://localhost:5173` 访问前端，勿混用 `127.0.0.1:5173`，以确保 `Origin` 请求头与 `PUBLIC_ORIGIN` 严格匹配。

---

## 4. 生产构建与部署指南

### 4.1 构建前后端产物

```powershell
# 1. 构建后端 Release 二进制（单 codegen 单元 + fat LTO + strip symbols）
cargo build --release --locked -p questionnaire-api
# 产物路径：target/release/questionnaire-api（Windows 下为 questionnaire-api.exe）

# 2. 构建前端静态资源（含 vue-tsc 类型检查）
Set-Location frontend
pnpm install --frozen-lockfile
pnpm run build
# 产物路径：frontend/dist/
```

若仅在本机预览打包后的前端产物，可先停止 `pnpm run dev`，在 `frontend/` 下执行 `pnpm run preview`（同样监听 `http://localhost:5173` 并代理 `/api`）。

### 4.2 生产环境部署拓扑与配置

生产环境推荐通过 **Nginx / Caddy 等反向代理**统一对外提供 HTTPS 服务，将前端静态产物与后端 API 置于同一 Origin 下：

1. **反向代理路由**：
   - `/api/*` 與 `/health` 反向代理至后端 `http://127.0.0.1:3000`，并透传 `Host`、`Origin`、`Cookie` 与 `X-CSRF-Protection` 请求头。
   - 其余路径托管 `frontend/dist/` 静态文件，并对 history 路由配置 `try_files $uri $uri/ /index.html` 回退。
2. **后端生产环境变量**：
   - `PUBLIC_ORIGIN=https://survey.example.com`（精确填写对外 HTTPS 裸 origin，无路径、无末尾 `/`）。
   - `COOKIE_SECURE=true`（HTTPS 环境必须开启，确保 Session Cookie 仅经加密通道传输）。
   - `DATABASE_URL=postgres://...`（使用最小权限专用数据库账号，勿使用超级用户）。

---

## 5. 安全与业务边界

- **同源 CSRF 防护**：所有非 `GET`/`HEAD`/`OPTIONS` 请求必须满足 `Origin == PUBLIC_ORIGIN` 且携带 `X-CSRF-Protection: 1`；服务端不开启跨域 CORS，所有响应默认附带 `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff`。
- **固定窗口限流**：`AUTH_REQUESTS_PER_MINUTE`（认证写接口）与 `SUBMISSIONS_PER_MINUTE`（公开提交接口）为单实例全局计数，用于基础防滥用，不按 IP 识别自然人。
- **隐私边界**：`anonymous` 模式绝不解析 Cookie、不记录账号关联；`login_once` 模式在库内记录填写者账号 ID 以执行跨版本唯一约束，但发布者的答卷列表、详情、统计和 CSV 导出均不暴露填写者身份。管理员治理接口仅可查看问卷元数据与已发布题面，不可读取他人答卷内容。

---

## 6. 开发检查与编译配置

按实际改动范围执行对应门禁：

| 改动范围 | 必跑检查命令 |
| --- | --- |
| Rust 代码（`backend/`、`logic-core/`、`logic-wasm/`） | `cargo test --locked --workspace`<br>`cargo clippy --locked --workspace --all-targets -- -D warnings`<br>`cargo fmt --all -- --check` |
| 前端代码（`frontend/`） | `pnpm run test:regression`<br>`pnpm run typecheck`（或 `pnpm run build`）<br>`pnpm run format:check` |
| 修改 `logic-core/` 或 `logic-wasm/` 后同步前端 | 在 `frontend/` 执行 `pnpm run wasm:build`（需 `wasm32-unknown-unknown` target 与 `wasm-pack`），再跑两端回归测试 |

- 可选风格审查：`cargo clippy --locked --workspace --all-targets -- -W clippy::pedantic -W rust-2024-compatibility`。这些提示用于人工取舍，不替代默认 Clippy 门禁；优先保持代码可读性、业务语义与性能，不为清零告警机械拆分连贯算法、改写公开字段或引入额外分配。分页命名、PATCH 三态和统计浮点转换等合理写法可以保留，不使用批量 `allow` / `expect` 掩盖提示。

- **无需数据库即可跑单测**：`cargo test --locked --workspace` 的测试按层级组织——`logic-core/src/tests/{identity,domain,logic,logic_document,flow,logic_text}.rs` 模块化单元测试、`logic-core/tests/{domain,logic,flow,logic_text}.rs` 领域契约集成测试（与 `fixtures/survey-logic/` 共读人工预期）、`logic-wasm/tests/smoke.rs` WASM JSON 信封边界测试，以及 `backend/src/tests/{config,error_and_http,security_and_router,auth_and_admin,surveys_and_submissions,reports}.rs` 模块化单元与 HTTP 边界测试；前端 `pnpm run test:regression` 递归执行 `frontend/tests/{contract,unit,integration}/`：`contract` 共读夹具语义，`unit` 覆盖 17 个模块（`value-contract`、`numbers`、`json`、`format`、`logic-errors`、`answers`、`documents`、`condition-builder`、`flow-tools`、`editor-logic`、`survey-templates`、`logic-text-tools`、`api`、`resource`、`stores`、`router`、`wasm-envelope`），`integration` 覆盖 `editor-workflow`/`flow-inputs`；共享 `frontend/tests/helpers/{fixtures,documents,browser}.mjs` 的夹具缓存、固定身份构造器与浏览器环境桩，加载真实入库 WASM 并共读同一批夹具。两者均在内存中完成，不依赖外部数据库。
- **编译调优（根 `Cargo.toml` 与 `.cargo/config.toml`）**：
  - `dev` profile 对工作区自身代码使用 `opt-level = 0`，对第三方依赖使用 `opt-level = 3`，兼顾增量编译速度与依赖运行性能。
  - `release` profile 启用 `opt-level = 3`、`codegen-units = 1`、`lto = "fat"`、`strip = "symbols"` 与 `panic = "abort"`。
  - 若本机安装了 nightly 与 `rustc-codegen-cranelift-preview`，开发期可使用 `cargo +nightly cranelift -p questionnaire-api` 获得更快的单 crate 增量编译；正式检查与发布仍使用默认 stable + LLVM。
