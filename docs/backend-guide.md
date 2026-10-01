# 后端使用与代码阅读指南

本文从“发送一个请求”出发介绍已有实现，不要求你先读完 Rust 教材。先按 [README](../README.md) 准备数据库、`.env` 并启动服务。这里的命令仍从**仓库根目录**运行；包在 `backend` 目录，包名是 `questionnaire-api`。

本文的命令是操作说明，不是运行证明；不据此宣称全部验收通过或达到某个性能指标。已验证范围以 [需求分析中的实施状态](requirements-analysis.md) 为准；该文档第 11.2 节记录过一次已在临时实例上完成的备份恢复演练，但它不代表你的环境已经核对。

## 1. PowerShell 最小真实闭环

这个流程新建一个普通发布者、创建含六种题型的问卷，匿名提交一份答卷，再用发布者身份读取统计和 CSV。它会在数据库中留下真实账号、问卷和答卷，不是测试替身。不要填真实敏感信息。

### 1.1 注册、登录并保留 Cookie

以下片段按顺序在**同一个新的 PowerShell 窗口**执行，服务在另一个窗口运行。默认配置使用 `http://localhost:3000`。用户名自动生成以免重复；在凭据窗口自行输入 8—128 个字符的密码，不把密码写进脚本或命令历史。

```powershell
$Base = 'http://localhost:3000'
$Origin = 'http://localhost:3000' # 使用前端配置时改为 http://localhost:5173，需与 PUBLIC_ORIGIN 一致
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

注册只建账号，不自动登录。登录的 `Set-Cookie` 被 `-SessionVariable PublisherSession` 保存，此后管理请求加 `-WebSession $PublisherSession`；不要从 Cookie 中复制 Session 秘密。`Get-Credential` 的密码在发送时仍会短暂成为进程内明文，所以关闭不再使用的终端，不保存会话对象。

这里每个写请求都有 **`Origin` 和 `X-CSRF-Protection: 1`**，不是只有已登录请求才需要。`Origin` 必须和 `.env` 的 `PUBLIC_ORIGIN` 完全一致；不能尾随 `/`。真实浏览器会设置 Origin，命令行需要自己传入。

### 1.2 构造完整 Document 并创建问卷

`Document` 必须包含 `title`、`description`、`purpose`、`thank_you`、`questions`。即使文字暂时为空，也不是随意省略这些字段。题型配置与 `id/title/required` 同层，**没有 `kind` 包裹层**。题目与选项用稳定 UUID 标识，不能用标题或数组下标代替。

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

`ConvertTo-Json -Depth 20` 很重要：默认深度不足以容纳题目和选项。显式转 UTF-8 字节可以避免 Windows PowerShell 发送中文时使用不合适的默认编码。

### 1.3 选择匿名模式，然后发布已保存修订

```powershell
$CollectionBody = @{ mode = 'anonymous'; deadline = $null } | ConvertTo-Json -Compress
$Survey = Invoke-RestMethod -Method Patch -Uri "$Base/api/surveys/$SurveyId/collection" @WriteOptions -WebSession $PublisherSession -Body ([System.Text.Encoding]::UTF8.GetBytes($CollectionBody))

$PublishBody = @{ revision = $Survey.revision } | ConvertTo-Json -Compress
$Published = Invoke-RestMethod -Method Post -Uri "$Base/api/surveys/$SurveyId/publish" @WriteOptions -WebSession $PublisherSession -Body ([System.Text.Encoding]::UTF8.GetBytes($PublishBody))
$Published.survey | Select-Object id, status, current_version_id, share_url

# 不携带发布者 WebSession：用匿名填写者的视角读取当前公开题面
$Public = Invoke-RestMethod -Uri "$Base/api/public/surveys/$SurveyId"
$Public.document.purpose
$Public.privacy
```

发布请求只传 `revision`，发布的是服务器已经保存的草稿，不是另传一份 Document。`share_url` 是同源公开 API 路径。Vue 前端将其对应到网页 `/s/:id`，并在填写前展示收集目的及 `privacy` 提示，详见 [前端指南](frontend-guide.md)。

### 1.4 提交一份答卷，并重试同一次提交

一个逻辑提交只生成一次随机 UUIDv4 秘密。下例在 Windows 上用 `.NET Guid.NewGuid()` 生成；它是 `idempotency_key`，不是公开的问卷/版本 ID。不要打印、放进 URL、日志或分享给他人。

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

首次成功的 HTTP 状态为 `201`，合法重试为 `200`，都返回 `{ status, receipt_id }`。最后的比较应为 `True`，同一次提交不会新增第二份答卷。示例匿名提交故意**不传** `-WebSession`；即使传了登录 Cookie，匿名模式也不会关联账号。

不要在未知结果时改答案或换秘密重发：网络中断可能发生在服务端已经提交事务之后。应保留原请求用于重试。已完成且不再需要重试时，再移除终端中的敏感变量：

```powershell
Remove-Variable SubmissionSecret, Submission, SubmissionBody, SubmissionBytes
```

### 1.5 读取统计、答卷和 CSV

明确传 `version_id`，避免你以后发布了新版本却误把“当前版本统计”当作旧版本统计：

```powershell
$VersionId = $Public.version_id
$Statistics = Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/statistics?version_id=$VersionId" -WebSession $PublisherSession
$Statistics | Select-Object version_id, version_number, total_count, timezone
$Statistics.questions | Select-Object title, type, answered_count

