# 后端开发指南

本文面向开发与维护者，完整说明 `questionnaire-api`（`backend/`）与 `questionnaire-logic`（`logic-core/`）的请求链路、数据库 Schema、版本与并发锁序、数据校验与统计口径、API 路由速查以及数据库备份恢复流程。

环境配置与快速启动见 [README](../README.md)。

---

## 1. 工作区结构与模块导读

后端位于三成员 Cargo workspace 中，严格分为「HTTP 边界 → 业务用例 → 领域核心 / 数据访问 → PostgreSQL 18」：

| 位置 | 包名 / 模块 | 职责说明 |
| --- | --- | --- |
| `backend/src/main.rs`、`lib.rs` | `questionnaire-api` | 二进制入口、组合根 `run()`、共享状态 `AppState`、OpenAPI 路由合并与 `/health` |
| `backend/src/config.rs` | 配置边界 | 解析并校验环境变量（`DATABASE_URL`、`LISTEN_ADDR`、`PUBLIC_ORIGIN`、`COOKIE_SECURE` 等） |
| `backend/src/error.rs`、`http.rs` | 协议边界 | 统一错误类型 `AppError`（含 500 `X-Error-Id` 追踪）；`ApiJson` / `ApiQuery` / `ApiPath` 抽取器與 `Pagination` |
| `backend/src/security.rs`、`auth.rs` | 安全与鉴权 | 同源 `Origin` + `X-CSRF-Protection: 1` 写请求守卫、固定窗口限流、`no-store`/`nosniff` 响应头；Argon2id 密码哈希与 Cookie Session |
| `backend/src/entities.rs`、`repository.rs` | 持久化与锁序 | 6 张表的 SeaORM Entity 定义；归属查询、只读快照与固定的「先 user 后 survey」行锁助手 |
| `backend/src/migration.rs`、`schema.sql` | 迁移与 DDL | 唯一权威 PostgreSQL 建表 DDL（服务启动时自动执行 `Migrator::up`） |
| `backend/src/surveys.rs` | 发布者生命周期 | 创建问卷、乐观锁草稿保存、预览、整卷复制、幂等发布、回收设置、关闭/恢复/归档与删除 |
| `backend/src/submissions.rs` | 公开填写与提交 | 只读快照读取公开题面（`GET /api/public/surveys/{id}`）；加锁、重验 Session、路径重建与幂等提交（`POST .../responses`） |
| `backend/src/reports.rs` | 报表与导出 | 按版本与 `[from, to)` 时间筛选的答卷分页列表、单份详情、三态统计聚合与防公式注入 CSV 导出 |
| `backend/src/admin.rs` | 平台治理 | 管理员分页查看用户/问卷/审计日志，同事务执行账号禁用/启用与问卷下架/恢复并记录原因 |
| `logic-core/src/*` | `questionnaire-logic` | 纯领域核心（`domain.rs`、`identity.rs`、`logic.rs`、`logic_document.rs`、`flow.rs`、`logic_text.rs`）；由 `backend/src/lib.rs` 通过 `pub use questionnaire_logic::domain;` 聚合再导出 |
| `logic-core/tests/`、`logic-wasm/tests/`、`backend/src/tests/` | 测试分层 | 领域契约集成测试、WASM JSON 信封边界测试与后端模块化单元/HTTP 边界测试；全部纯内存运行 |

---

## 2. PowerShell 端到端真实闭环

以下脚本在已启动后端服务（默认 `http://localhost:3000`）的前提下，演示从注册、登录、创建含六种题型的问卷、设置回收模式、发布版本、匿名幂等提交到查询统计与导出 CSV 的完整闭环：

