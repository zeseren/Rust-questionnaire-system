# 后端开发指南

这是一份「读代码 + 跑通流程」的向导，写给第一次接触本项目的开发者。它不要求你先读完 Rust 教材，也不重复安装步骤：请先按 [README 的前三节](../README.md) 准备好数据库、`.env` 并启动服务，再回到这里。

后端运行在一个三成员 Cargo workspace 里，分工是固定的：

| 目录 | crate 名 | 职责 |
| --- | --- | --- |
| `backend/` | `questionnaire-api` | HTTP JSON API、鉴权、事务与业务用例 |
| `logic-core/` | `questionnaire-logic` | 纯领域核心：题面与答案规则、表达式、流程引擎、文本 DSL；不依赖 HTTP/数据库，可编译到 `wasm32-unknown-unknown` |
| `logic-wasm/` | `logic-wasm` | wasm 边界，把领域核心交给前端复用同一套实现 |

`backend/src/lib.rs` 用 `pub use questionnaire_logic::domain;` 再导出领域核心，因此业务代码里的 `questionnaire_api::domain::Document` 和直接使用 `questionnaire_logic::domain::Document` 是同一个类型，下面所有模块路径也保持这个写法。

全文分六节：第 1 节在 PowerShell 里跑一个最小真实闭环；第 2 节顺着一个请求读源码；第 3、4 节分别是版本并发规则与数据校验口径；第 5 节是路由与错误速查；第 6 节是备份与独立恢复。

## 1. PowerShell 最小真实闭环

这段流程会新建一个普通发布者、创建一份含六种题型的问卷、匿名提交一份答卷，再用发布者身份读取统计和 CSV。它操作的是真实数据库，不是测试替身；请使用本机演示数据，不要填真实敏感信息。

### 1.1 注册、登录并保留 Cookie

以下片段请按顺序在**同一个新的 PowerShell 窗口**执行，服务在另一个窗口运行。默认地址是 `http://localhost:3000`；用户名自动生成以免重复，密码在凭据窗口里输入（8—128 个字符），不会写进脚本或命令历史。

```powershell
$Base = 'http://localhost:3000'
$Origin = 'http://localhost:3000' # 前端开发时改为 http://localhost:5173，须与后端 PUBLIC_ORIGIN 一致
$WriteOptions = @{
    Headers = @{ Origin = $Origin; 'X-CSRF-Protection' = '1' }
    ContentType = 'application/json; charset=utf-8'
    ErrorAction = 'Stop'
}
$Username = 'learner_' + [guid]::NewGuid().ToString('N').Substring(0, 12)
$Credential = Get-Credential -UserName $Username -Message '创建演示发布者：请输入自己的密码（8—128 个字符）'
$CredentialBody = @{
    username = $Credential.UserName
    password = $Credential.GetNetworkCredential().Password
} | ConvertTo-Json -Compress
$CredentialBytes = [System.Text.Encoding]::UTF8.GetBytes($CredentialBody)
try {
    $Registered = Invoke-RestMethod -Method Post -Uri "$Base/api/auth/register" @WriteOptions -Body $CredentialBytes
    $Publisher = Invoke-RestMethod -Method Post -Uri "$Base/api/auth/login" @WriteOptions -Body $CredentialBytes -SessionVariable PublisherSession
} finally {
    Remove-Variable Credential, CredentialBody, CredentialBytes
}
Invoke-RestMethod -Uri "$Base/api/auth/me" -WebSession $PublisherSession
```

要点：

- 注册只建账号，不会自动登录。
- 登录响应的 `Set-Cookie` 被 `-SessionVariable PublisherSession` 保存下来，后续管理请求都要带 `-WebSession $PublisherSession`。
- **每个写请求**（不只是已登录请求）都必须携带 `Origin` 和 `X-CSRF-Protection: 1`。`Origin` 必须与 `.env` 里的 `PUBLIC_ORIGIN` 完全一致，不能带尾随 `/`。浏览器会自动设置 Origin，命令行需要自己传。
- `Get-Credential` 的密码在发送瞬间仍是进程内明文，用完请关闭终端；不要把密码或 Cookie 复制到别处。

### 1.2 构造包含六种题型的 Document 并创建问卷

`Document` 的字段是固定的：`title`、`description`、`purpose`、`thank_you`、`questions`。即使某些文字暂时为空也不能省略字段（未知字段会被拒绝）。题型配置与 `id`/`title`/`required` 在同一层，没有 `kind` 包裹层。题目和选项用稳定的 UUID 标识，不能用标题或数组下标代替。

```powershell
$QSingle = [guid]::NewGuid().ToString()
$QMultiple = [guid]::NewGuid().ToString()
$QText = [guid]::NewGuid().ToString()
$QTextarea = [guid]::NewGuid().ToString()
$QNumber = [guid]::NewGuid().ToString()
$QRating = [guid]::NewGuid().ToString()
$SingleYes = [guid]::NewGuid().ToString()
$SingleNo = [guid]::NewGuid().ToString()
$MultiCode = [guid]::NewGuid().ToString()
$MultiDocs = [guid]::NewGuid().ToString()

$Document = @{
    title = 'Rust 后端学习体验'
    description = '只用于本机学习，不收集姓名、联系方式等信息。'
    purpose = '了解示例使用体验，帮助本人调整学习顺序。'
    thank_you = '感谢参与这次本机演示。'
    questions = @(
        @{
            id = $QSingle; title = '是否愿意继续学习？'; required = $true
            type = 'single_choice'
            options = @(
                @{ id = $SingleYes; label = '愿意' }
                @{ id = $SingleNo; label = '暂时不' }
            )
        }
        @{
            id = $QMultiple; title = '想继续了解哪些内容？'; required = $true
            type = 'multiple_choice'; min_choices = 1; max_choices = 2
            options = @(
                @{ id = $MultiCode; label = '业务代码' }
                @{ id = $MultiDocs; label = '使用文档' }
            )
        }
        @{
            id = $QText; title = '一句话感受'; required = $false
            type = 'text'; max_length = 100
        }
        @{
            id = $QTextarea; title = '补充建议'; required = $false
            type = 'textarea'; max_length = 500
        }
        @{
            id = $QNumber; title = '本次学习分钟数'; required = $true
            type = 'number'; integer = $true; min = 0; max = 1440
        }
        @{
            id = $QRating; title = '易懂程度'; required = $true
            type = 'rating'; min = 0; max = 5
            min_label = '还不理解'; max_label = '非常清楚'
        }
    )
}
$CreateBody = @{ document = $Document } | ConvertTo-Json -Depth 20 -Compress
$Survey = Invoke-RestMethod -Method Post -Uri "$Base/api/surveys" @WriteOptions -WebSession $PublisherSession -Body ([System.Text.Encoding]::UTF8.GetBytes($CreateBody))
$SurveyId = $Survey.id
$Survey | Select-Object id, status, revision
```

