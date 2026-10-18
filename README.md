# 问卷调查系统（Rust + Vue 3）

面向校园课程反馈、社团活动反馈和一般研究调查的问卷调查系统：发布者自主设计、发布和管理问卷，填写者通过链接在电脑或手机上作答，发布者查看分版本统计并导出数据，管理员处理账号与内容治理。

后端是 Rust + Axum + SeaORM 的 HTTP JSON API，前端是 Vue 3 + TypeScript + Vite 的工作台，采用 **Fluent 2** 视觉（圆角控件、分级阴影、带轻微色调的 Mica 基底与纯色磁贴，支持浅色、深色和跟随系统）。启动后从 `http://localhost:5173` 进入工作台。

## 核心功能

| 领域 | 已实现能力 |
| --- | --- |
| 账号与权限 | 注册、登录、退出、改密；Cookie Session；发布者只能操作自己的问卷与答卷；管理员可禁用/启用账号、下架/恢复问卷并留下审计记录 |
| 问卷编辑 | 六类基础题型：单选、多选、单行文本、多行文本、数字、评分；草稿保存、发布前校验、复制问卷 |
| 规则与流程 | 可视化规则/跳题编辑器与文本 DSL 两种写法；条件显示与跳题；电脑/手机草稿预览、分步填写 |
| 发布与回收 | 不可变发布版本；匿名填写或同账号跨版本限交一次；截止时间、关闭、显式恢复、不可撤销归档 |
| 提交与报表 | 完整答卷原子写入；UUIDv4 提交秘密与幂等重试；答卷查看、按版本统计、CSV 导出 |
| 平台治理 | 管理员禁用/启用账号、下架/恢复问卷及审计；治理接口不提供他人的答卷内容 |
| 运维 | PostgreSQL 迁移、健康检查、OpenAPI JSON |

真实接口驱动整个工作台：编辑、预览、公开填写页、答卷统计和平台管理都直接读写业务数据，没有用演示数据替代业务结果。

## 架构概览

根目录是三个成员的 Cargo workspace，加上一个独立的 Vue 3 前端：

```
frontend/           Vue 3 + TypeScript + Vite + Pinia + Vue Router（pnpm）
      │  调用入库的 wasm 产物
logic-wasm/         WASM 边界：入参出参均为 JSON 字符串信封（包名 logic-wasm）
      │
logic-core/         纯领域 DSL 与流程核心（包名 questionnaire-logic）
      │  pub use questionnaire_logic::domain;
backend/            HTTP JSON API 与业务用例（包名 questionnaire-api）
```

- `backend/`：HTTP 边界、业务用例与数据访问，通过 `pub use questionnaire_logic::domain;` 再导出领域核心，因此业务代码里的 `questionnaire_api::domain::*` 路径保持不变。
- `logic-core/`：`domain.rs`、`identity.rs`、`logic.rs`、`logic_document.rs`、`logic_text.rs` + `logic_text.pest`、`flow.rs`，负责表达式、文档规则、流程与文本语法；没有 HTTP/DB 依赖，可编译到 `wasm32-unknown-unknown`。
- `logic-wasm/`：把领域核心以泛型 `Envelope<T>` 单遍序列化成 JSON 字符串信封 `{"ok":…}` / `{"error":…}`，供浏览器调用。
- `frontend/`：DSL 规则与流程直接调用 `frontend/src/wasm/index.ts` 包装的入库产物 `frontend/src/wasm/pkg/`，**不再有 TypeScript 镜像实现**；包管理器固定为 pnpm，锁文件是 `frontend/pnpm-lock.yaml`。

规则引擎只有 `logic-core` 一份实现，前端与后端因而共享同一套表达式、文档和流程语义。范围与验收状态见[需求分析](docs/requirements-analysis.md)，运行与代码阅读见[后端指南](docs/backend-guide.md)和[前端指南](docs/frontend-guide.md)。`frontend/` 下的 `prototype.html` 与 `fluent-prototype.html` 保留为独立风格原型（后者对应当前 Fluent 2 视觉），不被应用引用；实际入口为 `frontend/index.html`。

## 1. 准备环境

以下命令使用 **Windows PowerShell**，从**仓库根目录（含 `Cargo.toml`、`.env.example`、`compose.yaml` 的目录）**运行，而不是 `backend/src`。