$ResponseDetail = Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/responses/$($Receipt.receipt_id)" -WebSession $PublisherSession
$ResponseDetail | Select-Object receipt_id, version_id, submitted_at

$DataDirectory = Join-Path $env:LOCALAPPDATA 'questionnaire-data'
New-Item -ItemType Directory -Path $DataDirectory -Force | Out-Null
$CsvPath = Join-Path $DataDirectory "survey-$SurveyId.csv"
Invoke-WebRequest -UseBasicParsing -Uri "$Base/api/surveys/$SurveyId/export.csv?version_id=$VersionId" -WebSession $PublisherSession -OutFile $CsvPath
Import-Csv -LiteralPath $CsvPath -Encoding UTF8 | Select-Object receipt_id, version_number
```

在这个刚创建、只有上述一份提交的问卷上，`total_count` 应为 `1`，CSV 应有一条数据行。这里是操作后的核对目标，不是预先替你执行后的结果。CSV 保存在用户本地应用数据目录，不进入源码仓库；仍需限制文件访问，不上传公共网盘。会话过期或修改密码后，重新登录再下载。

## 2. 沿着一个请求读源码

推荐先读保存草稿，再读提交答卷。它们分别展示“防止覆盖旧内容”和“并发下只接受一份”的核心逻辑。

### 2.1 入口、HTTP 边界和业务函数

实际结构不是一套空的多层框架：每个业务模块把 HTTP handler 与它调用的具体业务函数放在同一文件，只有共用的数据访问规则放进 `repository.rs`。

```text
main.rs: main
  └─ lib.rs: run → 配置、连接池、迁移、router、HTTP 监听
       └─ security.rs: guard → 同源写校验、全局限流
            └─ surveys.rs: save_draft（HTTP handler）
                 ├─ http.rs: ApiPath / ApiJson 解析输入
                 ├─ auth.rs: AuthUser 提取有效账号
                 └─ surveys.rs: save_survey_draft（业务函数）
                      ├─ domain.rs: validate_draft
                      ├─ repository.rs: lock_owned_survey / expire_survey
                      └─ entities.rs: survey::ActiveModel → PostgreSQL 事务
