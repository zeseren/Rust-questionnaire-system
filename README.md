# 问卷调查系统（Rust + Vue）

这是一个包含 Rust HTTP API 和 Vue 3 前端 MVP 的问卷调查系统，适合从真实业务流程学习前后端开发。前端位于 `frontend/`，采用 **Fluent 2** 风格：圆角控件、分级阴影、带轻微色调的 Mica 基底与纯色磁贴，支持浅色、深色和跟随系统。启动后从 `http://localhost:5173` 进入工作台。

## 已实现的范围

- 普通账号注册、登录、退出、改密；Cookie Session；发布者只能管理自己的问卷和答卷。
- 六类题目：单选、多选、单行文本、多行文本、数字、评分；保存草稿、发布前校验、复制问卷。
- 不可变发布版本；匿名填写或同账号跨版本限交一次；截止时间、关闭、显式恢复、不可撤销归档。
- 完整答卷原子写入、UUIDv4 提交秘密和幂等重试；查看答卷、按版本统计、CSV 导出。
- 管理员禁用/启用账号、下架/恢复问卷及审计；管理员治理接口不提供他人的答卷。
- PostgreSQL 迁移、健康检查和 OpenAPI JSON。
- 真实接口驱动的工作台、六题型编辑器、电脑/手机草稿预览、公开填写页、答卷统计及平台管理；无演示数据替代业务结果。

范围和验收状态见 [需求分析](docs/requirements-analysis.md)，使用与代码阅读见 [前端指南](docs/frontend-guide.md) 和 [后端指南](docs/backend-guide.md)。原有 [`frontend/prototype.html`](frontend/prototype.html) 与 [`frontend/fluent-prototype.html`](frontend/fluent-prototype.html) 保留为独立风格原型（后者对应当前 Fluent 2 视觉）；实际应用入口为 `frontend/index.html`。性能压测和面向公众长期运营的隐私合规交付不在当前范围内。

## 1. 准备环境

以下命令使用 **Windows PowerShell**，从**仓库根目录（含 `Cargo.toml`、`.env.example`、`compose.yaml` 的目录）**运行，而不是 `backend/src`。

| 工具 | 要求 |
| --- | --- |
| Rust / Cargo | Rust **1.94 或更新版本**；crate 使用 Rust 2024 edition，最低版本以 `backend/Cargo.toml` 为准，不能只按 edition 推断编译器版本 |
| Windows 编译环境 | 安装 Rust MSVC 工具链所需的 Visual Studio C++ Build Tools 和 Windows SDK |
| PostgreSQL | 按本指南使用 **PostgreSQL 18**；服务端与 `psql`、`createdb`、`pg_dump`、`pg_restore` 客户端均准备好，并将客户端 `bin` 目录加入 PATH |
| Node.js / npm | 推荐 Node.js **24.12 或更新版本**，也支持 22.18+ 的 22.x；准确范围见 `frontend/package.json`，依赖由 `package-lock.json` 锁定 |
| PowerShell | Windows PowerShell 5.1 或 PowerShell 7；中文请求示例显式发送 UTF-8 字节 |

根目录是单成员 Cargo workspace，成员目录为 `backend`，包名为 `questionnaire-api`。当前锁定的核心依赖为 Axum 0.8.9、SeaORM / sea-orm-migration 2.0.2、SQLx 0.9.0、Tokio 1.53.1；准确版本以 [Cargo.lock](Cargo.lock) 为准，命令加 `--locked` 避免无意更新依赖。PostgreSQL 18 是本项目的运行基线，并非对所有旧大版本作兼容保证。

```powershell
rustc --version
cargo --version
psql --version
node --version
npm --version
```

## 2. 本机数据库：创建专用账号和空库

先启动已安装的 PostgreSQL 服务。下面用安装时的数据库管理员 `postgres` 建账号；`-W` 会交互提示密码，不把密码放进命令行。**不要让应用日常使用超级用户账号。**

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

