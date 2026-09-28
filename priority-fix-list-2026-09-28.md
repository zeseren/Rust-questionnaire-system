# 优先修复清单：3 条用户可见 + 1 条运维可见（2026-09-28）

> 来源：[《防御性编程独立复核与取舍》](defensive-programming-independent-assessment-2026-09-28.md) §7「优先三件事」，外加从第二批提前的内部错误诊断缺口（本文编号 ④）。
> 状态：**已于 `improve/priority-fixes` 分支实施**，实现与验证证据见 §6。
> 证据边界：修复前的问题判断来自静态源码核对；本次实施后按仓库约定用真实 PostgreSQL 与真实服务冒烟（§6），未做压测与浏览器闭环。§①—§④ 的行号对应修复前版本。

## 0. 摘要

| # | 问题 | 可见性 | 决策结果 | 是否改公开契约 |
|---|---|---|---|---|
| ① | 合法问卷填不完（容量与提示不一致） | 填写者 / 发布者 | 发布期约束总预算 | 否（新增 422 场景，不缩已有输入范围以外的契约） |
| ② | 发布结果不确定（重试产生第二个版本） | 发布者 / 填写者 | 内容一致即幂等 | 否（新增 200 幂等响应） |
| ③ | 前端 30s 超时管不到响应正文 | 所有用户 | 无需决策 | 否 |
| ④ | 500 不可定位（日志只有固定字符串） | 运维 / 开发者 | 无需决策 | 否（仅日志字段 + 响应头） |

共同点：① 是可预测却无人提示的容量冲突；② 是「重试不安全」的结果不确定；③ 是「等待无期限」；④ 是前三者发生时查不到原因。

## ① 合法问卷填不完（容量与提示不一致）

**症状**：填写者逐题都合法，整份提交被拒：`413 body_too_large`（文案「请求体不能超过 1 MiB」）。100 题 × 5000 中文字符 ≈ 1.5 MB（UTF-8 3 字节/字符），必然超限。

**证据（修复前）**：

| 事实 | 位置 |
|---|---|
| 领域允许 100 题、单题 5000 个 Unicode 标量值 | `backend/src/domain.rs:15,19`（校验见 `:203-204,238-239`） |
| HTTP 全局上限 1 MiB | `backend/src/lib.rs:135` |
| 413 文案与错误码 | `backend/src/http.rs:29-35` |
| 前端对 413 / 体积零处理（无预检、无提示） | `frontend/src/` 全仓无 `body_too_large` 处理 |

独立复核报告本地构造的紧凑 JSON 为 1,506,722 字节（其 §4.1），与上述算术一致。

**用户代价**：填到最后才发现无解，只能回头删内容；发布规则与填写页面都不提示这条组合上限。

## ② 发布结果不确定（重试产生第二个版本）

**症状**：发布请求超时（响应丢失，实际已成功）→ 用户重试 → 又插入一个内容相同的新版本 V2。报表按版本统计被拆成两份；仍持有 V1 的填写者提交时被 `409 version_changed`（文案「问卷已更新，请保留当前输入并重新加载题面核对」）打回——一次误重试使所有已打开页面失效。

**证据（修复前）**：

| 事实 | 位置 |
|---|---|
| 发布只校验草稿 revision，成功后不推进 revision | `backend/src/surveys.rs:837`；`ensure_revision` 定义 `:1051-1058`；发布事务体 `:846-893`（仅 `number = max+1` 插版本 + 更新 `status/current_version_id/updated_at`） |
| 草稿保存会推进 revision（对照） | `backend/src/surveys.rs:763-770` |
| 前端发布分支不更新 `revision`（只更新 `survey.value`） | `frontend/src/views/EditorView.vue:166-177` |
| 旧版本提交被拒 | `backend/src/submissions.rs:257-261` |

## ③ 前端 30s 超时管不到响应正文

**症状**：响应头已到、正文传输慢或连接挂住时，UI 无限等待——GET 无提示地卡死；POST 可能已在服务端提交完成，用户却永远等不到结果，刷新后也不知道写入是否生效。

**证据（修复前）**：