```

阅读顺序及对应职责：

| 文件 | 先找这些函数/类型 | 看什么 |
| --- | --- | --- |
| [`main.rs`](../backend/src/main.rs)、[`lib.rs`](../backend/src/lib.rs) | `main`、`run`、`router`、`AppState` | Tokio 入口；配置、迁移、共享状态；各模块如何合并路由 |
| [`http.rs`](../backend/src/http.rs)、[`error.rs`](../backend/src/error.rs) | `ApiJson`、`ApiPath`、`ApiQuery`、`Pagination`、`AppError` | 字符串/JSON 只在边界解析；错误统一成 HTTP JSON |
| [`surveys.rs`](../backend/src/surveys.rs) | `save_draft` → `save_survey_draft`；`publish` → `publish_survey` | handler 不做数据库拼装；业务函数检查归属、修订、状态并提交事务 |
| [`submissions.rs`](../backend/src/submissions.rs) | `submit` → `submit_response` → `lock_public_survey` | 模式判断、行锁、重验 Session、幂等、版本匹配、原子写入 |
| [`domain.rs`](../backend/src/domain.rs) | `Document`、`QuestionKind`、`validate_answers` | 不依赖网络和数据库的题面/答案规则 |
| [`repository.rs`](../backend/src/repository.rs)、[`entities.rs`](../backend/src/entities.rs) | `owned_survey`、`lock_user`、`lock_survey`、各 `Model` | 共用归属过滤与锁；数据库字段映射 |
| [`reports.rs`](../backend/src/reports.rs) | `statistics`、`response_snapshot`、`summarize_question`、`encode_csv` | 同一版本与时间过滤；一次结果快照生成报表和导出 |
| [`auth.rs`](../backend/src/auth.rs)、[`admin.rs`](../backend/src/admin.rs) | `authenticate`、`change_password`、`change_user`、`change_survey` | Session 失效与账号资格变化；治理操作和审计原子落库 |
| [`migration.rs`](../backend/src/migration.rs)、[`schema.sql`](../backend/src/schema.sql) | `InitialSchema::up` | 迁移执行 SQL；主键、外键、唯一索引作为数据库约束 |

### 2.2 只理解此处用到的 Rust 写法

- **`Result` 与 `?`**：`save_survey_draft` 返回 `AppResult<SurveyDetail>`，即成功得到详情、失败得到 `AppError`。`input.document.validate_draft()?` 表示校验失败就提前返回，经错误转换生成统一响应；不是吞掉错误或自动重试。`ValidationError` 中的 `question_id: Option<Uuid>` 可以定位到具体题目，问卷级错误没有题目 ID。
- **`async` / `.await`**：`state.db.begin().await?` 要等待数据库，但等待期间让 Tokio 运行其他请求。`.await` 不代表操作已经原子化；事务与行锁才约束并发。CPU 较重的 Argon2 密码计算在 `auth.rs` 的阻塞线程池中执行，避免占住异步工作线程。
- **`State(state): State<AppState>`**：这是 Axum 的参数提取与解构，意思是从 router 提供的共享状态取出 `state`；同理 `ApiJson(input): ApiJson<SaveDraft>` 解析出结构化请求，而不是在业务里反复解析 JSON。
- **`Arc`**：`AppState.config: Arc<Config>` 和 `limits: Arc<Limits>` 让多个请求共享同一份配置/限流器。克隆 `Arc` 增加共享引用计数，不复制整份配置；它也不是数据库锁。数据库连接来自共享连接池，不是每个请求临时建立一条新连接。
- **`enum`**：`QuestionKind` 把六种题型及各自字段放在六个分支中。`Rating` 有 `min_label`，`Text` 有 `max_length`，无需给所有题型塞一堆无意义的空字段。`SurveyStatus` 表达问卷状态，`match` 清楚列出状态分支。
- **Serde**：`#[serde(tag = "type", rename_all = "snake_case")]` 使 `SingleChoice` 在 JSON 中成为 `"type":"single_choice"`；题型配置在 JSON 中与 `id`、`title`、`required` 位于同一层。`Document` 等输入 DTO 的 `deny_unknown_fields` 拒绝不认识的字段；不要靠多传 `role` 等字段绕过注册权限。Serde 的 `flatten` 会让 `deny_unknown_fields` 失效，所以 `Question` 手写反序列化：先取出公共字段，其余字段交给带 `deny_unknown_fields` 的 `QuestionKind`，把 `min` 拼成 `minimum` 这类错误会返回 `400 invalid_json`，而不是静默丢弃规则。`Serialize` 是把 Rust 值编码出去，`Deserialize` 是把输入解析成 Rust 值。
- **`Option` 不是通用兜底**：`current_version_id: None` 表示从未发布。回收设置中的 `deadline: Option<Option<DateTime<Utc>>>` 专门区分 PATCH 的三态：省略保持原值、`null` 清空、时间字符串替换。
- **SeaORM 的 `Model`、`ActiveModel`、`Set`**：`survey::Model` 是查到的一行。`record.into_active_model()` 将它变成可更新对象，`active.draft = Set(...)` 明确标记该字段要写入；`active.update(&tx).await?` 执行 SQL，不是给内存对象赋值就自动保存。`insert_draft` 用 `survey::ActiveModel { id: Set(...), revision: Set(1), ... }` 插入新行；部分更新里的 `..Default::default()` 留下未设置字段，并不把其他列重置为默认值。
- **`&tx` 与 `&state.db`**：前者是当前事务，后者是连接池。已进入业务事务后，相关读写必须继续用 `&tx`，否则可能跑到另一条连接，失去前面锁与原子性的保护。

SeaORM 做大部分查询和写入；`reports::response_snapshot` 使用参数绑定的 SQLx 查询，并复用 SeaORM 的 PostgreSQL pool。这里的 SQLx 不是第二套数据库，也没有绕过 `owner_id` 过滤。HTTP 返回专门的 DTO，而不是直接序列化实体，所以密码哈希、内部账号关联和提交秘密不会因“返回整行”泄漏。

## 3. 表、版本与并发规则

### 3.1 六张业务表

| 表 | 关键字段及作用 |
| --- | --- |
| `users` | `id`、唯一的小写 `username`、Argon2id `password_hash`、`role`、`disabled` |
| `sessions` | `id_hash` 是浏览器 Session 秘密的 SHA-256 摘要；`user_id`、`expires_at` 控制有效身份 |
| `surveys` | `owner_id`、`status`、`mode`、`deadline`、独立的 `taken_down`；可变 `draft`、`revision`、`current_version_id` |
| `versions` | 每次发布一行：`survey_id`、递增 `number`、不可变 `document`、`published_at` |
| `responses` | 整份规范化 `answers`、`version_id`、`submitted_at`；`id` 是回执；`respondent_id/scope/key_hash/payload_hash` 用于限次和幂等 |
| `audits` | 治理人 `actor_id`、`action`、目标 `target_id`、必填 `reason`、时间；与治理改动同事务写入 |