两个容易踩的细节：`ConvertTo-Json -Depth 20` 必不可少，默认深度装不下题目和选项的嵌套；显式转 UTF-8 字节是为了避免 Windows PowerShell 用不合适的默认编码发送中文。

创建成功返回 `201`，`revision` 从 `1` 开始，`status` 是 `unpublished`。

### 1.3 设置填写模式并发布

```powershell
$CollectionBody = @{ mode = 'anonymous'; deadline = $null } | ConvertTo-Json -Compress
$Survey = Invoke-RestMethod -Method Patch -Uri "$Base/api/surveys/$SurveyId/collection" @WriteOptions -WebSession $PublisherSession -Body ([System.Text.Encoding]::UTF8.GetBytes($CollectionBody))

$PublishBody = @{ revision = $Survey.revision } | ConvertTo-Json -Compress
$Published = Invoke-RestMethod -Method Post -Uri "$Base/api/surveys/$SurveyId/publish" @WriteOptions -WebSession $PublisherSession -Body ([System.Text.Encoding]::UTF8.GetBytes($PublishBody))
$Published.survey | Select-Object id, status, current_version_id, share_url

# 不携带发布者 WebSession：以匿名填写者的视角读取当前公开题面
$Public = Invoke-RestMethod -Uri "$Base/api/public/surveys/$SurveyId"
$Public.document.purpose
$Public.privacy
```

- 第一次发布前必须先设置 `anonymous` 或 `login_once`；**首次发布后模式冻结**，改模式会返回 `409 mode_frozen`。
- 发布请求只传 `revision`：发布的是服务器上已保存的那一版草稿，不需要再传一份 Document。首次发布返回 `201`，问卷状态变为 `collecting`。
- `share_url` 是同源公开 API 路径 `/api/public/surveys/{id}`；Vue 前端把它对应到网页地址 `/s/:id`，并在填写前展示收集目的与 `privacy` 提示，详见[前端指南](frontend-guide.md)。

### 1.4 提交一份答卷并重试同一次提交

一次逻辑提交只生成一次随机 UUIDv4 作为 `idempotency_key`。它是秘密，不要打印、放进 URL 或分享给别人；它也不是问卷/版本 ID。

```powershell
$SubmissionSecret = [guid]::NewGuid().ToString()
$Submission = @{
    version_id = $Public.version_id
    idempotency_key = $SubmissionSecret
    answers = @(
        @{ question_id = $QSingle; value = $SingleYes }
        @{ question_id = $QMultiple; value = @($MultiCode, $MultiDocs) }
        @{ question_id = $QText; value = '流程清楚' }
        @{ question_id = $QTextarea; value = "先发送请求，再阅读代码。`n这是一条演示建议。" }
        @{ question_id = $QNumber; value = 30 }
        @{ question_id = $QRating; value = 4 }
    )
}
$SubmissionBody = $Submission | ConvertTo-Json -Depth 20 -Compress
$SubmissionBytes = [System.Text.Encoding]::UTF8.GetBytes($SubmissionBody)
$Receipt = Invoke-RestMethod -Method Post -Uri "$Base/api/public/surveys/$SurveyId/responses" @WriteOptions -Body $SubmissionBytes
$Receipt

# 使用原秘密、原版本、原答案重试，不重新生成 UUID
$RetryReceipt = Invoke-RestMethod -Method Post -Uri "$Base/api/public/surveys/$SurveyId/responses" @WriteOptions -Body $SubmissionBytes
$Receipt.receipt_id -eq $RetryReceipt.receipt_id
```

首次成功的状态码是 `201`，合法重试是 `200`，两者都返回 `{ status, receipt_id }`；最后的比较应为 `True`，同一次提交不会多出第二份答卷。匿名提交故意**不带** `-WebSession`：匿名模式不会解析 Cookie，即使带上登录 Cookie 也不会关联账号。

如果请求结果未知（网络中断、超时），不要在改答案或换秘密后重发——中断可能发生在服务端已经提交事务之后。保留原请求重试即可。确认不再需要重试后再清理变量：

```powershell
Remove-Variable SubmissionSecret, Submission, SubmissionBody, SubmissionBytes
```

### 1.5 读取统计、答卷详情与导出 CSV

```powershell
$VersionId = $Public.version_id
$Statistics = Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/statistics?version_id=$VersionId" -WebSession $PublisherSession
$Statistics | Select-Object version_id, version_number, total_count, timezone
$Statistics.questions | Select-Object title, type, answered_count

$ResponseDetail = Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/responses/$($Receipt.receipt_id)" -WebSession $PublisherSession
$ResponseDetail | Select-Object receipt_id, version_id, submitted_at
$ResponseDetail.question_states