| 事实 | 位置 |
|---|---|
| 定时器在 `finally` 清除（fetch 返回响应头即清） | `frontend/src/api.ts:37,75`（原行号） |
| 成功正文在请求函数返回之后才读取，处于超时窗口之外 | 原 `api()`、原 `download()` |
| 错误响应正文在请求函数内部读取，受保护（对照） | 原 `api.ts:53` |

即「错误有超时、成功没有」。独立复核报告 §5 用内存转译实验验证：30 ms 截止、100 ms 正文延迟，实际 129 ms 未中止。

## ④ 500 不可定位（运维可见）

**症状**：线上任何 500，日志只有固定字符串 `"internal operation failed"`，响应只有「服务暂时无法完成请求」。查询失败、唯一约束冲突、脏数据、序列化失败、内部违约全部无法区分，也没有请求关联标识供用户回传。

**证据（修复前）**：`AppError::internal(_error)` 丢弃错误对象，仅记录固定字符串（`backend/src/error.rs:76-83`）；响应契约 `{code, message, question_id}` 也带不出 id。

## 5. 显式边界与未纳入范围

- ① 的估算假设客户端以 UTF-8 直传文本；主动把非 ASCII 转义成 `\uXXXX` 的客户端仍可能触发 413，这是显式写明的边界。
- ② 只处理「内容与当前版本完全一致」的重放；草稿确实被改过后的发布仍正常产生新版本。
- ④ 的标识只进日志与响应头，前端不展示；是否在界面提示单列决策。

## 6. 实施与验证结果（2026-09-28，`improve/priority-fixes`）

| # | 实现 | 验证证据 |
|---|---|---|
| ① | `domain.rs` 新增 `MAX_SUBMISSION_BYTES`（`lib.rs` 的 body limit 改为引用它）、`Document::estimated_submission_bytes` 与 `ensure_submission_budget`；`publish_survey` 在发布事务内调用，超限返回 422 | 领域测试 `submission_budget_rejects_surveys_that_cannot_fit_one_request`、`estimated_submission_bytes_covers_a_worst_case_payload`；真实服务：60 题 × 5000 的问卷 `preview` 显示 `publishable=true`，`publish` 返回 `422 validation_failed`，文案「问卷最坏情况下需要约 1204096 字节提交，超过单次提交上限 1048576 字节…」 |
| ② | `publish_survey` 在 revision 校验后比较草稿与当前版本文档，内容一致时返回 `200` 与当前版本（不新增版本）；handler 透传状态码；OpenAPI 增加 200 响应 | 真实服务：同一 revision 二次发布 → `200`，`version.id` 与首次完全一致；改稿保存后发布 → `201`、`version.number=2`；版本列表 `total=2` |
| ③ | `api.ts` 重构：`send()` 返回响应与定时器句柄，`api()`/`download()` 在读完 `text()`/`blob()` 后才清除，失败路径立即清除 | 临时 harness（真实 HTTP 服务器延迟正文 + 真实 undici fetch，把 30 s 压缩为 50 ms）：修复前 3037 ms 完成、修复后 63 ms 中止（`status=0 code=invalid_response`）；截止 5 s 时 3 s 正文正常完成 3025 ms；`npm run build` 与 `npx prettier --check src/api.ts` 通过 |
| ④ | `AppError::internal` 生成 16 位 error id，并记录 `error_id`/`error_kind`/`location`（`#[track_caller]` 贯穿 `?` 与 `From` 转换）；内部错误响应加 `X-Error-Id` 头，响应体不变 | 真实服务：把 `versions.document` 改成过 CHECK 但过不了 `Document` 反序列化的对象后，`GET /api/public/surveys/{id}` → `500` + `X-Error-Id: 0340f9567bb54619`；日志同一 `error_id`，`error_kind="serde_json::error::Error"`，`location=backend\src\submissions.rs:192:20` |

**门禁**：`cargo fmt --all -- --check`、`cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo test --locked --workspace`（10/10 通过）、`npm run build`、`npx prettier --check src/api.ts`。

**中途环境修复**：`E:\.cargo\config.toml` 存在重复的 `[source.ustc]` 表，导致任何 cargo 命令解析失败；删除较早的非 sparse 版本、保留 sparse+ 条目后恢复（仓库外改动）。

**未覆盖**：未做压测、未做浏览器闭环；③ 的 harness 用 Node 的 undici 与真实 HTTP 服务器，未在浏览器实测；④ 的前端展示未实现。