如果已有同名账号或数据库，不要重复创建或删除已有数据；改用已有的专用账号/库，并同步调整连接配置。

## 3. 配置、迁移和启动

首次配置时复制模板；已有 `.env` 时直接编辑，不要覆盖：

```powershell
Copy-Item .env.example .env
notepad .env
```

修改 `DATABASE_URL`，将模板密码替换成刚设置的数据库密码。连接串中的密码若含 `@`、`:`、`/`、`#`、`%` 等字符，必须进行 URI 百分号编码；它不是普通原样拼接的文本。`.env` 含数据库凭据，只保留在本机，不提交、不截图分享。

仅使用 API 时可保留以下默认配置；同时运行前端时，将 `PUBLIC_ORIGIN` 改为 `http://localhost:5173`：

```dotenv
LISTEN_ADDR=127.0.0.1:3000
PUBLIC_ORIGIN=http://localhost:3000
COOKIE_SECURE=false
SESSION_TTL_SECONDS=86400
AUTH_REQUESTS_PER_MINUTE=60
SUBMISSIONS_PER_MINUTE=600
RUST_LOG=questionnaire_api=info
```

应用通过 `dotenvy` 读取根目录 `.env`；已经设置的进程环境变量优先于文件。若连接到了意外的数据库，检查当前终端是否遗留了 `DATABASE_URL` 环境变量，不要直接在不确定的库上操作。

```powershell
# 只执行迁移，然后退出
cargo run --locked -p questionnaire-api -- migrate

# 启动 HTTP 服务，保持此终端运行；Ctrl+C 停止
cargo run --locked -p questionnaire-api
```

默认命令是 `serve`，也可写 `cargo run --locked -p questionnaire-api -- serve`。**服务启动本身也会先运行迁移**，因此无需每次手工执行 `migrate`；迁移版本由 SeaORM 记录。数据库不可达或配置不合法会直接报错，不会自动创建数据库或切换到内存存储。

另开一个仍在仓库根目录的 PowerShell：

```powershell
Invoke-RestMethod http://localhost:3000/health
$OpenApi = Invoke-RestMethod http://localhost:3000/api/openapi.json
$OpenApi.info
```