$DataDirectory = Join-Path $env:LOCALAPPDATA 'questionnaire-data'
New-Item -ItemType Directory -Path $DataDirectory -Force | Out-Null
$CsvPath = Join-Path $DataDirectory "survey-$SurveyId.csv"
Invoke-WebRequest -UseBasicParsing -Uri "$Base/api/surveys/$SurveyId/export.csv?version_id=$VersionId" -WebSession $PublisherSession -OutFile $CsvPath
Import-Csv -LiteralPath $CsvPath -Encoding UTF8 | Select-Object receipt_id, version_number
```

始终显式传 `version_id`：省略时接口默认取当前发布版本，发布过新版本后再查会看到不同的数据。在这份刚创建、只有一份提交的问卷上，`total_count` 应为 `1`，CSV 应有一条数据行。CSV 落在本地应用数据目录，不进源码仓库，仍属于敏感数据。

## 2. 沿着一个请求读源码

推荐按「先读保存草稿，再读提交答卷」的顺序：前者展示如何避免覆盖别人的修改，后者展示并发下如何只接受一份。

### 2.1 三个 crate 如何协作

`backend` 只持有 HTTP 与业务代码；所有 DSL 语义住在 `logic-core/src/`。因此领域规则可以同时被 HTTP 后端和前端 wasm 使用，不会出现两份实现。

```text
logic-core/  domain.rs  identity.rs  logic.rs  logic_document.rs  logic_text.rs + logic_text.pest  flow.rs
                   ▲                                    ▲
                   │ pub use questionnaire_logic::domain │ wasm-bindgen JSON 信封
              backend/ (questionnaire-api)          logic-wasm/ (logic-wasm)
```

### 2.2 一次「保存草稿」的调用链

```text
main.rs: main
  └─ lib.rs: run → 读取配置、建立连接池、执行迁移、组装 router、监听 HTTP
       └─ security.rs: guard → 同源写校验、全局限流、no-store / nosniff
            └─ surveys.rs: save_draft（HTTP handler）
                 ├─ http.rs: ApiPath / ApiJson 在边界解析输入
                 ├─ auth.rs: AuthUser 提取有效账号
                 └─ surveys.rs: save_survey_draft（业务函数）
                      ├─ domain（logic-core/src/domain.rs）: validate_draft
                      ├─ repository.rs: lock_owned_survey / expire_survey
                      └─ entities.rs: survey::ActiveModel → PostgreSQL 事务
```

每个业务模块都把 HTTP handler 和它调用的业务函数放在同一个文件里，只有共用的归属过滤与行锁集中在 `repository.rs`。handler 负责提取参数、返回 DTO；业务函数负责校验、加锁和提交事务。

### 2.3 文件导读表

| 文件 | 先看这些函数/类型 | 说明 |
| --- | --- | --- |
| [`main.rs`](../backend/src/main.rs)、[`lib.rs`](../backend/src/lib.rs) | `main`、`run`、`router`、`AppState` | Tokio 入口；配置、迁移、连接池、路由合并与 OpenAPI 组装；`serve` / `migrate` / `create-admin` 子命令 |
| [`http.rs`](../backend/src/http.rs)、[`error.rs`](../backend/src/error.rs) | `ApiJson`、`ApiPath`、`ApiQuery`、`Pagination`、`AppError` | 输入只在边界解析一次；错误统一成 HTTP JSON |
| [`surveys.rs`](../backend/src/surveys.rs) | `save_draft` → `save_survey_draft`；`publish` → `publish_survey` | handler 不做数据库拼装；业务函数检查归属、修订号、状态后提交事务 |
| [`submissions.rs`](../backend/src/submissions.rs) | `public_survey` → `read_public_survey`；`submit` → `submit_response` → `lock_public_survey` | 只读快照读题面；写路径加锁、重验 Session、幂等、版本匹配、原子写入 |
| [`domain`（`logic-core/src/domain.rs`）](../logic-core/src/domain.rs) | `Document`、`QuestionKind`、`validate_draft`、`validate_publish`、`validate_answers` | 不依赖网络和数据库的题面/答案规则 |
| [`identity.rs`](../logic-core/src/identity.rs) | `parse`、`deserialize`、`deserialize_vec` | 非零、小写连字符配置 UUID 的统一解析边界 |
| [`logic.rs`](../logic-core/src/logic.rs)、[`logic_document.rs`](../logic-core/src/logic_document.rs) | `compile_expression`、`DocumentLogic::compile` | 表达式类型、变量依赖、题目规则、容量与引用重写 |
| [`flow.rs`](../logic-core/src/flow.rs) | `FlowSession`、`FlowEvaluator` | 前向流程、改答清理、提交与报表的路径重建 |
| [`logic_text.rs`](../logic-core/src/logic_text.rs)、[`logic_text.pest`](../logic-core/src/logic_text.pest) | `parse_logic_text`、`format_logic_text`、`Grammar`、`PRECEDENCE` | pest 文法与 Pratt 优先级；资源预检后编译到统一 AST；文本格式化唯一实现 |
| [`logic-wasm/src/lib.rs`](../logic-wasm/src/lib.rs) | `parse_logic_text`、`compile_document_logic`、`FlowSession` | wasm 边界：入参出参都是 JSON 字符串，信封为 `{"ok":…}` / `{"error":…}` |
| [`repository.rs`](../backend/src/repository.rs)、[`entities.rs`](../backend/src/entities.rs) | `owned_survey`、`lock_user`、`lock_survey`、`lock_session_user`、各 `Model` | 共用的归属过滤与行锁顺序；数据库字段映射 |
| [`reports.rs`](../backend/src/reports.rs) | `statistics`、`response_detail`、`response_snapshot`、`summarize_question`、`encode_csv` | 同一版本与时间过滤；一次结果快照同时生成统计与导出 |
| [`auth.rs`](../backend/src/auth.rs)、[`admin.rs`](../backend/src/admin.rs) | `authenticate`、`change_password`、`change_user`、`change_survey` | Session 生命周期与账号资格；治理操作与审计同事务落库 |
| [`security.rs`](../backend/src/security.rs) | `guard`、`check` | 同源写校验与单实例固定窗口限流 |
| [`migration.rs`](../backend/src/migration.rs)、[`schema.sql`](../backend/src/schema.sql) | `InitialSchema::up` | 建表 SQL；主键、外键、唯一索引作为数据库层约束 |

### 2.4 读懂本项目需要的语言与框架常识

- **`Result` 与 `?`**：`save_survey_draft` 返回 `AppResult<SurveyDetail>`——成功拿到详情，失败得到 `AppError`。`input.document.validate_draft()?` 的意思是「校验失败就提前返回」，错误会经 `From` 转换统一成 HTTP 响应；它不会吞掉错误，也不会自动重试。`ValidationError.question_id: Option<Uuid>` 让错误能定位到具体题目，问卷级错误的 `question_id` 为 `null`。
- **`async` / `.await`**：`state.db.begin().await?` 在等待数据库时把线程让给 Tokio 处理其他请求。`.await` 本身不保证原子性，约束并发的是事务和行锁。较重的 Argon2 密码运算放在 `spawn_blocking` 阻塞线程池里执行，不占用异步工作线程。
- **Axum 提取器与解构**：`State(state): State<AppState>` 是从 router 提供的共享状态里取出 `state`；`ApiJson(input): ApiJson<SaveDraft>` 在进入业务前把请求体解析成结构化类型，业务代码于是只处理类型，不反复解析 JSON。
- **`Arc` 与连接池**：`AppState.config: Arc<Config>`、`limits: Arc<Limits>` 让所有请求共享同一份配置和限流器；克隆 `Arc` 只增加引用计数，不复制数据。数据库连接来自共享连接池（`max_connections = 10`），不是每个请求新建一条。
- **`enum` 表达题型**：`QuestionKind` 把六种题型和各自的专属字段放进六个分支——评分题有 `min_label`，文本题有 `max_length`，不需要给所有题型塞无用字段。`match` 会强制你处理每个分支。
- **Serde 的两种工作方向**：`Deserialize` 把 JSON 解析成 Rust 值，`Serialize` 把 Rust 值编码出去。`#[serde(tag = "type", rename_all = "snake_case")]` 让 `SingleChoice` 在 JSON 里变成 `"type": "single_choice"`。输入 DTO 上的 `deny_unknown_fields` 会拒绝不认识的字段，所以多传 `role` 之类的字段不会悄悄生效。因为 Serde 的 `flatten` 会让 `deny_unknown_fields` 失效，`Question` 手写了反序列化：先取出公共字段（`id`/`title`/`required`），其余全部交给严格的 `QuestionKind`，于是 `min` 拼错成 `minimum` 会返回 `400 invalid_json`，而不是被静默丢弃。
- **`Option` 表达真实语义**：`current_version_id: None` 表示从未发布；回收设置里的 `deadline: Option<Option<DateTime<Utc>>>` 专门用来区分 PATCH 的三种情况——字段省略保持原值、传 `null` 清空、传时间字符串替换。
- **SeaORM 的 `Model` / `ActiveModel` / `Set`**：`survey::Model` 是查到的一行；`record.into_active_model()` 变成可更新对象；`active.draft = Set(...)` 明确标记哪些字段要写；`active.update(&tx).await?` 才真正执行 SQL——给内存对象赋值不会自动保存。部分更新里的 `..Default::default()` 表示「这些列不动」，不会把其他列重置。
- **`&tx` 与 `&state.db`**：前者是当前事务，后者是连接池。已经进入业务事务后，相关读写必须继续用 `&tx`，否则可能换到另一条连接，前面的锁和原子性就失效了。