题面和整份答案使用 JSONB；不是每道题一张表。题目 ID 决定答案归属，数组顺序决定展示顺序，重命名或排序不应换 ID。复制问卷调用 `renew_ids` 更新所有题目/选项 ID，生成独立草稿，不继承答卷、回收设置或跨版本限次资格。复制按历史文本容量校验（`Document::validate_copy_draft`），因此容量收紧前发布的问卷仍能复制成可编辑草稿，而不是在复制时被当前上限拒绝。

数据库用复合外键保证答案的版本属于同一问卷；`(survey_id, number)` 保证版本号唯一；`(survey_id, scope, key_hash)` 保证同作用域提交秘密唯一。`responses_once_per_account` 对非空 `respondent_id` 约束 `(survey_id, respondent_id)` 唯一，**不含版本号**，因此不能靠发布新版本再次占用同账号的填写资格。

### 3.2 草稿 revision 与发布 version 不同

- `revision` 从 `1` 开始，每次成功保存整份草稿加一。保存和发布都要求传入最后读到的修订号；`ensure_revision` 在事务锁内比较，旧值返回 `409`，不会静默覆盖另一窗口保存的内容。这就是此处的乐观锁。
- 发布插入新的 `versions` 行，然后切换 `surveys.current_version_id`。历史版本不被草稿编辑覆盖，答卷绑定提交时的版本。当前实现的不可变性由业务写路径维护；数据库运维人员仍可直接改数据，因此不要绕过 API 手改题面和答案。
- 草稿 revision 未变、且内容与当前版本完全一致时，发布按幂等重放处理：不新增版本，返回 `200` 与当前版本；只有确实创建新版本才返回 `201`。重放判定先于当前内容上限与提交预算校验，所以容量收紧前的历史版本在内容未改动时仍能重复发布；内容有变化的新发布继续按当前上限返回 `422`。这样响应丢失后的重试不会凭空产生第二个内容相同的版本号。
- 草稿与新发布版本的单行文本上限为 200、多行文本上限为 1000 个 Unicode 标量值（`MAX_TEXT_ANSWER_LENGTH` / `MAX_TEXTAREA_ANSWER_LENGTH`），每题可以调低。调查说明、收集目的仍各允许 5000 个。旧草稿超过新上限时需手动调整，系统不自动截断内容或更改已发布版本；复制历史版本也按旧容量校验，得到的是一份需要调整后才能保存或发布的可编辑草稿。答案校验保留历史题面的结构检查，并按版本内保存的 `max_length` 执行。
- 预览与发布均按最坏情况估算一次完整提交的字节数（`domain::Document::estimated_submission_bytes`）；超过 `MAX_SUBMISSION_BYTES` 时，预览返回 `publishable=false` 和容量错误，发布返回相同错误的 `422`。该常量与 HTTP body limit 共用（1 MiB）。当前 100 道多行题各允许 1000 字时预算为 606856 字节，合法配置均可容纳；保留预算检查防止后续扩大容量时出现回退。文本预算包括两端引号和每标量最多六字节的控制字符转义，数字与评分按规范化前最多 64 字节计算。估算假设客户端使用紧凑 JSON、标准 UUID 和默认 UTF-8 字符串编码；主动转义非 ASCII 或添加大量空白的请求仍可能触发 `413`。预算仅限制新发布，不阻断历史版本的小载荷答卷及成功重试。
- 发布不替你保存客户端尚未上传的内容，也不会让已关闭问卷自动恢复回收。第一次发布前必须设置 `anonymous` 或 `login_once`；首次发布后模式冻结。
- `unpublished` 首次发布后变成 `collecting`；关闭或到期后视为 `closed`。到期后即使延长或清空 `deadline`，仍需显式调用 `resume`。`expire_survey` 先记住旧截止时间已到期的事实，再应用新设置，避免延期顺手抹掉关闭状态。
- `archive` 不可撤销，禁止继续编辑、发布、恢复；保留版本、答卷、报表和复制能力，复制历史容量问卷得到的是待调整的可编辑草稿。彻底删除只允许从未发布的问卷。
- `taken_down` 是独立治理标志。发布者恢复回收不能撤销管理员下架；管理员恢复问卷也不自动修改关闭状态或截止时间。

### 3.3 锁顺序为什么重要

回收相关写入使用相同顺序：**发布者用户行 `FOR SHARE` → 问卷行 `FOR UPDATE`**。`repository::lock_owned_survey` 把这个顺序集中起来，随后检查归属。公开读题面也使用短事务检查资格，避免把失效状态的题面继续作为可填写内容返回。

`AuthUser` 提取器在读取请求体之前完成认证，请求体可能迟到。因此所有者写入（新建、保存、复制、删除、发布、回收设置、关闭/恢复/归档）都经 `repository::lock_session_user`：锁定账号行后，在同一事务里用原 Cookie 重新 `authenticate` 并核对身份。退出、改密、禁用都要取得同一账号的 `NO KEY UPDATE` 锁，所以持锁后的重验看得到撤销结果，迟到请求返回 `401`，不会写入。