| 工具 | 要求 |
| --- | --- |
| Rust / Cargo | Rust **1.94 或更新版本**；三个成员都使用 Rust 2024 edition，`rust-version` 声明在各自的 `Cargo.toml`，不能只按 edition 推断编译器版本 |
| Windows 编译环境 | 安装 Rust MSVC 工具链所需的 Visual Studio C++ Build Tools 和 Windows SDK |
| PostgreSQL | **PostgreSQL 18**；服务端与 `psql`、`createdb`、`pg_dump`、`pg_restore` 客户端均准备好，并把客户端 `bin` 目录加入 PATH |
| Node.js / pnpm | Node.js `^22.18.0` 或 `>=24.12.0`（与 `frontend/package.json` 的 `engines` 一致）。前端依赖一律用 pnpm 安装，版本由同文件的 `packageManager` 字段固定，锁文件为 `frontend/pnpm-lock.yaml`（`package-lock.json` 已删除，npm 只用于首次安装 pnpm） |
| PowerShell | Windows PowerShell 5.1 或 PowerShell 7；中文请求示例显式发送 UTF-8 字节 |

当前锁定的核心依赖为 Axum 0.8.9、SeaORM / sea-orm-migration 2.0.4、SQLx 0.9.0、Tokio 1.53.2，准确版本以 [Cargo.lock](Cargo.lock) 为准，命令加 `--locked` 避免无意更新依赖。路由与 OpenAPI 由 utoipa 6.0.0 / utoipa-axum 0.3.0 统一登记，Cookie 解析与构造使用 cookie 0.18.2，Rust DSL 文法与运算优先级使用 pest / pest_derive 2.9.2。PostgreSQL 18 是本项目的运行基线，并非对所有旧大版本作兼容保证。

```powershell
rustc --version
cargo --version
psql --version
node --version
pnpm --version
```

本机没有 pnpm 时先安装一次；具体版本由 `frontend/package.json` 的 `packageManager` 字段承担，不需要手工挑选：

```powershell
npm install --global pnpm
```

## 2. 本机数据库：创建专用账号和空库

先启动已安装的 PostgreSQL 服务。下面用安装时的数据库管理员 `postgres` 建账号；`-W` 会交互提示密码，不把密码放进命令行。

```powershell
psql -h 127.0.0.1 -p 5432 -U postgres -d postgres -W
```

进入 `psql` 后执行一次；`\password` 会提示输入你自己的新密码：

```sql
CREATE ROLE questionnaire LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
\password questionnaire
\q
```

回到仓库根目录的 PowerShell，创建由该账号拥有的 UTF-8 空库：

```powershell
createdb -h 127.0.0.1 -p 5432 -U postgres -W --owner=questionnaire --encoding=UTF8 --template=template0 --locale=C questionnaire
psql -h 127.0.0.1 -p 5432 -U questionnaire -d questionnaire -W -c "SELECT current_user, current_database();"
```

应用日常只应使用这个专用账号，不要用超级用户跑服务。如果已有同名账号或数据库，沿用现有的即可，不要重复创建或删除已有数据，并同步调整连接配置。

## 3. 配置、迁移和启动

首次配置时复制模板；已有 `.env` 时直接编辑，不要覆盖：

```powershell
Copy-Item .env.example .env
notepad .env
```

把 `DATABASE_URL` 的密码换成刚设置的数据库密码。密码若含 `@`、`:`、`/`、`#`、`%` 等字符，必须做 URI 百分号编码。`.env` 含数据库凭据，只保留在本机，不提交、不截图分享。

仅使用 API 时可保留以下默认配置；同时运行前端时，把 `PUBLIC_ORIGIN` 改成 `http://localhost:5173`：

```dotenv
LISTEN_ADDR=127.0.0.1:3000
PUBLIC_ORIGIN=http://localhost:3000
COOKIE_SECURE=false
SESSION_TTL_SECONDS=86400
AUTH_REQUESTS_PER_MINUTE=60
SUBMISSIONS_PER_MINUTE=600
RUST_LOG=questionnaire_api=info
```

应用通过 `dotenvy` 读取根目录 `.env`，已经设置的进程环境变量优先于文件。若连接到了意外的数据库，先检查当前终端是否遗留了 `DATABASE_URL` 环境变量。

```powershell
# 只执行迁移，然后退出
cargo run --locked -p questionnaire-api -- migrate

# 启动 HTTP 服务，保持此终端运行；Ctrl+C 停止
cargo run --locked -p questionnaire-api
```

