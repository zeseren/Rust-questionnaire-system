# 问卷调查后端（Rust）

这是一个可通过 HTTP API 使用的问卷后端，适合从真实业务流程学习 Rust。**当前没有前端页面**：访问根路径不会出现管理界面，分享地址返回的是公开题面的 JSON，不是可点击填写的网页。

## 已实现的范围

- 普通账号注册、登录、退出、改密；Cookie Session；发布者只能管理自己的问卷和答卷。
- 六类题目：单选、多选、单行文本、多行文本、数字、评分；保存草稿、发布前校验、复制问卷。
- 不可变发布版本；匿名填写或同账号跨版本限交一次；截止时间、关闭、显式恢复、不可撤销归档。
- 完整答卷原子写入、UUIDv4 提交秘密和幂等重试；查看答卷、按版本统计、CSV 导出。
- 管理员禁用/启用账号、下架/恢复问卷及审计；管理员治理接口不提供他人的答卷。
- PostgreSQL 迁移、健康检查和 OpenAPI JSON。

不包含前端、性能压测或面向公众长期运营的隐私合规交付；不要把教学演示和知情同意的非敏感调查直接当作可公开运营的成品。范围和验收状态见 [需求分析](docs/requirements-analysis.md)，具体使用与代码阅读见 [后端指南](docs/backend-guide.md)。

## 1. 准备环境

以下命令使用 **Windows PowerShell**，从**仓库根目录（含 `Cargo.toml`、`.env.example`、`compose.yaml` 的目录）**运行，而不是 `backend/src`。

| 工具 | 要求 |
| --- | --- |
| Rust / Cargo | Rust **1.94 或更新版本**；crate 使用 Rust 2024 edition，最低版本以 `backend/Cargo.toml` 为准，不能只按 edition 推断编译器版本 |
| Windows 编译环境 | 安装 Rust MSVC 工具链所需的 Visual Studio C++ Build Tools 和 Windows SDK |
| PostgreSQL | 按本指南使用 **PostgreSQL 18**；服务端与 `psql`、`createdb`、`pg_dump`、`pg_restore` 客户端均准备好，并将客户端 `bin` 目录加入 PATH |
| PowerShell | Windows PowerShell 5.1 或 PowerShell 7；中文请求示例显式发送 UTF-8 字节 |

根目录是单成员 Cargo workspace，成员目录为 `backend`，包名为 `questionnaire-api`。当前锁定的核心依赖为 Axum 0.8.9、SeaORM / sea-orm-migration 2.0.2、SQLx 0.9.0、Tokio 1.53.1；准确版本以 [Cargo.lock](Cargo.lock) 为准，命令加 `--locked` 避免无意更新依赖。PostgreSQL 18 是本项目的运行基线，并非对所有旧大版本作兼容保证。

```powershell
rustc --version
cargo --version
psql --version
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

本机开发保留：

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
- 不开放跨域 CORS。将来接页面时，应通过同源代理让页面与 `/api` 使用同一 origin，而不是把来源校验关闭。
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

保留的 7 个关键测试集中在 [`backend/tests/domain.rs`](backend/tests/domain.rs)，不启动数据库；它们不能替代事务、权限和并发提交的真实服务验证。本轮运行过的 HTTP、数据库及备份恢复场景见 [实施与验证记录](docs/requirements-analysis.md#11-后端实施与验证记录)。