`submit_response` 通过 `lock_public_survey` 获取这些锁；登录限次同样经 `lock_session_user` 锁填写者用户行为 `SHARE` 并重验 Session。等待锁期间可能改密、退出或被禁用，所以不能只信任事务前提取出的用户身份。

登录创建 Session、修改密码以及管理员改变用户状态使用用户行 **`FOR NO KEY UPDATE`**，不是更强的 `FOR UPDATE`。它与提交持有的 `SHARE` 冲突，能把资格改变和提交排队；同时与外键检查用的 `KEY SHARE` 兼容，避免并发治理写审计时因不必要的强锁互相卡住。治理问卷仍遵循发布者 `SHARE` → 问卷 `UPDATE`，在同一事务写审计。

问卷行锁使关闭、发布新版本和提交在同一份问卷上有确定顺序。服务在拿到锁之后读取提交时间，再检查截止；不会把等待锁之前的旧时间当作提交发生时间。事务只有在全部校验和插入成功后才提交；错误不会留下半份答卷或单独占用限次资格。这保证业务一致性，但也意味着同一问卷的写入会排队，不能据此宣称高并发吞吐量。

### 3.4 幂等成功记录就是答卷的一部分

服务端只保存 UUIDv4 秘密的 SHA-256 `key_hash`，不保存明文秘密。作用域由问卷和填写模式决定：匿名是 `anonymous`；登录限次是账号 UUID。答案先按题目 UUID 排序、多选按选项 UUID 排序，再与 `version_id` 一起计算 `payload_hash`。

查到成功记录时，先匹配原版本，再用**原版本的不可变题面**规范化答案。匹配后返回原回执；改了版本或规范化答案则 `409 idempotency_conflict`。题目排列顺序、多选集合顺序、数字 `1` 与 `1.0` 不构成不同载荷；重复题目/选项仍是非法输入，而不是靠排序掩盖。

原成功请求可以在关闭、下架、发布者禁用或当前版本切换后合法重试，不占新名额；登录限次仍须有有效的填写者身份。没有成功记录的旧版本请求不能提交到新版本，返回 `version_changed`；客户端应保留输入、重新加载题面核对，不能自动换版本强行提交。

## 4. 数据边界与报表口径

### 4.1 匿名不是无法识别的承诺

- `anonymous` 不解析 Cookie 来关联填写者，`respondent_id` 为空；不使用设备指纹、地理位置来判定身份，也不承诺自然人限次或严格一人一票。
- `login_once` 内部保留账号关联以实现同账号跨版本一次，因此不能称为“完全匿名”。发布者的答卷详情、列表、统计和导出不含该账号字段。
- 问卷链接可被转发，不是保密访问控制。应用管理员只有治理权限，不能以管理员身份读取他人答卷；数据库账号、备份运维权限则能接触数据，不能与应用角色混为一谈。
- 密码用 Argon2id；随机 Session 秘密和提交秘密用 SHA-256。快速摘要适用于高熵秘密，不能拿来替代密码哈希。改密在同一事务删除该账号全部 Session，当前窗口和其他设备均需重新登录；禁用账号也撤销其 Session。

### 4.2 文本、缺失与数字

- 草稿允许不完整，但容量、ID 唯一性、数值有限性等基本约束始终有效。发布再要求标题、目的、至少一道题、有效题型配置；最多 100 题，每道选择题最多 50 个选项。单题上限之和可以合法超过单次提交的请求上限，因此发布期还按最坏情况总量预算把关（见 3.2）。
- 文字按 Rust `chars()`，即 Unicode 标量值计数，不是 UTF-8 字节数，也不等同于用户眼里的字形数量。非空白答案保留原文；不会静默截断、修剪或按内容判断“有效性”。
- `null`、空白文本、空多选视为缺失：可选题不存这一键，必填题拒绝。**合法数值 `0` 不等于缺失。** 单行文本拒绝换行，多行文本允许换行。
- 题面文字、文本答案、治理原因拒绝 `U+0000`（NUL），不把无法存入 PostgreSQL 文本/JSONB 的输入拖到数据库层再报内部错误。文字作为纯文本使用，不是可信 HTML。
- 数字题只接受 JSON 数字，拒绝字符串、布尔值、非有限值；边界是闭区间。`integer=true` 检查数学整数，评分还受 `i32` 整数范围和题面范围约束。
- 数字与评分的 JSON 表示在规范化前最多 64 字节（`MAX_NUMBER_JSON_BYTES`），包括负号、小数点和指数。与 `serde_json` 一致，省略的正指数符号 `+` 也计入长度；例如 `1e3` 按 `1e+3` 计数。超限返回题目级 `422`，前端数字输入执行相同校验，不先舍入再发送。
- 数字规范化和统计使用 **`f64`**，不是任意精度十进制或金额系统。`serde_json` 启用 `arbitrary_precision` 保留数字原文，`finite_number` 对没有小数点和指数的**原始裸整数**先检查 `i64::MIN..=u64::MAX` 范围，再核对转换成 `f64` 是否保持整数精度；不允许先舍入，再把两个不同整数当成同一份答案。`f64` 只有 53 位整数精度，例如 `9007199254740993` 会被拒绝；不是简单禁止所有大于 `2^53` 的数，部分更大的整数仍可精确表示。
- 带小数点或指数形式按有限 `f64` 语义处理，可能发生二进制浮点舍入；`1` 与 `1.0`、正零与负零按相同答案处理。原始整数检查不意味着整个 API 提供任意精度保证，也无法恢复客户端在发送前已经舍入的数字。需要精确十进制或大整数的调查，不应借此接口假装保存了完整精度。

