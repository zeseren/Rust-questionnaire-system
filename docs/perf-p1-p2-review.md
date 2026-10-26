# P1–P2 性能优化代码审查报告与待办记录

> 审查范围：`main` (`02b0cbf`) → `perf/p1-p2-optimizations` (`5bc5fdf`)，共 21 个文件（+1898 / -1023 行）。
> 审查方式：8 个并行 `reviewer` 子代理按模块局部性分片独立审查。

---

## 一、分片审查结论总览

| 分片代理             | 负责文件                                                                               | 结论 (Verdict)         | 置信度 | 摘要                                                                                                                                                                                         |
| -------------------- | -------------------------------------------------------------------------------------- | ---------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ReviewBackendCore2` | `backend/src/lib.rs`<br>`backend/src/tests/support.rs`<br>`AGENTS.md`                  | `completed_with_notes` | 0.90   | `AppState.versions` 在全部构造点挂载一致；OpenAPI `Bytes` 预序列化零拷贝、响应头与 404/405 路径不变；1 项文档描述细节建议。                                                                  |
| `ReviewRepoCache2`   | `backend/src/repository.rs`                                                            | `completed`            | 0.93   | `VersionCache` 读写锁绝不跨 `.await`、`survey_id` 归属过滤完备、256 上限淘汰安全；`SurveyMetaRow` 10 列投影与 `FOR UPDATE` 行锁有效；`users → surveys` 锁序与 Session 重验完整保留。无缺陷。 |
| `ReviewSubmitAuth2`  | `backend/src/submissions.rs`<br>`backend/src/auth.rs`                                  | `requires_fixes`       | 0.70   | 提交与公开题面路径的错误码、检查顺序、行锁、`HashWriter` 均与 `main` 等价；发现 1 项中危锁序问题（`auth::login` 全表清理过期会话）与 2 项低优建议。                                          |
| `ReviewSurveyAdmin2` | `backend/src/surveys.rs`<br>`backend/src/admin.rs`                                     | `completed_with_notes` | 0.85   | 写路径去 `draft` 后的状态码、事务锁序、缓存预热与审计原子性正确；发现 1 项低危时间戳微秒精度差异与 2 项拷贝优化建议。                                                                        |
| `ReviewReports2`     | `backend/src/reports.rs`<br>`backend/src/schema.sql`                                   | `completed_with_notes` | 0.90   | `load_report` 快照内版本归属校验、空样本短路、`spawn_blocking` 双重解包、CSV 逐字节输出与 3 个新复合索引全部正确；1 项统计题目克隆建议。                                                     |
| `ReviewDomainFlow2`  | `logic-core/src/domain.rs`<br>`logic-core/src/flow.rs`                                 | `completed`            | 0.92   | `PreparedDocument`、`FlowEvaluator` `Cow` 借用复用、`validate_answers_only` / `evaluate_stored_states`、`FlowSnapshotRef` 序列化、单遍文本校验均与 `main` 逐项等价。无缺陷。                 |
| `ReviewLogicDoc2`    | `logic-core/src/logic.rs`<br>`logic-core/src/logic_document.rs`                        | `completed`            | 0.92   | `Binding.options: Box<[Uuid]>` 二分查找、Kahn 拓扑排序（`BTreeSet::pop_first` 最小 UUID 确定性顺序与环检测）、`effective_flow` `Arc<[Node]>` 共享及 `check_budget` 全部正确。无缺陷。        |
| `ReviewTextWasm2`    | `logic-core/src/logic_text.rs`<br>`logic-wasm/src/lib.rs`<br>`frontend/src/wasm/pkg/*` | `completed_with_notes` | 0.88   | `Formatter` 单缓冲区输出逐字节等价、Pratt `Budget` 增量预算消除 $O(N^2)$ 且阈值/偏移不变、`Location` 惰性路径与 WASM 信封结构一致；1 项生成产物格式建议。                                    |

---

## 二、审查发现的问题清单（后续跟进）

### 1. [中危 / P2] `auth::login` 全表清理过期会话跨账号加锁

- **位置**：`backend/src/auth.rs:187-192`（`login`）
- **问题描述**：
  ```rust
  let current = lock_unchanged_credentials(&tx, candidate.id, &candidate.password_hash)
      .await?
      .ok_or_else(invalid_credentials)?;
  session::Entity::delete_many()
      .filter(session::Column::ExpiresAt.lte(Utc::now()))
      .exec(&tx)
      .await?;
  ```
  事务仅持有当前登录用户 `candidate.id` 的 `users` 行锁，但 `delete_many()` 未限定 `user_id`，会对全表所有账号的过期 `sessions` 行加锁。若另一账号 Y 同时执行 `change_password`（`auth.rs:291`）或管理员执行 `change_user`（`admin.rs:122`，先锁账号 Y 再按 `sessions_user` 索引删 Y 的全部会话），两条 `DELETE` 在重叠过期行上可能因索引扫描顺序不同（`sessions_expiry` vs `sessions_user`）形成锁等待环触发 PostgreSQL 死锁（返回 500）；且全表过期行积压较多时会延长当前用户 `users` 行锁持有时间。
- **修复方案**：
  在 `login` 的清理语句上追加 `.filter(session::Column::UserId.eq(candidate.id))` 仅清理当前登录账号的过期会话（满足“持谁的 `users` 锁、只动谁的 `sessions` 行”），或将全局过期会话清理移至事务外的独立定时/后台维护任务。

---

### 2. [低危 / P3] `surveys` 写路径去除 `RETURNING` 后返回未截断纳秒的 `Utc::now()`

- **位置**：`backend/src/surveys.rs:550-551, 581-592, 685, 703-707, 772, 830-873`
- **问题描述**：
  PostgreSQL `timestamptz` 仅保存微秒精度（6 位小数），且 `reports.rs:464-475` 的 `TimeRange` 显式拒绝 7–9 位亚微秒小数。改造前由 `RETURNING` 从数据库读回微秒截断值；改造后直接将 `Utc::now()`（9 位纳秒精度）填入响应 DTO，导致写响应中的 `created_at` / `updated_at` / `published_at` 与后续从数据库读出的 6 位微秒值存在亚微秒差异。
- **修复方案**：
  在生成 `now` 时统一截断到微秒精度：`DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).expect("当前时间戳在合法微秒范围内")`。

---

### 3. [建议 / P3] 非阻塞性能与文档改进项

| 编号   | 位置                                                             | 问题描述                                                                                                                                              | 建议方案                                                                                                                            |
| ------ | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| **S1** | `backend/src/surveys.rs:778-795` (`publish_survey`)              | 幂等重放分支（`current.document == record.draft`）在重放判断前已构造 `PreparedDocument` 并 `prepared.document().clone()`，重放时编译产物直接被丢弃。  | 先做 `document.validate_publish()?` 与幂等重放判断（重放时直接 move `document` 返回），仅在确认新建版本时调用 `prepare_publish()`。 |
| **S2** | `backend/src/reports.rs:741-747` (`summarize_report`)            | 因 `PreparedDocument` 由 `Arc` 共享，`summarize_report` 使用了 `.questions.iter().cloned()`，每次统计请求克隆了每道 `Question`。                      | 将 `summarize_question` 与 `summarize_choices` 改为接收 `&Question` / `&[Choice]` 借用。                                            |
| **S3** | `backend/src/submissions.rs:238`<br>`backend/src/reports.rs:342` | `PublicSurvey` 与 `ResponseDetail` 响应 DTO 仍持有 `Document` 值类型，缓存命中时需 `version.prepared.document().clone()` 一次。                       | 将 `PreparedDocument` 内部及只读响应 DTO 的 `document` 升级为 `Arc<Document>` 实现序列化零拷贝。                                    |
| **S4** | `backend/src/surveys.rs:720` (`copy_survey`)                     | 归档分支先通过 `lock_owned_survey` 读取了整行 `draft` 随后丢弃，再去读 `versions` 表整行。                                                            | 改用 `lock_owned_survey_meta`，按分支分别单列投影 `version::Column::Document` 或 `survey::Column::Draft`。                          |
| **S5** | `backend/src/auth.rs:269-275` (`change_password`)                | `tokio::try_join!` 并行执行旧密码校验与新密码哈希，在旧密码错误的失败路径上也会消耗 2 次 Argon2id 计算。                                              | 属成功路径耗时减半的刻意权衡；若需防滥用可单独收紧改密接口限流配额。                                                                |
| **S6** | `AGENTS.md:28`                                                   | 文档描述“公开/认证接口按需省略 `AuthUser` 或 `HeaderMap`”，而 `submissions.rs` 两个公开 handler 的实际参数顺序为 `(State, ApiPath, HeaderMap, ...)`。 | 调整文档表述或统一 `submissions.rs` 的 `HeaderMap` 抽取位置。                                                                       |
| **S7** | `frontend/src/wasm/pkg/*`                                        | `wasm-pack` 生成产物与前端 Prettier（单引号、无分号、2 空格）风格存在格式差异。                                                                       | 在 `wasm:build` 后执行 `prettier --write src/wasm/pkg` 保持生成产物格式一致。                                                       |

---

## 三、深度审查发现与已落地重构（2026-10-26）

在上述报告基础上又做了一轮以「单一事实来源、按需投影、消除隐式深拷贝」为目标的深度审查。新增发现与修复结果如下。

### 1. `auth::login` 跨账号清理会话（关闭问题 1）

- `session::Entity::delete_many()` 追加 `.filter(session::Column::UserId.eq(candidate.id))`，只清理当前已持锁账号的过期会话。
- 保持 `users → sessions` 固定锁序：不再与其它账号的 `change_password` / `change_user` 删除重叠行，消除基于不同索引扫描顺序的死锁窗口，也不再延长全局过期行清理对 `users` 行锁的持有时间。

### 2. 微秒精度时间戳统一（关闭问题 2）

- 新增 `repository::now_micros()`：`DateTime::from_timestamp_micros(Utc::now().timestamp_micros())`。
- `surveys.rs` 的 `detail`、`create_survey`、`save_survey_draft`、`copy_survey`、`publish_survey`、`update_collection`、`change_status` 与 `admin.rs::change_survey` 全部改用该函数；写响应中的 `created_at` / `updated_at` / `published_at` 与 PostgreSQL 读回值一致。
- 仅用于状态物化比较的 `Utc::now()` 保持原样，不参与任何返回时间戳。

### 3. `reports.rs` 内存放大、题目克隆与求值器双轨（关闭 S1–S3）

- 文本题答案容器改回 `Vec::new()`，不再按答卷总数预留。
- `summarize_question` 接收 `&Question`、`summarize_choices` 接收 `&[Choice]`；`summarize_report` 删除 `.questions.iter().cloned()`，统计请求不再克隆题目与选项。
- 删除仅测试使用的 `report_evaluator` 双轨入口；`snapshot_row` 改为接收 `&PreparedDocument`，内部按 `prepared.document().logic.is_some()` 调用 `prepared.evaluate_stored_states(&answers)`，报表重建与提交校验共用同一份编译产物。
- `ResponseDetail.document` 与 `PublicSurvey.document` 改为 `#[schema(value_type = Document)] Arc<Document>`，经 `PreparedDocument::document_arc()` 返回共享句柄，缓存命中时不再深拷贝 1 MiB 题面；后端 `serde` 启用 `rc` 特性以保持 JSON 序列化结果不变。

### 4. `SurveyMetaRow` / `VersionLookup` 全面 Clean Cutover（关闭 S4 及额外发现）

- 新增 `repository::VersionLookup::{Hit, Miss}`、`lookup_version()` 与 `VersionLookup::resolve()`，替换 `submissions.rs` 公开题面、`reports.rs` 答卷详情与 `load_report` 三处手写的 `Result<CachedVersion, VersionRow>` 反模式；事务提交后统一通过 `resolve` 解码、编译并写缓存。
- 新增 `repository::load_survey_draft()` 单列投影，写路径构造响应时不再搬回 1 MiB 草稿。
- `copy_survey` 改用 `lock_owned_survey_meta`：归档分支只投影 `version::Column::Document`（缓存命中直接复用），非归档分支调用 `load_survey_draft`。
- `update_collection` 与 `change_status` 改用 `lock_owned_survey_meta` + `expire_survey` + `update_without_returning`，不再用 `RETURNING *` 回传草稿；最后只读一次草稿列构造详情。
- `publish_survey` 只读一次草稿列并只调用一次 `prepare_publish`；幂等重放分支用 `prepared.into_document()` 零克隆返回，新版本分支再用 `Arc::new` 写入缓存。
- 删除重复的 `lock_survey`、`lock_owned_survey` 与基于 `survey::Model` 的 `expire_survey`；`expire_survey_meta` 更名为唯一的 `expire_survey(SurveyMetaRow)`；`ensure_editable`、`ensure_revision`、`survey_detail` 只保留基于 `SurveyMetaRow` 的一份实现。

### 5. 测试与文档同步

- `backend/src/tests/reports.rs`：逻辑重建用例改用 `PreparedDocument::new(夹具.文档)`，所有统计用例以 `&题` / `&多行题` 调用 `summarize_question`。
- `backend/src/tests/surveys_and_submissions.rs`：`ensure_editable`、`ensure_revision`、`survey_detail` 用例改用 `SurveyMetaRow::from(&record)` 与已解析 `Document`。
- 未改动的待办：S5（改密并行 Argon2id 的刻意权衡）、S6（`AGENTS.md` 参数顺序描述）、S7（WASM 生成产物格式化）保持原建议。