```powershell
$Base = 'http://localhost:3000'
$Origin = 'http://localhost:5173' # 须与后端 .env 中的 PUBLIC_ORIGIN 完全一致
$WriteOpts = @{
    Headers     = @{ Origin = $Origin; 'X-CSRF-Protection' = '1' }
    ContentType = 'application/json; charset=utf-8'
    ErrorAction = 'Stop'
}
function To-Utf8Json($Obj) {
    [System.Text.Encoding]::UTF8.GetBytes(($Obj | ConvertTo-Json -Depth 20 -Compress))
}

# 1. 注册并登录发布者账号
$AuthPayload = To-Utf8Json @{
    username = 'demo_' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    password = 'demo_password_123'
}
Invoke-RestMethod -Method Post -Uri "$Base/api/auth/register" @WriteOpts -Body $AuthPayload | Out-Null
Invoke-RestMethod -Method Post -Uri "$Base/api/auth/login" @WriteOpts -Body $AuthPayload -SessionVariable PubSession | Out-Null

# 2. 创建包含六类题型的问卷草稿
$Q1 = [guid]::NewGuid().ToString(); $OptA = [guid]::NewGuid().ToString(); $OptB = [guid]::NewGuid().ToString()
$Q2 = [guid]::NewGuid().ToString(); $OptC = [guid]::NewGuid().ToString(); $OptD = [guid]::NewGuid().ToString()
$Q3 = [guid]::NewGuid().ToString(); $Q4 = [guid]::NewGuid().ToString()
$Q5 = [guid]::NewGuid().ToString(); $Q6 = [guid]::NewGuid().ToString()

$Survey = Invoke-RestMethod -Method Post -Uri "$Base/api/surveys" @WriteOpts -WebSession $PubSession -Body (To-Utf8Json @{
    document = @{
        title       = '课程与工具链体验调查'
        description = '用于本机端到端接口验证。'
        purpose     = '验证六类题型、版本发布与统计导出的完整闭环。'
        thank_you   = '感谢您的参与！'
        questions   = @(
            @{ id = $Q1; title = '整体评价'; required = $true; type = 'single_choice'; options = @(@{ id = $OptA; label = '满意' }, @{ id = $OptB; label = '需改进' }) }
            @{ id = $Q2; title = '关注模块'; required = $true; type = 'multiple_choice'; min_choices = 1; max_choices = 2; options = @(@{ id = $OptC; label = 'DSL 引擎' }, @{ id = $OptD; label = '统计导出' }) }
            @{ id = $Q3; title = '一句话总结'; required = $false; type = 'text'; max_length = 200 }
            @{ id = $Q4; title = '详细建议'; required = $false; type = 'textarea'; max_length = 1000 }
            @{ id = $Q5; title = '每周使用时长（小时）'; required = $true; type = 'number'; integer = $false; min = 0; max = 168 }
            @{ id = $Q6; title = '易用性评分'; required = $true; type = 'rating'; min = 1; max = 5; min_label = '困难'; max_label = '顺畅' }
        )
    }
})
$SurveyId = $Survey.id

# 3. 设置公开匿名模式并发布 V1
Invoke-RestMethod -Method Patch -Uri "$Base/api/surveys/$SurveyId/collection" @WriteOpts -WebSession $PubSession -Body (To-Utf8Json @{
    mode = 'anonymous'; deadline = $null
}) | Out-Null
$Published = Invoke-RestMethod -Method Post -Uri "$Base/api/surveys/$SurveyId/publish" @WriteOpts -WebSession $PubSession -Body (To-Utf8Json @{
    revision = $Survey.revision
})
$VersionId = $Published.version.id

# 4. 匿名读取公开题面并提交答卷（附幂等重放验证）
$Public = Invoke-RestMethod -Uri "$Base/api/public/surveys/$SurveyId"
$SubmitPayload = To-Utf8Json @{
    version_id      = $Public.version_id
    idempotency_key = [guid]::NewGuid().ToString()
    answers         = @(
        @{ question_id = $Q1; value = $OptA }
        @{ question_id = $Q2; value = @($OptC, $OptD) }
        @{ question_id = $Q3; value = '响应迅速' }
        @{ question_id = $Q4; value = "纸本排版清晰。`n规则校验准确。" }
        @{ question_id = $Q5; value = 12.5 }
        @{ question_id = $Q6; value = 5 }
    )
}
$Receipt1 = Invoke-RestMethod -Method Post -Uri "$Base/api/public/surveys/$SurveyId/responses" @WriteOpts -Body $SubmitPayload
$Receipt2 = Invoke-RestMethod -Method Post -Uri "$Base/api/public/surveys/$SurveyId/responses" @WriteOpts -Body $SubmitPayload
# $Receipt1.receipt_id 与 $Receipt2.receipt_id 完全相同（首次 201，重试 200）