### 4.3 版本、时间和统计

答卷列表、统计、CSV 一次只选择一个版本；省略 `version_id` 表示当前发布版本，不是合并全部历史版本。答卷详情始终附带其原版本题面。统计与 CSV 的 `response_snapshot` 一次 SELECT 固定结果，再从同一批数据计算总数、各题结果与趋势，不在同一响应里混用前后两次计数；不同请求之间仍可能有新答卷进入。

时间过滤使用 RFC3339，必须带 `Z` 或时区偏移；区间为 **`[from, to)`**：包含开始、不包含结束，两个值同时存在时必须 `from < to`。PostgreSQL 存储精度为微秒；过滤边界不能含非零的微秒以下精度，不会静默截断。客户端统一发送最多六位小数即可，额外全零尾位不改变时刻。不要把服务器本地时区或无时区日期字符串当作输入约定。

内部时间按 UTC 比较；报表的返回时间、CSV 提交时间和自然日趋势明确用 **Asia/Shanghai（当前业务固定 `+08:00`）**，不依赖服务器操作系统时区。查询含 `+08:00` 时要编码 `+`，否则 URL 查询解码可能把它当空格。例如在前面脚本之后：

```powershell
$From = [uri]::EscapeDataString('2026-01-01T00:00:00+08:00')
$To = [uri]::EscapeDataString('2027-01-01T00:00:00+08:00')
Invoke-RestMethod -Uri "$Base/api/surveys/$SurveyId/statistics?version_id=$VersionId&from=$From&to=$To" -WebSession $PublisherSession
```

统计口径：

- `total_count` 是当前版本及时间范围内的答卷数；`answered_count` 是真正回答该题的答卷数，排除缺失。
- 选择题 `proportion = 选项频次 / 本题 answered_count`，返回 `0..1` 比例，不是已乘 100 的百分数。多选合计可超过 100%。**分母为零时 `proportion=null`，不是 0 或 NaN。**
- 数字的均值、最小、最大排除缺失但包含合法零；没有回答时返回 `null`。评分均值同理。
- 评分 `distribution` 是按分值升序的**稀疏分布**，只列实际出现的分数，没出现的分数代表频次 0；不会为一个很大的评分范围分配每个整数桶。趋势同样省略没有提交的日期。
- 文本统计返回原始文本与回执/时间，不做自动情感分析或主观质量判定。

### 4.4 CSV 不是裸 JSON 的无损替代

`encode_csv` 生成 UTF-8 CSV，使用 CRLF 行结束和标准 CSV 转义。表头含回执、版本、上海时区提交时间及题目标题/UUID；选择题保存选项 UUID，多选在**一个 CSV 字段里保留 JSON 数组**，不是用逗号拼接；未答可选题留空。字段中的逗号、双引号和换行由 CSV 库处理。

`formula_safe` 检查每个单元格，包括表头：以 `=`、`+`、`-`、`@` 开头，或在前导空白、控制字符、常见不可见字符后出现这些前缀时，在**原字段最前面加单引号 `'`**，防止电子表格把调查内容当公式执行。原始数据库答案不因此被修改。负数也可能带这个文本保护前缀，这是导出安全语义，不是算错数；需要程序分析原始类型时优先用受权限保护的答卷 JSON。

## 5. 全部路由组与精确接口入口