绝大多数查询与写入走 SeaORM；只有 `reports::response_snapshot` 用参数绑定的 SQLx 查询，并复用同一个连接池——它不是第二套数据库，SQL 里仍然带 `owner_id` 过滤。所有对外响应都是专门的 DTO，不会直接序列化数据库实体，所以密码哈希、内部填写者关联和提交秘密不会因为「返回整行」而泄漏。

## 3. 表结构、版本隔离与并发控制

### 3.1 六张业务表

| 表 | 关键字段与作用 |
| --- | --- |
| `users` | `id`、唯一的小写 `username`、Argon2id `password_hash`、`role`（`user`/`admin`）、`disabled` |
| `sessions` | `id_hash`（浏览器 Session 秘密的 SHA-256）、`user_id`、`expires_at` |
| `surveys` | `owner_id`、`status`、`mode`、`deadline`、独立的治理标志 `taken_down`；可变的 `draft`、`revision`、`current_version_id` |
| `versions` | 每次发布一行：`survey_id`、递增 `number`、不可变的 `document`、`published_at` |
| `responses` | 规范化后的整份 `answers`、`version_id`、`submitted_at`；`id` 就是回执；`respondent_id`/`scope`/`key_hash`/`payload_hash` 支撑限次与幂等 |
| `audits` | 治理人 `actor_id`、`action`、目标 `target_id`、必填 `reason`（1—500 字符）、时间；与治理改动写在同一事务 |

题面和整份答案都以 JSONB 存储，不是「一道题一张表」。题目 ID 决定答案归属，数组顺序决定展示顺序，所以重命名或排序不应更换 ID。

数据库层面还有几组关键约束：

- 复合外键 `surveys(id, current_version_id) → versions(survey_id, id)` 与 `responses(survey_id, version_id) → versions(survey_id, id)` 保证引用到的版本属于同一份问卷。
- `versions` 的 `(survey_id, number)` 唯一，版本号在同一问卷内不重复。
- `responses` 的 `(survey_id, scope, key_hash)` 唯一，同一作用域下同一个提交秘密只落一行。
- 部分唯一索引 `responses_once_per_account` 约束非空 `respondent_id` 上的 `(survey_id, respondent_id)`，**不含版本号**——发布新版本不能给同一个账号再争取一次填写资格。