默认子命令是 `serve`，也可写 `cargo run --locked -p questionnaire-api -- serve`。**服务启动本身会先运行迁移**，因此不必每次手工执行 `migrate`，迁移版本由 SeaORM 记录。数据库不可达或配置不合法会直接报错，不会自动创建数据库，也没有内存存储或 SQLite 回退。

另开一个仍在仓库根目录的 PowerShell 验证：

```powershell
Invoke-RestMethod http://localhost:3000/health
$OpenApi = Invoke-RestMethod http://localhost:3000/api/openapi.json
$OpenApi.info
```

`/health` 成功应返回 `status = ready`，含义是数据库连接就绪。完整可复制的请求流程在[后端指南：PowerShell 最小真实闭环](docs/backend-guide.md#1-powershell-最小真实闭环)。

### 3.1 启动前端 MVP

在根目录 `.env` 设置 `PUBLIC_ORIGIN=http://localhost:5173`、`COOKIE_SECURE=false`，然后启动或重启后端。另开 PowerShell，在仓库根目录运行：

```powershell
Set-Location frontend
pnpm install --frozen-lockfile
pnpm run dev
```

打开 **http://localhost:5173**，注册账号后即可创建、编辑、发布问卷。发布后的填写链接为 `/s/:id`，可复制给填写者；后端响应里的 `share_url` 仍是公开 JSON 接口路径。管理员初始化沿用第 4 节，登录后会显示“平台管理”。

开发服务器固定使用 `localhost:5173`，把 `/api` 代理到 `http://127.0.0.1:3000`，并保留浏览器的 `Origin` 和 Cookie。后端使用其他地址时，可复制 `frontend/.env.example` 为 `frontend/.env` 并修改 `API_PROXY_TARGET`，或在运行 `pnpm run dev` 的终端设置该环境变量；它只由 Vite 服务器读取，不要把密钥放进 `VITE_` 变量。

构建与本机预览也在 `frontend/` 中执行：

```powershell
pnpm run build
# 先停止 pnpm run dev，避免占用同一端口
pnpm run preview
```

`build` 包含 TypeScript 检查，生成 `frontend/dist/`；`preview` 使用相同的 `5173` 端口与 API 代理，仍需运行后端及数据库。前端不由 Rust 服务托管，直接双击 `dist/index.html` 也用不了实际应用。更多页面结构与调试说明见[前端指南](docs/frontend-guide.md)。

## 4. 按需初始化管理员

普通注册不能指定管理员角色。下面的命令**只新建管理员，不会升级或覆盖已有账号**。仍在仓库根目录运行，用交互凭据窗口输入密码，避免密码出现在命令历史或进程参数里：

```powershell
$AdminCredential = Get-Credential -UserName admin_bootstrap -Message '输入新管理员用户名及密码（密码 8—128 个字符）'
$env:ADMIN_USERNAME = $AdminCredential.UserName
$env:ADMIN_PASSWORD = $AdminCredential.GetNetworkCredential().Password
try {
    cargo run --locked -p questionnaire-api -- create-admin
    if ($LASTEXITCODE -ne 0) { throw '管理员创建失败，请查看上方错误' }
} finally {
    Remove-Item Env:ADMIN_USERNAME, Env:ADMIN_PASSWORD
    Remove-Variable AdminCredential
}
```

环境变量仅供这次进程读取，不要长期写进 `.env`。管理员通过同一个 `/api/auth/login` 登录；改密会撤销该账号的**所有旧 Session，包括当前 Session**，所有设备都需要重新登录。

## 5. 可选：用 Compose 提供本机数据库

[compose.yaml](compose.yaml) **只提供 PostgreSQL 18，不启动 Rust 后端**。这条路径要求 Docker Engine / Docker Desktop 与 Compose 插件；当前开发机器没有 Docker，**Compose 启动路径未在本机运行验证**。

不要与本机 PostgreSQL 同时占用 `5432`。启动前必须预设 `POSTGRES_PASSWORD`：

```powershell
$DatabaseCredential = Get-Credential -UserName questionnaire -Message '设置容器中 questionnaire 数据库账号的密码'
$env:POSTGRES_PASSWORD = $DatabaseCredential.GetNetworkCredential().Password
try {
    docker compose up -d db
    if ($LASTEXITCODE -ne 0) { throw 'Compose 启动失败' }
    docker compose ps
} finally {
    Remove-Item Env:POSTGRES_PASSWORD
    Remove-Variable DatabaseCredential
}
```

把 `.env` 的 `DATABASE_URL` 配成同一账号、密码、数据库和 `127.0.0.1:5432`，再按第 3 节启动后端。数据库用命名卷保存，已有卷时改变 `POSTGRES_PASSWORD` **不会重置库内密码**；需要解析 Compose 文件的命令仍要按上面的方式提供该变量。不要用删除卷来“修复”密码或连接问题，卷里可能已有答卷。

## 6. 安全与使用边界

- 所有写请求（包括注册、登录和匿名提交）都要求 `Origin` 精确等于 `PUBLIC_ORIGIN`，并带 `X-CSRF-Protection: 1`。使用 `localhost` 就全程使用 `localhost`，不要与 `127.0.0.1` 混用 Cookie 域名。
- `.env.example` 里的 HTTP 与 `COOKIE_SECURE=false` **仅用于本机开发**。非本机部署必须使用 HTTPS，设置 `COOKIE_SECURE=true`，并把 `PUBLIC_ORIGIN` 设为真实的 HTTPS origin（无路径、无末尾 `/`）。当前 Rust 服务自身监听 HTTP；HTTPS/TLS 终止、反向代理与应用容器镜像不在项目范围内。
- 不开放跨域 CORS。前端开发及本机预览通过 Vite 同源代理访问 `/api`，来源校验始终开启。
- 限流是**单个服务实例的全局固定窗口**：`AUTH_REQUESTS_PER_MINUTE` 覆盖 `/api/auth/` 的写请求，`SUBMISSIONS_PER_MINUTE` 覆盖公开提交。它不是 IP 或账号身份限流，不实现严格一人一票，也不是多实例共享限流。
- 匿名模式不关联账号，但不承诺不可追踪，公开链接不等于保密访问控制；数据库运维权限与应用管理员权限是两回事。
- 答卷、CSV 与数据库备份含调查数据，须限制访问并安全保存；备份恢复操作及核对方法见[后端指南](docs/backend-guide.md#6-备份与独立恢复)。
- 性能压测和面向公众长期运营的隐私合规交付不在当前范围内。

## 7. 开发检查

检查按改动范围执行对应端的必跑门禁：同一份代码已经通过的门禁结果直接复用，commit、改分支和纯文档整理不触发重跑。测试保护当前业务契约与已确认缺陷，不按文件或函数数量补覆盖，纯前端改动不跑 Rust，纯后端改动不跑前端。

| 改动范围 | 对应检查 |
|---|---|
| 文档、注释、纯样式 | 审阅差异；样式按需核对受影响页面 |
| 前端纯逻辑或测试 | 在 `frontend/` 运行相关文件，例如 `pnpm run test:regression tests/editor.test.mjs` |
| Vue 组件或类型 | 按需运行 `pnpm run typecheck`；交互变化核对相关页面 |
| 前端依赖、打包或懒加载 | `pnpm run build` 已含类型检查，无需再跑 typecheck |
| Rust 领域逻辑或测试 | 从根目录运行相关目标，例如 `cargo test --locked -p questionnaire-api --test domain` |
| HTTP、权限或 SQL 行为 | 核对受影响的真实服务路径 |
| 共享 DSL 契约 | 运行受影响的两端测试，避免预览与服务端语义分离 |
| wasm 产物重建（`logic-core/` 或 `logic-wasm/`） | 在 `frontend/` 运行 `pnpm run wasm:build` 重建并同步产物（需 Rust 工具链 + `wasm32-unknown-unknown` target + wasm-pack 0.15），再跑受影响的两端测试；只改前端时无需 Rust，直接用 `frontend/src/wasm/pkg/` 里入库的产物 |

影响跨越多个模块时运行所属端的完整测试：Rust 为 `cargo test --locked --workspace`，前端为 `pnpm run test:regression`。修改 Rust 代码时必须运行 `cargo clippy --locked --workspace --all-targets -- -D warnings` 并对改动文件执行 `rustfmt --edition 2024`；修改前端代码时必须运行 `pnpm run typecheck`（或 `pnpm run build`）并对改动文件执行 `pnpm exec prettier --write`。

Rust 的四个测试目标位于 `backend/tests/`：`domain` 约束题型与规范化，`logic` 约束表达式与变量依赖，`flow` 约束路径、身份复制和最终提交，`logic_text` 约束文本解析。它们都不启动数据库，也不需要环境变量。

前端使用 Node 内置运行器自动发现 `frontend/tests/`，命令后传文件路径即可缩小范围，无需维护脚本内的文件清单。后端测试与前端（经 wasm 引擎）共用 [fixtures/survey-logic](fixtures/survey-logic) 中的人工期望：DSL 语义只有 `logic-core` 一份实现，共享样例用于确认两端边界一致；流程行为集中在共享样例，编辑器测试只补配置转换与同步等自身行为。`logic-wasm/tests/smoke.rs` 在原生目标覆盖同一 JSON 信封边界，随 `cargo test --locked --workspace` 一起执行。项目没有新增测试框架或自动提交钩子。

历史验证结果见[需求与验证记录](docs/requirements-analysis.md)，其中的全量命令和通过数量是当时的实施记录，不是后续修改必须重复的清单。

DSL 与流程引擎的[实施计划](docs/survey-dsl-plan.md)已完成，并在 2026-10-12 统一为 wasm 单实现：表达式、文档编译、流程与文本解析只有 `logic-core` 一份代码，`logic-wasm` 以 JSON 信封导出，`frontend/src/wasm/pkg/` 的产物入库，前端不再有 TS 镜像实现；文本语法契约见[实施计划](docs/survey-dsl-plan.md#文本语法契约阶段-6)。服务端重建有效路径，支持原提交载荷的幂等重放，并在详情、统计与 CSV 中区分跳过和未答。筛选结束的持久化策略、重复组和子流程仍留待后续，无规则问卷保持单页行为。当前尚无正式发布版本，后续开发不承担历史答卷或旧格式兼容要求。

两端的[输入契约回归](fixtures/survey-logic/input-contracts.json)固定 Unicode 空白和配置 UUID 的接受集：U+0085 是空白，U+FEFF 是内容；配置 UUID 必须非零、小写、带连字符。

### 7.1 编译速度配置

根 `Cargo.toml` 的 profile 与 `.cargo/config.toml` 的别名用于调整编译耗时和产物；release 还把 panic 策略改为直接终止进程。默认工具链仍是 stable：

```powershell
cargo build --locked -p questionnaire-api            # 开发构建：自己的代码 O0，依赖 O3
cargo build --release --locked -p questionnaire-api   # 发布构建：单 codegen 单元 + fat LTO + strip
```

- `dev`：自己的代码 `opt-level = 0`，第三方依赖 `opt-level = 3`。依赖几乎不改动，只有首次冷编译更慢；之后改业务代码的增量编译不受影响，同时 dev 下依赖（JSON、数据库、模板）的运行速度接近发布版。dev 保留 debuginfo，`gdb`/`lldb` 与 backtrace 照常可用。
- `release`：`opt-level = 3`、`codegen-units = 1`、`lto = "fat"` 换跨 crate 内联与死代码消除，`strip = "symbols"` 减体积，`panic = "abort"` 禁用 panic 栈展开。本机全量冷构建实测约 5 分钟，与不开 LTO 的旧配置持平，产物从 13.7 MB 降到 6.87 MB。发生 panic 时会直接终止进程，Tokio 任务边界与 `catch_unwind` 均无法恢复；第 7 节的普通测试使用 unwind 策略，通过测试不能验证 release 的 panic 行为。
- 需要更快的开发期反馈时，可以临时切到 nightly 的 Cranelift 后端（省掉 LLVM 优化管线，机器码更慢但语义等价）：

```powershell
rustup toolchain install nightly --component rustc-codegen-cranelift-preview
cargo +nightly cranelift -p questionnaire-api
```

Cranelift 别名内置 `--locked`，只作开发期可选路径，不写进 `rust-toolchain.toml`：默认的 `cargo build`、`cargo test`、`cargo clippy` 与发布构建仍使用 stable + LLVM。第一次在某个 `target/` 目录里用它要重建依赖（本机实测 222 个 crate、6 分 07 秒），之后两套产物并存，来回切换几乎不花时间（实测切回 stable 重编 0 个 crate、约 2 秒），`cargo clean` 后会重新付这 6 分钟。依赖里含 C 代码（`ring`，由 `sqlx` 的 rustls 特性引入）时，Cranelift 只作用于 Rust 代码，C 部分仍由系统编译器构建。

`.cargo/config.toml` 还把 cargo 子进程（build script、cc-rs、`cl.exe`）的 `TMP`/`TEMP` 固定到工作区内的 `target/`（用 `relative = true`，不写死盘符）：TMP/TEMP 指向 cargo 子进程写不了的位置（例如工作区外的目录或根本不存在的目录）时，编译 C 依赖会报 `error D8050: 无法执行 c1.dll`。临时目录需要放在别处时改这一处即可。