`/health` 成功应返回 `status = ready`，含义是数据库连接就绪，不等于所有业务验收通过。完整可复制流程在 [后端指南：最小真实闭环](docs/backend-guide.md#1-powershell-最小真实闭环)。

### 3.1 启动前端 MVP

在根目录 `.env` 设置 `PUBLIC_ORIGIN=http://localhost:5173`、`COOKIE_SECURE=false`，然后启动或重启后端。另开 PowerShell，在仓库根目录运行：

```powershell
Set-Location frontend
npm ci
npm run dev
```

打开 **http://localhost:5173**，注册账号后即可创建、编辑、发布问卷。发布后的填写链接为 `/s/:id`，可复制给填写者；前端分享的是网页，后端响应中的 `share_url` 仍是公开 JSON 接口路径。管理员初始化沿用第 4 节，登录后会显示“平台管理”。

开发服务器固定使用 `localhost:5173`，将 `/api` 代理到 `http://127.0.0.1:3000`，保留浏览器的 `Origin` 和 Cookie。后端使用其他地址时，可复制 `frontend/.env.example` 为 `frontend/.env` 并修改 `API_PROXY_TARGET`，或在运行 `npm run dev` 的终端设置该环境变量。它仅由 Vite 服务器读取；不要用 `VITE_` 环境变量存放密钥。

构建与本机预览也在 `frontend/` 中执行：

```powershell
npm run build
# 先停止 npm run dev，避免占用同一端口
npm run preview
```

`build` 包含 TypeScript 检查，生成 `frontend/dist/`；`preview` 使用相同的 `5173` 端口与 API 代理，仍需运行后端及数据库。不能通过双击 `dist/index.html` 使用实际应用。前端不由 Rust 服务托管，访问后端根路径不会显示工作台。

## 4. 按需初始化管理员

普通注册不能指定管理员角色。下面的命令**只新建管理员，不会升级或覆盖已有账号**。仍在仓库根目录运行，使用交互凭据窗口，不写示例密码，也不把密码作为 CLI 参数：

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

环境变量仅供这次进程读取，不应长期放入 `.env`。它们仍是进程内的敏感数据，这种写法避免命令历史和进程参数出现明文，不代表内存中没有明文。管理员通过同一个 `/api/auth/login` 登录；密码修改会撤销该账号**所有旧 Session，包括当前 Session**，所有设备都必须重新登录。

## 5. 可选：用 Compose 提供本机数据库

[compose.yaml](compose.yaml) **只提供 PostgreSQL 18，不启动 Rust 后端**。此路径要求 Docker Engine / Docker Desktop 与 Compose 插件；当前开发机器没有 Docker，**Compose 启动路径未在本机运行验证**。

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

将 `.env` 的 `DATABASE_URL` 配成同一账号、密码、数据库和 `127.0.0.1:5432`，再按第 3 节启动后端。数据库用命名卷保存；已有卷时，改变 `POSTGRES_PASSWORD` **不会重置库内密码**。后续执行需要解析 Compose 文件的命令时，仍须按上述方式提供该变量。不要用删除卷来“修复”密码或连接问题，卷里可能已有答卷。

## 6. 使用边界

- 所有写请求，包括注册、登录和匿名提交，都要求 `Origin` 精确等于 `PUBLIC_ORIGIN`，并带 `X-CSRF-Protection: 1`。使用 `localhost` 就全程使用 `localhost`，不要与 `127.0.0.1` 混用 Cookie 域名。
- `.env.example` 的 HTTP 与 `COOKIE_SECURE=false` **仅用于本机开发**。非本机部署必须使用 HTTPS，设置 `COOKIE_SECURE=true`，并将 `PUBLIC_ORIGIN` 设为真实的 HTTPS origin（无路径、无末尾 `/`）。当前 Rust 服务自身监听 HTTP；HTTPS/TLS 终止、反向代理与应用容器镜像不在项目范围内。
- 不开放跨域 CORS。前端开发及本机预览通过 Vite 同源代理访问 `/api`；保持来源校验开启。
- 限流是**单个服务实例的全局固定窗口**：`AUTH_REQUESTS_PER_MINUTE` 覆盖 `/api/auth/` 的写请求；`SUBMISSIONS_PER_MINUTE` 覆盖公开提交。它不是 IP/账号身份限流，不实现严格一人一票，也不是多实例共享限流。
- 匿名模式不关联账号，但不承诺不可追踪；公开链接不等于保密访问控制。数据库运维权限不同于应用管理员权限。
- 答卷、CSV 与数据库备份含调查数据，须限制访问并安全保存。备份恢复操作及核对方法见 [后端指南](docs/backend-guide.md#6-备份与独立恢复)。

## 7. 开发检查

从仓库根目录执行：

```powershell
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

领域回归测试位于 [`backend/tests/domain.rs`](backend/tests/domain.rs)，DSL 表达式与文档编译回归位于 [`backend/tests/logic.rs`](backend/tests/logic.rs)，流程与改答清理回归位于 [`backend/tests/flow.rs`](backend/tests/flow.rs)，均不启动数据库；它们不能替代事务、权限和并发提交的真实服务验证。运行过的 HTTP、数据库及备份恢复场景见 [实施与验证记录](docs/requirements-analysis.md#11-后端实施与验证记录)。

前端在 `frontend/` 执行 `npm run build` 检查类型并构建，`npm run format` 整理源码，`npm run format:check` 检查格式。`npm run test:regression` 使用 Node 内置测试运行器执行 `frontend/tests/` 中的数字精度、表达式、文档规则与流程回归，不添加测试依赖。两端 DSL 测试读取同一目录下的[表达式样例](fixtures/survey-logic/expressions.json)、[文档样例](fixtures/survey-logic/documents.json)、[流程样例](fixtures/survey-logic/flows.json)与[最终提交样例](fixtures/survey-logic/submissions.json)。没有引入前端自动化测试框架；实际浏览器联调范围单独记录在 [前端实施与验证记录](docs/requirements-analysis.md#12-前端-mvp-实施与验证记录)，不将静态原型视为业务验收。

DSL 与流程引擎正在按[实施计划](docs/survey-dsl-plan.md)分阶段开发。第四阶段已支持普通完成流程的服务端发布与提交、历史版本路径重建、幂等重放，以及详情/统计/CSV 的适用性区分。筛选结束流程暂不开放发布；网页规则编辑、分步预览和填写留到第五阶段，现有网页对带规则文档提示暂未开放。无规则问卷保持原有行为。

### 7.1 编译速度配置

根 `Cargo.toml` 的 profile 与 `.cargo/config.toml` 的别名用于调整编译耗时和产物；release 还将 panic 策略改为直接终止进程。默认工具链仍是 stable：

```powershell
cargo build --locked -p questionnaire-api            # 开发构建：自己的代码 O0，依赖 O3
cargo build --release --locked -p questionnaire-api   # 发布构建：单 codegen 单元 + fat LTO + strip
```

- `dev`：自己的代码 `opt-level = 0`，第三方依赖 `opt-level = 3`。依赖几乎不改动，只有首次冷编译更慢；之后改业务代码的增量编译不受影响，同时 dev 下依赖（JSON、数据库、模板）的运行速度接近发布版。dev 保留 debuginfo，`gdb`/`lldb` 与 backtrace 照常可用。
- `release`：`opt-level = 3`、`codegen-units = 1`、`lto = "fat"` 换跨 crate 内联与死代码消除，`strip = "symbols"` 减体积，`panic = "abort"` 禁用 panic 栈展开。本机全量冷构建实测约 5 分钟，与不开 LTO 的旧配置持平，产物从 13.7 MB 降到 6.87 MB。发生 panic 时会直接终止进程，Tokio 任务边界与 `catch_unwind` 均无法恢复；第 7 节的普通测试使用 unwind 策略，通过测试不能验证 release 的 panic 行为。
- 需要更快的开发期反馈时，可临时切到 nightly 的 Cranelift 后端（省掉 LLVM 优化管线，机器码更慢但语义等价）：

```powershell
rustup toolchain install nightly --component rustc-codegen-cranelift-preview
cargo +nightly cranelift -p questionnaire-api
```

Cranelift 别名内置 `--locked`，只作开发期可选路径，不写进 `rust-toolchain.toml`：默认 `cargo build`、`cargo test`、`cargo clippy` 与发布构建仍使用 stable + LLVM。第一次在某个 `target/` 目录里用它要重建依赖（本机实测 222 个 crate、6 分 07 秒），之后两套产物并存，来回切换几乎不花时间（实测切回 stable 重编 0 个 crate、约 2 秒），`cargo clean` 后会重新付这 6 分钟。依赖里含 C 代码（`ring`，由 `sqlx` 的 rustls 特性引入）时，Cranelift 只作用于 Rust 代码，C 部分仍由系统编译器构建。

`.cargo/config.toml` 还把 cargo 子进程（build script、cc-rs、`cl.exe`）的 `TMP`/`TEMP` 固定到工作区内的 `target/`（用 `relative = true`，不写死盘符）：TMP/TEMP 指向 cargo 子进程写不了的位置（例如工作区外的目录或根本不存在的目录）时，编译 C 依赖会报 `error D8050: 无法执行 c1.dll`。临时目录需要放在别处时改这一处即可。