复制问卷会调用 `Document::renew_ids()` 重写全部题目与选项 ID，生成一份独立草稿：不继承答卷、回收设置或跨版本限次资格，并按当前草稿上限重新校验内容。

### 3.2 草稿 revision 与发布 version 是两回事

- `revision` 从 `1` 开始，每成功保存一次整份草稿加一。保存和发布都要传最后读到的修订号，`ensure_revision` 在事务锁内比较：值不匹配返回 `409 revision_conflict`，不会静默覆盖另一个窗口的内容。这就是这里的乐观锁。
- 发布只做一件事：往 `versions` 插入新行，再把 `surveys.current_version_id` 指向它。历史版本不被草稿编辑影响，每份答卷绑定它提交时的版本。
- 发布**先**执行 `Document::deserialize`、`validate_publish()` 和 `mode` 非空检查，**再**判断这版草稿是否与当前版本内容完全相同。相同时按幂等重放处理：不新增版本，直接返回 `200` 和当前版本；确实新建版本才返回 `201`。这样响应丢失后的重试不会凭空多出一个内容相同的版本号，同时校验结果也不会因为「看起来像重试」而被跳过。
- 第一次发布前必须设置填写模式，之后模式冻结；首次发布把 `unpublished` 变成 `collecting`。截止时间到期后问卷按 `closed` 处理，即使之后延长或清空 `deadline`，也要显式调用 `resume` 才恢复回收（`expire_survey` 会先把「已到期」这个事实落库，再应用新设置，避免延期顺手抹掉关闭状态）。
- `archive` 不可撤销：归档后不能编辑、发布或恢复，但历史版本、答卷、报表和复制能力保留。彻底删除只允许从未发布过的问卷。
- `taken_down` 是独立的治理标志。发布者恢复回收不会撤销管理员的下架；管理员恢复问卷也不会改动关闭状态或截止时间。

### 3.3 锁顺序与只读快照

所有回收相关的写入都使用同一个顺序：**发布者用户行 `FOR SHARE` → 问卷行 `FOR UPDATE`**。`repository::lock_owned_survey` 把这个顺序集中在一处，锁后立刻校验归属。公开读题面走的是另一条路：`GET /api/public/surveys/{id}` 用 `RepeatableRead + ReadOnly` 只读事务快照检查回收状态、发布者状态和登录限次资格（匿名模式完全不解析 Cookie），**不取行写锁**，因此读题面不会和答卷提交互相阻塞。

`AuthUser` 提取器在读取请求体之前就完成了认证，而请求体可能迟到。所以所有「所有者写入」（新建、保存、复制、删除、发布、回收设置、关闭/恢复/归档）都要经过 `repository::lock_session_user`：锁住账号行后，在同一个事务里用原 Cookie 重新 `authenticate` 并核对身份。退出登录、修改密码、被禁用都要取得同一账号的 `NO KEY UPDATE` 锁，因此持锁后的重验一定能看到撤销结果，迟到请求会得到 `401` 而不会写入。

`submit_response` 通过 `lock_public_survey` 取同样的锁：登录限次模式还会经 `lock_session_user` 锁填写者并重验 Session。治理类写入（禁用用户、下架问卷）同样用 `NO KEY UPDATE`——它与提交持有的 `SHARE` 冲突，从而和提交排队；同时不与外键检查用的 `KEY SHARE` 冲突，避免管理员交叉治理时无谓互锁。

服务拿到锁之后才读取提交时间再检查截止，不会把「等待锁之前的旧时间」当成提交时刻。事务只有在全部校验和插入都成功后提交，因此不会留下半份答卷或单独占用限次资格。代价是同一份问卷上的写入会排队——这是业务一致性要求，不是高并发吞吐设计。

只读路径统一使用快照：本人问卷列表、答卷列表、统计与 CSV、管理员治理列表都是 `RepeatableRead + ReadOnly`（统计与 CSV 的 `response_snapshot` 用一条 SELECT 固定结果集），保证一个响应里的总数与分页/统计来自同一时刻，也不与写入争抢行锁。问卷列表和管理员治理列表通过 JSONB 投影只取 `title`，不逐行反序列化整份 `Document`。

### 3.4 幂等提交：成功记录本身就是答卷

服务端只保存 UUIDv4 秘密的 SHA-256（`key_hash`），不保存明文。作用域由问卷和填写模式决定：匿名是 `"anonymous"`，登录限次是账号 UUID。答案先按题目 UUID 排序、多选按选项 UUID 排序，再与 `version_id` 一起算出 `payload_hash`。指纹、答案和回执写在同一条 `responses` 记录里，天然原子。

提交时的判定顺序：

1. `idempotency_key` 必须是安全随机 UUIDv4，否则 `422`。
2. 用 `(survey_id, scope, key_hash)` 查历史成功记录；命中且版本不同，直接 `409 idempotency_conflict`。
3. 没有历史记录时检查回收资格与账号限次；有记录时不再重复检查（于是关闭、下架、发布者禁用或版本切换之后，原请求仍能合法重试，也不占新名额）。
4. 用**原版本的不可变题面**规范化答案——有规则的版本还会从入口重建路径，只有已校验且在有效路径内的答案参与后续条件。
5. 指纹一致返回原回执（`200`）；指纹不同返回 `409`；全新记录插入并返回 `201`。

因此题目排列顺序、多选集合顺序、`1` 与 `1.0` 都不构成不同载荷，而重复题目/选项仍是非法输入，不会被排序掩盖。没有成功记录、但提交到旧版本时返回 `409 version_changed`：客户端应保留输入、重新加载题面核对，不能自动换版本强行提交。

## 4. 数据校验与统计口径

### 4.1 匿名与登录限次的隐私边界