精确字段、请求体、响应体、查询参数、错误码见启动服务后的 [OpenAPI JSON](http://localhost:3000/api/openapi.json)。它由 `lib.rs::openapi` 合并各业务模块的 `ApiDoc` 生成；仓库没有 Swagger 网页。此处只给路由导航，不手工重复全部 schema。更改端口或部署 origin 后，相应调整链接。

### 基础与认证

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/health` | 数据库就绪探测 |
| GET | `/api/openapi.json` | OpenAPI 定义 |
| POST | `/api/auth/register` | 注册普通账号，不自动登录 |
| POST | `/api/auth/login` | 登录并设置 HttpOnly / SameSite=Lax Cookie |
| POST | `/api/auth/logout` | 撤销当前 Session、清 Cookie，可重复调用 |
| GET | `/api/auth/me` | 当前有效账号 |
| PUT | `/api/auth/password` | 验证旧密码、更新密码、撤销全部 Session |

### 本人的问卷管理

除创建/列表外，`{id}` 均是本人的问卷 UUID；管理员走这些接口也不能越过归属检查。

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET / POST | `/api/surveys` | 筛选分页列表 / 创建草稿 |
| GET / DELETE | `/api/surveys/{id}` | 管理详情 / 删除从未发布的问卷 |
| PUT | `/api/surveys/{id}/draft` | 带 revision 保存整份草稿 |
| GET | `/api/surveys/{id}/preview` | 已保存草稿与发布校验结果，不产生答卷 |
| POST | `/api/surveys/{id}/copy` | 复制为新草稿 |
| POST | `/api/surveys/{id}/publish` | 发布指定 revision（内容未变时幂等返回当前版本） |
| PATCH | `/api/surveys/{id}/collection` | 模式与截止设置 |
| POST | `/api/surveys/{id}/close` | 关闭回收 |
| POST | `/api/surveys/{id}/resume` | 显式恢复回收 |
| POST | `/api/surveys/{id}/archive` | 不可撤销归档 |
| GET | `/api/surveys/{id}/versions` | 分页列出发布版本 |

### 公开填写与发布者报表

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/public/surveys/{id}` | 当前可填写题面与隐私提示；登录限次须认证 |
| POST | `/api/public/surveys/{id}/responses` | 整份提交或合法幂等重试 |
| GET | `/api/surveys/{id}/responses` | 本人问卷答卷分页列表 |
| GET | `/api/surveys/{id}/responses/{receipt_id}` | 一份答卷及原版本题面 |
| GET | `/api/surveys/{id}/statistics` | 本人问卷选定版本统计 |
| GET | `/api/surveys/{id}/export.csv` | 本人问卷选定版本 CSV |

### 管理员治理

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/admin/users` | 账号分页列表 |
| PATCH | `/api/admin/users/{id}` | `disabled` 与必填 `reason` |
| GET | `/api/admin/surveys` | 治理所需问卷元数据 |
| GET | `/api/admin/surveys/{id}` | 元数据与当前已发布题面，不返回他人草稿/答卷 |
| PATCH | `/api/admin/surveys/{id}` | `taken_down` 与必填 `reason` |
| GET | `/api/admin/audits` | 治理审计分页列表 |

通用分页从 `page=1` 开始，默认 `page_size=20`，范围 `1..=100`。请求体上限为 1 MiB（`domain::MAX_SUBMISSION_BYTES`，与发布期总量预算共用）。错误结构为 `{"error":{"code":"...","message":"...","question_id":null}}`，题目级错误会给出 UUID；JSON/路径格式错误通常为 `400`、无有效身份 `401`、拒绝权限或来源 `403`、资源不可访问 `404`、业务冲突 `409`、领域校验 `422`、限流 `429`。内部错误（`500`）的日志带 `error_id`、错误类别与发起位置，响应头 `X-Error-Id` 返回同一标识供检索；日志与响应都不含原始 SQL 或用户数据。管理接口对他人问卷使用不可访问结果，不以猜测 UUID 的方式提供查询入口。

## 6. 备份与独立恢复

**下面是操作步骤，不表示已经在你的环境演练成功。** [需求分析第 11.2 节](requirements-analysis.md#112-后端阶段实际执行的验证) 记录过一次在临时实例上用同类步骤完成的演练；当前 Compose 路径仍未在本机验证。首次实际演练应使用非敏感演示数据，保存命令退出结果和核对结果；不要在唯一的生产库上试恢复命令。

### 6.1 备份到受保护的文件

使用 PostgreSQL 18 客户端连接本指南的 PostgreSQL 18 服务。`pg_dump` 备份一个数据库，**不会替你备份集群角色和部署密钥**；另外安全管理角色配置、`.env` 和恢复所需凭据。先确定备份文件夹只允许本人/授权运维读取，不是公开同步目录；备份含密码哈希、Session 哈希、内部填写者关联及全部答案，应加密保管并设置保留期限。

```powershell
$BackupDirectory = Join-Path $env:LOCALAPPDATA 'questionnaire-backups'
New-Item -ItemType Directory -Path $BackupDirectory -Force | Out-Null
$DumpPath = Join-Path $BackupDirectory ('questionnaire-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.dump')
pg_dump -h 127.0.0.1 -p 5432 -U questionnaire -d questionnaire -W -Fc -f $DumpPath
if ($LASTEXITCODE -ne 0) { throw '备份失败，不要把此文件当作可恢复备份' }
```

必须使用 **`pg_dump -Fc -f 文件`**。不要写 `pg_dump ... > backup.dump`，也不要把二进制归档通过 PowerShell 文本管道传递，避免编码/重定向破坏文件。`pg_dump` 自身取得一致快照，不是按表随意复制；但若之后还在写入，实时库总数当然可能比该快照多。演练核对时暂停写入，或记录备份时对应的版本与时间筛选范围。

### 6.2 恢复到新的独立空库

在同一服务器上生成一个独立库名，避免误用原库。恢复目标不能先执行应用迁移，也不能含旧表。下面仍由数据库管理员创建库，应用专用账号负责恢复；每步密码交互输入：

```powershell
$RestoreDb = 'questionnaire_restore_' + (Get-Date -Format 'yyyyMMdd_HHmmss')
createdb -h 127.0.0.1 -p 5432 -U postgres -W --owner=questionnaire --encoding=UTF8 --template=template0 --locale=C $RestoreDb
if ($LASTEXITCODE -ne 0) { throw '独立恢复库创建失败' }
pg_restore -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W --exit-on-error --no-owner --no-privileges $DumpPath
if ($LASTEXITCODE -ne 0) { throw '恢复失败，此库不能当作完成恢复的结果' }
```

`--no-owner --no-privileges` 让对象由恢复账号拥有，不照搬原库的 ACL；正式恢复如有额外角色授权，应由授权运维明确重建，不能假设上述两项也恢复了权限。没有 `--clean`，没有覆盖原库，也不自动删除失败目标库。失败后先查清原因，再由你选择另一个新空库重新演练。

### 6.3 核对行数、版本关系和报表

先直接核对恢复库中的业务计数与版本对应关系：

```powershell
$CountSql = "SELECT 'users' AS table_name, count(*) AS row_count FROM users UNION ALL SELECT 'surveys', count(*) FROM surveys UNION ALL SELECT 'versions', count(*) FROM versions UNION ALL SELECT 'responses', count(*) FROM responses UNION ALL SELECT 'audits', count(*) FROM audits;"
psql -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W -c $CountSql
psql -h 127.0.0.1 -p 5432 -U questionnaire -d $RestoreDb -W -c "SELECT s.id AS survey_id, s.current_version_id, v.number, v.id AS version_id, count(r.id) AS response_count FROM surveys s LEFT JOIN versions v ON v.survey_id = s.id LEFT JOIN responses r ON r.survey_id = s.id AND r.version_id = v.id GROUP BY s.id, s.current_version_id, v.number, v.id ORDER BY s.id, v.number;"
```

将这些结果与暂停写入时的源库记录比较，不能只看到 `pg_restore` 退出成功就说业务恢复成功。随后验证应用确实能解释恢复数据：

1. 在**单独终端**从仓库根目录启动第二个 API 实例；通过进程环境变量把 `DATABASE_URL` 指向 `$RestoreDb`（密码仍需要 URI 编码），设置 `LISTEN_ADDR=127.0.0.1:3001`、`PUBLIC_ORIGIN=http://localhost:3001`、`COOKIE_SECURE=false`，运行 `cargo run --locked -p questionnaire-api`。不要改原实例的连接；恢复库已经含迁移记录，启动会照常检查迁移。
2. 在另一个客户端终端把 `$Base` 改为 `http://localhost:3001`，`Origin` 同步更新，使用**备份中已有发布者账号**重新登录，建立独立 `WebSession`。不要重新注册替代原账号，也不要复用原实例的会话；Cookie 不以端口隔离，更不应在同一个会话容器里混用两个本机实例。
3. 调用 `/health`、该问卷的 `/versions`、带明确 `version_id` 和相同时间区间的 `/statistics`；核对版本列表、每版本答卷数、`total_count`、关键选项频次/数值统计，并抽查一份答卷详情或 CSV。健康检查通过只证明连接，不证明数据完整。
4. 记录恢复所用归档、目标库名、命令结果及上述核对值，完成后停止恢复实例。恢复库与归档仍含敏感数据，按保留策略处理；不要因“演练结束”随意公开或上传。

恢复后的 `sessions` 也来自备份，仍在有效期内的旧会话可能继续有效。若真实事故恢复要求所有人重新登录，应由运维明确执行会话撤销并记录，而不是假定改了地址就使 Session 全部失效。

## 7. 当前交付边界

本文说明后端 API 与数据一致性实现；新增 Vue 前端及移动布局见 [前端指南](frontend-guide.md)。性能压测报告、公众长期运营的隐私合规流程、HTTPS/TLS 终止、部署反向代理与应用容器镜像不在项目范围内。全局内存限流只保护单实例固定窗口，不按身份判断人次，也不在多个实例间共享。对外运营还涉及告知同意、合法收集范围、数据保留与删除、运维权限及事件响应，不能用“匿名模式”三个字替代这些责任。