# 5. 查看分版本统计与答卷详情
$Stats = Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/statistics?version_id=$VersionId" -WebSession $PubSession
$Stats | Select-Object version_number, total_count, timezone
```

---

## 3. 数据库 Schema、版本隔离与并发控制

### 3.1 六张核心业务表（`backend/src/schema.sql`）

| 表名 | 核心字段与数据库级约束 |
| --- | --- |
| `users` | `id` (UUID PK)、唯一小写 `username`、Argon2id `password_hash`、`role` (`user`/`admin`)、`disabled`、`created_at` |
| `sessions` | `id_hash` (SHA-256 PK)、`user_id` (FK → `users`)、`expires_at`、`created_at` |
| `surveys` | `id` (UUID PK)、`owner_id` (FK → `users`)、`status` (`unpublished`/`collecting`/`closed`/`archived`)、`mode` (`anonymous`/`login_once`)、`deadline`、`taken_down`、`draft` (JSONB)、`revision` (乐观锁版本号)、`current_version_id` |
| `versions` | `id` (UUID PK)、`survey_id` (FK → `surveys`)、`number` (递增版本号，`(survey_id, number)` 唯一)、不可变快照 `document` (JSONB)、`published_at`；与 `surveys(id, current_version_id)` 构成复合外键 |
| `responses` | `id` (回执 UUID PK)、`survey_id`、`version_id`（复合外键 → `versions(survey_id, id)`）、`respondent_id`（部分唯一索引 `responses_once_per_account(survey_id, respondent_id) WHERE respondent_id IS NOT NULL`）、`scope`、`key_hash`（`responses_idempotency_unique(survey_id, scope, key_hash)`）、`payload_hash`、`answers` (JSONB)、`submitted_at` |
| `audits` | `id` (UUID PK)、`actor_id`、`action`、`target_type`、`target_id`、`reason` (1–500 字符)、`created_at` |

### 3.2 草稿 `revision` 与发布版本 `version`

1. **草稿乐观锁**：`surveys.revision` 从 `1` 起始，每次 `PUT /api/surveys/{id}/draft` 保存成功后递增。保存与发布均需传入客户端持有的 `revision`，不匹配时返回 `409 revision_conflict`。
2. **发布顺序与幂等重放**：`POST /api/surveys/{id}/publish` 在持锁事务内校验可编辑状态、`revision` 一致性及 `mode` 非空后，先比对当前草稿与 `current_version_id` 指向的版本内容——若完全相同则直接返回 `200 OK` 与当前版本（不新增版本号，且先于发布内容校验）；否则执行 `Document::prepare_publish()`，插入 `versions` 新行（`number = max + 1`）并更新 `current_version_id`，返回 `201 Created`。
3. **状态正交性**：
   - 截止时间（`deadline`）到期后问卷自动按 `closed` 处理；后续延长或清空 `deadline` 不会自动重开回收，必须显式调用 `POST .../resume`。
   - 管理员下架标志 `taken_down` 与发布者的 `status` 相互独立：发布者 `resume` 不会解除 `taken_down`，管理员恢复问卷也不会自动开启已关闭或已归档的问卷。

### 3.3 并发控制与无锁只读查询

- **按需问卷行锁**：发布者修改问卷（保存草稿、发布、关闭/恢复/归档）在事务内通过 `repository::lock_owned_survey_meta` 直接对 `surveys` 元数据行加 `FOR UPDATE` 排他锁并核对归属；公开提交答卷对 `surveys` 行加 `FOR SHARE` 共享锁，既与关闭/下架互斥，又允许同一问卷的并发答卷提交并行执行。
- **无锁只读查询**：公开读题面（`GET /api/public/surveys/{id}`）、问卷列表、答卷列表、统计、CSV 导出与管理员列表直接通过连接池执行普通只读查询，不开启长事务或持有行锁。列表查询通过 JSONB 字段投影（`cast_json_field("title")`）仅读取标题，不反序列化整份 `Document`。

### 3.4 幂等提交判定流程

`POST /api/public/surveys/{id}/responses` 在持锁事务内按以下顺序处理：

1. 校验 `idempotency_key` 必须为标准随机 UUIDv4（否则 `422`）。
2. 计算幂等作用域 `scope`（匿名模式固定为 `"anonymous"`，登录限次模式为填写者用户 UUID）与 `key_hash = SHA256(idempotency_key)`，查询 `(survey_id, scope, key_hash)` 是否已有历史记录。
3. 若无历史记录，则校验问卷当前可回收状态、`version_id == current_version_id`（不一致返回 `409 version_changed`）及登录限次唯一资格；若已有历史记录且 `version_id` 一致，则允许进入重放比对（即使问卷此后已关闭或切换了新版本）。
4. 使用目标版本的不可变 `Document` 校验并规范化答案（带规则版本通过 `FlowEvaluator` 从入口重建有效路径），计算规范化载荷哈希 `payload_hash`。
5. 若命中历史记录且 `payload_hash` 一致，返回 `200 OK` 与原 `receipt_id`；若 `payload_hash` 不同，返回 `409 idempotency_conflict`；若为首次提交，则插入 `responses` 并返回 `201 Created`。

---

## 4. 数据校验、统计与 CSV 导出口径

### 4.1 身份、文本与数值契约

- **严格 UUID 身份**（`logic-core/src/identity.rs`）：配置中的题目、选项、计算变量、规则、流程节点，以及作答中的 `question_id` 与选择题选项值，统一只接受 **36 字节、非全零、小写连字符 UUID**。
- **文本与空白**：长度按 Unicode 标量值（`str::chars().count()`）统计；空白判定遵循 Unicode `White_Space`（`U+0085` 为空白，`U+FEFF` 为非空白内容）；全面拒绝含 `U+0000` (NUL) 的字符串。
- **数值语义**：统一使用有限 `f64`（IEEE 754 binary64），拒绝 `NaN` 与 `Infinity`；`1` 与 `1.0`、`-0.0` 与 `0.0` 归一为同一数值 `0.0`；评分题分值同时受 `i32` 与题面闭区间 `[min, max]` 约束。

### 4.2 分版本三态统计口径

统计（`GET /api/surveys/{id}/statistics`）与导出一次只针对单个发布版本（默认当前版本）及左闭右开时间区间 `[from, to)`（RFC3339，精度至微秒；展示时区固定为 `Asia/Shanghai` `+08:00`）：

- **三态恒等式**：通过该历史版本的 `FlowEvaluator` 重建每份答卷中各题的适用状态：
  - `answered`（已答）、`unanswered`（路径内适用但未答）、`skipped`（被显示规则或分支跳过）。
  - 恒满足：`applicable_count = answered_count + unanswered_count`，且 `total_count = applicable_count + skipped_count`。
- **分母与空值**：选择题选项 `proportion = count / answered_count`（当 `answered_count == 0` 时返回 `null`）；数字题与评分题的均值/最值仅对 `answered` 样本计算（含合法 `0` 值，无有效回答时返回 `null`）。

### 4.3 CSV 导出与电子表格公式注入防护

- UTF-8 编码、CRLF 换行；表头依次为 `receipt_id`、`version_id`、`version_number`、`submitted_at`，随后为每道题的 `标题 [题目 UUID]`（有规则版本在每题后紧跟 `状态 [题目 UUID]` 列，取值为 `已答` / `适用未答` / `规则跳过`）。
- 多选题答案在单个单元格内编码为 JSON 字符串数组（如 `["uuid1","uuid2"]`）。
- **公式防护（`formula_safe`）**：任何单元格（含表头）若以 `=`、`+`、`-'、`@` 开头，或在前导空白/控制字符后出现上述字符，导出时自动在字段最前方前置单引号 `'`，防止 Excel / WPS 将用户文本作为公式执行。

---

## 5. API 路由与错误码速查

完整 OpenAPI 3.1 规范可在启动服务后访问 `GET /api/openapi.json` 获取。

### 5.1 路由总表

| 模块 | 方法 | 路径 | 说明 |
| --- | --- | --- | --- |
| 系统 | `GET` | `/health`、`/api/openapi.json` | 数据库就绪探针、OpenAPI 3.1 文档 |
| 认证 | `POST` | `/api/auth/register`、`/api/auth/login`、`/api/auth/logout` | 注册普通用户、登录下发 Cookie、退出销毁当前 Session |
| 认证 | `GET` / `PUT` | `/api/auth/me`、`/api/auth/password` | 获取当前登录用户、修改密码并撤销该账号全部 Session |
| 问卷 | `GET` / `POST` | `/api/surveys` | 本人问卷分页列表（支持 `title`、`status`）/ 创建新问卷草稿 |
| 问卷 | `GET` / `DELETE` | `/api/surveys/{id}` | 获取问卷详情與草稿 / 删除从未发布的草稿 |
| 问卷 | `PUT` / `GET` | `/api/surveys/{id}/draft`、`/api/surveys/{id}/preview` | 保存草稿（校验 `revision`）/ 预览已保存草稿与发布校验结果 |
| 问卷 | `POST` | `/api/surveys/{id}/copy`、`/api/surveys/{id}/publish` | 复制为全新草稿 / 发布指定 `revision` 为不可变版本 |
| 问卷 | `PATCH` | `/api/surveys/{id}/collection` | 更新填写模式（未发布前）与截止时间（`deadline` 三态语义） |
| 问卷 | `POST` | `/api/surveys/{id}/close`、`.../resume`、`.../archive` | 关闭回收 / 显式恢复回收 / 不可撤销归档 |
| 报表 | `GET` | `/api/surveys/{id}/versions`、`.../responses`、`.../responses/{receipt_id}` | 版本列表、答卷分页列表、含原版本题面与三态的答卷详情 |
| 报表 | `GET` | `/api/surveys/{id}/statistics`、`.../export.csv` | 分版本统计聚合、UTF-8 CSV 导出 |
| 公开 | `GET` / `POST` | `/api/public/surveys/{id}`、`.../responses` | 读取当前可填题面与隐私说明 / 幂等提交答卷 |
| 治理 | `GET` / `PATCH` | `/api/admin/users`、`/api/admin/users/{id}` | 用户分页列表 / 禁用或启用账号（必填 `reason`） |
| 治理 | `GET` / `PATCH` | `/api/admin/surveys`、`/api/admin/surveys/{id}` | 问卷治理列表、查看已发布题面 / 下架或恢复问卷（必填 `reason`） |
| 治理 | `GET` | `/api/admin/audits` | 治理审计日志分页列表 |

### 5.2 错误响应契约

所有错误均返回结构化信封 `{"error":{"code":"...","message":"...","question_id":null}}`：

| HTTP 状态码 | 典型 `code` | 触发场景 |
| --- | --- | --- |
| `400 Bad Request` | `invalid_json`、`invalid_path`、`invalid_query` | JSON 语法/类型不匹配、包含未知字段、非规范 UUID |
| `401 Unauthorized` | `unauthorized` | 未登录、Session 过期或被撤销、密码错误 |
| `403 Forbidden` | `csrf_rejected`、`forbidden`、`account_disabled` | `Origin` 不匹配或缺少 `X-CSRF-Protection: 1`、非管理员访问 `/api/admin/*` |
| `404 Not Found` | `not_found` | 资源不存在或不属于当前发布者 |
| `409 Conflict` | `revision_conflict`、`mode_frozen`、`version_changed`、`idempotency_conflict`、`already_submitted`、`survey_not_collecting` | 草稿修订号过期、首次发布后改模式、提交版本已过期、同幂等键载荷不同、登录限次已交过、问卷已关闭/到期/下架 |
| `413 Payload Too Large` | `payload_too_large` | 请求体超过 `1 MiB`（`MAX_SUBMISSION_BYTES`） |
| `422 Unprocessable Entity` | `validation_failed` | 题面/规则/流程/答案校验未通过（题目级错误携带 `question_id`） |
| `429 Too Many Requests` | `rate_limited` | 触发认证或公开提交固定窗口限流 |
| `500 Internal Server Error` | `internal_error` | 内部异常（响应头附带 `X-Error-Id`，不泄露 SQL/ORM 原文） |
| `503 Service Unavailable` | `not_ready` | `/health` 探测数据库不可达 |

---

## 6. 测试结构

测试按「模块内单元 → 跨模块契约 → 边界信封」三档组织，`cargo test --locked --workspace` 全部执行，无需数据库或环境变量：

| 目标 / 文件 | 覆盖内容 |
| --- | --- |
| `logic-core/src/tests/{identity,domain,logic,logic_document,flow,logic_text}.rs` | 模块内部不变量：身份严格解析、表达式类型检查与三值求值、变量拓扑排序、流程状态机、文本 DSL 词法与格式化 |
| `logic-core/tests/domain.rs` | 文档与答案的草稿/发布校验、容量边界与题目级错误定位 |
| `logic-core/tests/logic.rs` | 与 `fixtures/survey-logic/expressions.json` 共读的单表达式语义 |
| `logic-core/tests/flow.rs` | 与 `fixtures/survey-logic/flows.json` 共读的前向流程、改答清理与路径重建 |
| `logic-core/tests/logic_text.rs` | 与 `fixtures/survey-logic/texts.json` 共读的文本 DSL 解析、错误行列与格式化往返 |
| `logic-wasm/tests/smoke.rs` | WASM JSON 字符串信封：成功值解码、错误对象字段与领域错误分类穿透 |
| `backend/src/tests/config.rs` | 环境变量解析、默认值与非法值边界 |
| `backend/src/tests/error_and_http.rs` | `AppError` 信封、状态码映射、抽取器与分页参数 |
| `backend/src/tests/security_and_router.rs` | CSRF/Origin 守卫、固定窗口限流、安全响应头与路由合并 |
| `backend/src/tests/auth_and_admin.rs` | 纯策略与 DTO 边界：用户名/密码规则、Argon2id 哈希验证、Session Cookie 解析与生成、管理员角色与治理原因校验 |
| `backend/src/tests/surveys_and_submissions.rs` | 纯策略与契约：有效状态计算、实体枚举与 `schema.sql` 对齐、回收资格优先级、乐观锁与状态转移矩阵、模式冻结、UUIDv4 幂等键与提交载荷哈希 |
| `backend/src/tests/reports.rs` | 三态统计恒等式、时间区间筛选与 CSV 公式注入防护 |

编写约定：单元测试只验证本模块不变量，跨模块语义放进 `logic-core/tests/` 的契约集成测试，服务端真实 HTTP 路由与中间件行为在 `backend/src/tests/` 断言，避免同一路径被多层重复覆盖。

---

## 7. 数据库备份与独立恢复

1. **导出自定义格式备份**（勿通过 PowerShell 文本管道重定向二进制流）：
   ```powershell
   pg_dump -h 127.0.0.1 -p 5432 -U questionnaire -d questionnaire -W -Fc -f questionnaire.dump
   ```
2. **恢复至全新独立空库**（目标库预先由 `createdb` 创建且不跑应用迁移）：
   ```powershell
   createdb -h 127.0.0.1 -p 5432 -U postgres -W --owner=questionnaire --encoding=UTF8 --template=template0 --locale=C questionnaire_restore
   pg_restore -h 127.0.0.1 -p 5432 -U questionnaire -d questionnaire_restore -W --exit-on-error --no-owner --no-privileges questionnaire.dump
   ```
3. **核对**：比对源库与恢复库的 `users`、`surveys`、`versions`、`responses`、`audits` 行数及分版本统计结果。备份文件包含密码哈希、Session 摘要与完整答卷，须按敏感数据严格限制访问权限。