- `anonymous` 不解析 Cookie，`respondent_id` 为空，不使用设备指纹或地理位置判定身份，也不承诺按自然人限次或严格一人一票。
- `login_once` 为了做到「同一账号跨版本一次」，内部必须保留账号关联，所以它不是完全匿名；发布者看到的答卷详情、列表、统计和导出里都不含账号字段。
- 公开链接可以被转发，它不是保密访问控制。应用管理员只有治理权限，不能以管理员身份读取他人答卷；数据库账号和备份运维权限能接触数据，这与应用角色是两件事。
- 密码用 Argon2id（`spawn_blocking` 内调用 Argon2 0.6 的 `hash_password`，盐由库生成，保存完整 PHC 字符串）。Session 秘密和提交秘密用 `getrandom::fill` 生成高熵随机值，数据库只存 SHA-256 摘要——快速摘要适合高熵秘密，不能替代密码哈希。
- 修改密码会在同一事务里删除该账号的全部 Session，当前窗口和其他设备都要重新登录；禁用账号同样撤销其 Session。
- Cookie 解析用 `Cookie::split_parse` 遍历全部 Cookie 头，重复 `session` 直接拒绝，校验 64 位十六进制摘要且不做百分号解码；不使用会覆盖同名项的 `CookieJar`。设置 Cookie 时统一带上 `HttpOnly`、`SameSite=Lax`、`Path=/`、`Max-Age`，`Secure` 由配置决定；退出和改密下发 `Max-Age=0` 的清除 Cookie。

### 4.2 文本、缺失与身份

- 草稿允许不完整，但容量、ID 唯一性和数值有限性始终有效。发布额外要求：标题与收集目的非空白、至少一道题、题目标题和选项文字非空白，最多 100 题、每题最多 50 个选项。
- 文字长度按 Rust `chars()`（Unicode 标量值）计数，不是 UTF-8 字节数，也不等同于用户眼里的字形数。上限：问卷标题 200、题目标题 500、选项文字 500、调查说明与收集目的各 5000、结束语 1000、评分端点说明 100；文本题上限为单行 200、多行 1000，发布者可以调低。
- `null`、空白文本和空多选都算缺失：可选题不存这个键，必填题直接拒绝。**合法的数值 `0` 不是缺失。** 单行文本拒绝换行字符，多行文允许换行；非空白文本保留原文，不静默截断或修剪。
- 空白判定统一使用 Unicode `White_Space`（等价于 Rust `str::trim`）：`U+0085` 算空白，`U+FEFF` 算内容。前端填写、流程进度计数和 `answers.ts` 的校验用同一套规则。
- 配置中的题目、选项、变量、规则引用、流程节点与页面归属，以及答案中的 `question_id` 和**选项值**，统一只接受**非零、小写连字符 UUID**。Rust DTO 在反序列化时即拒绝非规范身份，选项值在规范化与规则求值时执行同一严格规则。
- 题面文字、文本答案和治理原因都拒绝 `U+0000`（NUL），不把无法存入 PostgreSQL 文本/JSONB 的输入拖到数据库层再报内部错误。文字一律按纯文本使用，不是可信 HTML。

### 4.3 数字语义

- 数字题只接受 JSON 数字，拒绝字符串、布尔值和非有限值；范围是闭区间。`integer = true` 时要求数学意义上的整数；评分题的整数值还受 `i32` 范围和题面范围约束。
- 校验与统计统一使用**有限 `f64`**（对应 JS 的 `number`）：`value.as_f64()` 能得到有限值即通过，数字的原始文本不参与校验。`serde_json` 使用默认配置。`1` 与 `1.0` 等价，正零与负零视为同一答案，负零归一为 `0.0`；超出双精度精确整数范围的数值按 `f64` 舍入。

### 4.4 版本、时间与统计口径

答卷列表、统计和 CSV 一次只针对一个版本；省略 `version_id` 表示当前发布版本，不会合并历史版本。答卷详情始终附带它提交时的那版题面。

带规则的版本会用该历史版本的 `FlowEvaluator` 重建适用性：详情里的 `question_states` 区分三种状态——`answered`（已答）、`unanswered`（在有效路径内但未答）、`skipped`（被规则跳过）。统计恒满足 `applicable_count = answered_count + unanswered_count`，且 `applicable_count + skipped_count` 等于本次答卷总数。是否跳过只来自路径重建，不靠「答案缺失」猜测。

时间过滤使用 RFC3339，必须带 `Z` 或时区偏移，区间是 **`[from, to)`**：包含 `from`、不包含 `to`，两个都给时必须 `from < to`。PostgreSQL 只存到微秒，过滤边界同样不允许超过微秒的非零精度，不会静默截断；客户端发最多六位小数即可，多余的零尾位不改变时刻。含 `+08:00` 的时间在 URL 里要编码 `+`：

```powershell
$From = [uri]::EscapeDataString('2026-01-01T00:00:00+08:00')
$To = [uri]::EscapeDataString('2027-01-01T00:00:00+08:00')
Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/statistics?version_id=$VersionId&from=$From&to=$To" -WebSession $PublisherSession
```

内部时间一律按 UTC 比较；返回给用户的时间、CSV 提交时间和自然日趋势固定用 **Asia/Shanghai（`+08:00`）**，不依赖服务器操作系统的时区设置。

统计口径要点：

- `total_count` 是选定版本与时间范围内的答卷数。
- 选择题的 `proportion = 选项频次 / 本题 answered_count`，返回 `0..1` 的比例而不是百分数；多选合计可以超过 1。**分母为 0 时 `proportion` 为 `null`**，不是 `0` 或 `NaN`。
- 数字的均值、最小值、最大值排除缺失但包含合法的 `0`；没有有效回答时返回 `null`。评分均值同理。
- 评分的 `distribution` 是按分值升序的**稀疏分布**，只列出实际出现过的分值；没出现的分值等价于频次 0，不为一个很大的评分范围分配每个整数桶。趋势同样省略没有提交的自然日。
- 文本统计返回原始文本、回执与时间，不做情感分析或主观质量判断。

### 4.5 CSV 导出与公式注入防护

`encode_csv` 生成 UTF-8 CSV，使用 CRLF 行结束和标准 CSV 转义。表头依次是回执、版本 ID、版本号、提交时间（Asia/Shanghai +08:00），随后每题一列「`题目标题 [题目 UUID]`」；带规则的版本会在每题后再加一列状态（已答 / 适用未答 / 规则跳过），无规则的历史版本保持原有列结构。选择题保存选项 UUID，**多选在一个字段里保留 JSON 数组**（不是逗号拼接），未答的可选题留空，字段内的逗号、双引号和换行交给 CSV 库处理。

`formula_safe` 会逐格检查（包括表头）：以 `=`、`+`、`-`、`@` 开头，或在前导空白、控制字符、常见不可见字符之后出现这些前缀时，在**原字段最前面加一个单引号 `'`**，避免电子表格把调查内容当公式执行。数据库里的原文不会被改动；负数也可能带这个保护前缀，这是导出安全语义，不是算错数——需要按原始类型分析时请用受权限保护的答卷 JSON。

## 5. 路由速查与错误契约

精确的字段、请求体、响应体和错误码，以启动服务后的 [OpenAPI JSON](http://localhost:3000/api/openapi.json) 为准（端口或 origin 改变时相应调整链接）。业务模块使用 `OpenApiRouter<AppState>` 和 `routes!`，从 handler 上同一份 `#[utoipa::path]` 登记路由与文档，`lib.rs::router()` 合并后用 `split_for_parts()` 生成文档并加入 session Cookie 安全方案。仓库里没有 Swagger 网页，文档只在组装时生成。

### 基础与认证

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/health` | 数据库就绪探测 |
| GET | `/api/openapi.json` | OpenAPI 定义 |
| POST | `/api/auth/register` | 注册普通账号，不自动登录 |
| POST | `/api/auth/login` | 登录并下发 HttpOnly / SameSite=Lax 的 `session` Cookie |
| POST | `/api/auth/logout` | 撤销当前 Session 并清 Cookie，可重复调用 |
| GET | `/api/auth/me` | 当前有效账号 |
| PUT | `/api/auth/password` | 校验旧密码、更新密码、撤销全部 Session |

### 本人的问卷管理

除创建与列表外，`{id}` 都是本人的问卷 UUID；管理员走这些接口也不能越过归属检查。

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET / POST | `/api/surveys` | 筛选分页列表（按标题子串与状态）/ 创建草稿 |
| GET / DELETE | `/api/surveys/{id}` | 管理详情 / 删除从未发布的问卷 |
| PUT | `/api/surveys/{id}/draft` | 携带 `revision` 保存整份草稿 |
| GET | `/api/surveys/{id}/preview` | 已保存草稿与发布校验结果，不产生答卷 |
| POST | `/api/surveys/{id}/copy` | 复制为独立新草稿 |
| POST | `/api/surveys/{id}/publish` | 发布指定 revision（内容未变时幂等返回当前版本） |
| PATCH | `/api/surveys/{id}/collection` | 修改填写模式与截止时间（三态语义见 2.4） |
| POST | `/api/surveys/{id}/close` | 关闭回收 |
| POST | `/api/surveys/{id}/resume` | 显式恢复回收 |
| POST | `/api/surveys/{id}/archive` | 不可撤销归档 |
| GET | `/api/surveys/{id}/versions` | 分页列出发布版本 |

### 公开填写与发布者报表

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/public/surveys/{id}` | 当前可填写题面与隐私提示；登录限次模式需要认证 |
| POST | `/api/public/surveys/{id}/responses` | 整份提交或合法幂等重试 |
| GET | `/api/surveys/{id}/responses` | 本人问卷的答卷分页列表 |
| GET | `/api/surveys/{id}/responses/{receipt_id}` | 一份答卷及其原版本题面 |
| GET | `/api/surveys/{id}/statistics` | 本人问卷选定版本的统计 |
| GET | `/api/surveys/{id}/export.csv` | 本人问卷选定版本的 CSV |

### 管理员治理

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/admin/users` | 账号分页列表 |
| PATCH | `/api/admin/users/{id}` | 设置 `disabled`，必须提供 `reason` |
| GET | `/api/admin/surveys` | 治理所需的问卷元数据 |
| GET | `/api/admin/surveys/{id}` | 元数据与当前已发布题面，不返回他人草稿或答卷 |
| PATCH | `/api/admin/surveys/{id}` | 设置 `taken_down`，必须提供 `reason` |
| GET | `/api/admin/audits` | 治理审计分页列表 |

### 分页、错误与通用约定

- 分页从 `page=1` 开始，`page_size` 默认 `20`、范围 `1..=100`；参数非法返回 `422`。
- 请求体上限 1 MiB（domain::MAX_SUBMISSION_BYTES），超限返回 413。
- 错误响应统一为 `{"error":{"code":"…","message":"…","question_id":null}}`；题目级错误会给出 UUID。
- 常见状态码：JSON/路径格式错误 `400`、无有效身份 `401`、来源或权限被拒 `403`、资源不可访问 `404`、业务冲突 `409`（含 `revision_conflict`、`idempotency_conflict`、`version_changed`、`mode_frozen` 等稳定 `code`）、领域校验失败 `422`、限流 `429`、内部错误 `500`、数据库未就绪 `503`。
- 内部错误会在日志里带 `error_id`、错误类别与发生位置，同时通过响应头 `X-Error-Id` 返回同一标识供检索；日志与响应都不含原始 SQL 或用户数据。
- 所有响应都带 `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff`；写请求必须来自配置的同源站点并携带 `X-CSRF-Protection: 1`，否则 `403 csrf_rejected`。
- 限流是单实例的固定窗口：认证类路径和公开提交各有独立配额（`AUTH_REQUESTS_PER_MINUTE`、`SUBMISSIONS_PER_MINUTE`），不按 IP 或身份区分，也不在多实例间共享。

## 6. 备份与独立恢复

演练请先用非敏感演示数据，保存命令输出和核对结果，不要拿唯一的生产库直接试恢复命令。历史演练记录见[需求分析第 9 章](requirements-analysis.md#9-架构演进历次关键决策与验证总结)。

### 6.1 备份到受保护的文件

用 PostgreSQL 18 客户端连接本项目的 PostgreSQL 18 服务。`pg_dump` 只备份一个数据库，**不会备份集群角色和部署密钥**，这些请另外管理；`.env` 与恢复所需凭据同样要单独保管。备份文件包含密码哈希、Session 哈希、内部填写者关联和全部答案，请放在只有本人或授权运维可读的目录，加密保管并设置保留期限。

```powershell
$BackupDirectory = Join-Path $env:LOCALAPPDATA 'questionnaire-backups'
New-Item -ItemType Directory -Path $BackupDirectory -Force | Out-Null
$DumpPath = Join-Path $BackupDirectory ('questionnaire-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.dump')
pg_dump -h 127.0.0.1 -p 5432 -U questionnaire -d questionnaire -W -Fc -f $DumpPath
if ($LASTEXITCODE -ne 0) { throw '备份失败，不要把此文件当作可恢复备份' }
```

必须使用 `pg_dump -Fc -f 文件` 这种写法：不要用 `pg_dump ... > backup.dump`，也不要让二进制归档经过 PowerShell 文本管道，那会破坏文件。`pg_dump` 自身取得一致快照，但如果备份期间仍有写入，实时库的总数当然可能比快照多——核对时请暂停写入，或记录备份对应的版本与时间范围。

### 6.2 恢复到新的独立空库

恢复目标用一个全新的独立库名，避免误用原库；目标库不能先执行应用迁移，也不能含旧表。下面由数据库管理员建库、应用账号执行恢复，每步交互输入密码：

```powershell
$RestoreDb = 'questionnaire_restore_' + (Get-Date -Format 'yyyyMMdd_HHmmss')
createdb -h 127.0.0.1 -p 5432 -U postgres -W --owner=questionnaire --encoding=UTF8 --template=template0 --locale=C $RestoreDb
if ($LASTEXITCODE -ne 0) { throw '独立恢复库创建失败' }
pg_restore -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W --exit-on-error --no-owner --no-privileges $DumpPath
if ($LASTEXITCODE -ne 0) { throw '恢复失败，此库不能当作完成恢复的结果' }
```

`--no-owner --no-privileges` 让对象由恢复账号拥有，不照搬原库的 ACL；如果正式恢复还需要额外角色授权，应由授权运维显式重建。这里没有 `--clean`，不会覆盖原库，也不会自动删除失败的目标库——失败后先查清原因，再选一个新的空库重试。

### 6.3 核对行数、版本关系与报表

先直接核对恢复库的业务计数与版本对应关系：

```powershell
$CountSql = "SELECT 'users' AS table_name, count(*) AS row_count FROM users UNION ALL SELECT 'surveys', count(*) FROM surveys UNION ALL SELECT 'versions', count(*) FROM versions UNION ALL SELECT 'responses', count(*) FROM responses UNION ALL SELECT 'audits', count(*) FROM audits;"
psql -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W -c $CountSql
psql -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W -c "SELECT s.id AS survey_id, s.current_version_id, v.number, v.id AS version_id, count(r.id) AS response_count FROM surveys s LEFT JOIN versions v ON v.survey_id = s.id LEFT JOIN responses r ON r.survey_id = s.id AND r.version_id = v.id GROUP BY s.id, s.current_version_id, v.number, v.id ORDER BY s.id, v.number;"
```

把这些结果与暂停写入时的源库记录逐项比较——`pg_restore` 退出成功只说明 SQL 没有报错，不等于业务数据已经恢复完整。接着验证应用确实能解释这些数据：

1. 在**单独终端**从仓库根目录启动第二个 API 实例：通过进程环境变量把 `DATABASE_URL` 指向 `$RestoreDb`（密码仍需 URI 编码），设置 `LISTEN_ADDR=127.0.0.1:3001`、`PUBLIC_ORIGIN=http://localhost:3001`、`COOKIE_SECURE=false`，运行 `cargo run --locked -p questionnaire-api`。不要改原实例的连接；恢复库已含迁移记录，启动会照常检查迁移。
2. 在另一个客户端终端把 `$Base` 改成 `http://localhost:3001`、`$Origin` 同步更新，用**备份里已有的发布者账号**重新登录并建立新的 `WebSession`。不要重新注册替代原账号，也不要复用原实例的会话——Cookie 不以端口隔离，两个本机实例更不该混用同一个会话容器。
3. 依次调用 `/health`、该问卷的 `/versions`、带明确 `version_id` 与相同时间区间的 `/statistics`，核对版本列表、每版本答卷数、`total_count`、关键选项频次或数值统计，并抽查一份答卷详情或 CSV。健康检查通过只证明连接正常，不证明数据完整。
4. 记录本次使用的归档、目标库名、命令结果与核对值，完成后停止恢复实例。恢复库和归档仍含敏感数据，请按保留策略处理。
5. 恢复库里的 `sessions` 也来自备份，仍在有效期内的旧会话可能继续有效。如果真实事故恢复要求所有人重新登录，应由运维明确执行会话撤销并留下记录。

## 7. 交付边界

本文覆盖后端 API 与数据一致性实现；Vue 前端与移动布局见[前端指南](frontend-guide.md)，DSL 与流程的阶段边界见 [DSL 实施计划](survey-dsl-plan.md)。性能压测报告、面向公众长期运营的隐私合规流程、HTTPS/TLS 终止、反向代理与容器镜像部署都不在本项目范围内；限流也只是单实例固定窗口，不按身份判断人次。如果要对外运营，告知同意、合法收集范围、数据保留与删除、运维权限和事件响应都需要另行设计，这不是「开了匿名模式」就能替代的。
