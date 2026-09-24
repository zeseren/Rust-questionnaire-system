# 后端代码审查工作存档

存档日期：2026-09-15（Asia/Shanghai）。

## 1. 任务与暂停位置

原始任务：按照 `F:\rust-skills` 的指导，对 `F:\RustWorkX` 的后端代码进行一次 review。

用户要求先存档，之后恢复。当前已完成主要模块的静态阅读与现有构建检查，正在验证候选问题，**尚未提交最终审查报告，也未修改后端实现**。下文的候选问题与运行验证结果分开记录，不把预期复现结果当成已经发生的结果。

恢复时应继续验证、筛选问题并形成按严重程度排序的报告；未经新的修复指令，不直接修改业务代码。

**2026-09-15 当日已完成恢复与验证**：第 4 节的四项候选问题全部在临时实例上复现，结果与结论见第 9 节。未修改任何后端业务代码。

## 2. 已遵循的指导与审查范围

- 仓库指导：根目录 `AGENTS.md`。中文文档、单成员 Cargo workspace、既有三层边界、PostgreSQL 18、统一用户与问卷锁顺序、Cookie 与 Origin 校验等约束继续适用。
- 用户指定技能：`F:\rust-skills\SKILL.md`。已阅读规则索引及相关规则，重点涉及错误处理、异步锁、阻塞工作、数值转换、严格反序列化、敏感信息、内存分配与反模式。
- 已核对入口、配置、统一错误、HTTP 提取器、安全中间件、认证、问卷生命周期、公开提交、报表、管理员治理、领域规则、DDL 与迁移，以及现有领域测试。
- 已对照 `docs/requirements-analysis.md` 的业务规则及后端验证记录、`docs/backend-guide.md` 的接口约定。
- 仅为判断后端问题影响，查看了前端实际发送分页参数的调用位置；前端不属于本次修改范围。
- 没有使用子代理，没有增加永久测试工程，没有开展性能压测。

`context7-mcp` 指导文件也已读取，但当前工具列表没有 Context7 接口，未调用该服务。关于表单查询反序列化的判断已核对本地安装依赖源码，仍待真实请求确认。

## 3. 已完成的验证

下列命令均从 `F:\RustWorkX` 执行并通过：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --locked --workspace` | 7 个领域集成测试通过；库、二进制及文档测试没有额外用例 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo build --locked -p questionnaire-api` | 通过，供临时 HTTP 复现使用 |

这些结果不证明 HTTP、数据库并发或下面的候选问题已经验证。恢复后无需无理由重复上述检查。

## 4. 候选问题与待验证步骤

### A. 显式分页参数可能使多个列表返回 400（候选 P1）

位置：

- `backend/src/surveys.rs:133`：`SurveyFilters` 通过 `serde(flatten)` 引入 `Pagination`。
- `backend/src/reports.rs:56`：`ResponseListQuery` 使用两个扁平字段，包括 `Pagination`。
- `backend/src/admin.rs:53`：`ListQuery` 扁平化 `Pagination`，用于三个管理员列表。
- `backend/src/http.rs:65`：分页字段类型是 `Option<u64>`。

依据：当前 `serde_urlencoded 0.7.1` 在 `deserialize_any` 中提供字符串，直接的 `deserialize_u64` 才负责从字符串解析整数。Serde 的扁平字段缓冲会失去原始解析器的数值转换行为，因此可能把 `page=1` 当作字符串交给要求 `u64` 的字段，最终由 `ApiQuery` 返回 `invalid_query`。

已查看的依赖源码：

`E:\.cargo\registry\src\mirrors.tuna.tsinghua.edu.cn-4dc01642fd091eda\serde_urlencoded-0.7.1\src\de.rs`，重点是第 176、194、256 行附近。

前端影响依据：`DashboardView.vue:32`、`:39`，`AdminView.vue:21`，`ReportsView.vue:68` 均显式传入分页参数。若复现成立，会影响工作台、答卷及管理员列表。

待验证：登录后分别请求不带参数和带 `?page=1&page_size=20` 的问卷、答卷及管理员列表；以直接提取 `Pagination` 的 `/api/surveys/{id}/versions` 作为对照。**尚未取得这些 HTTP 响应。**

修复方向：让 URL 查询 DTO 直接解析分页字段，或为扁平字段提供兼容查询字符串的数值反序列化，再交给现有 `Pagination::bounds()` 检查范围。不要只修改前端来回避后端契约问题。

### B. Session 撤销后迟到的问卷写请求可能仍可执行（候选 P1，需确认语义与复现）

位置：`backend/src/surveys.rs:341`、`:453`、`:481` 等写入处理器，以及 `backend/src/repository.rs:23`、`:51`。

依据：处理器在等待 JSON 请求体之前提取 `AuthUser`，业务函数随后只收到用户 ID。取得用户及问卷锁后，`lock_user` 只检查当前账号是否禁用，没有重新验证原 Cookie 对应的 Session。退出或改密会删除 Session，但不会使普通账号的 `disabled` 变为真。

对照：`auth.rs` 的改密、`submissions.rs` 的登录限次提交，以及 `admin.rs` 的治理写入，已经存在锁后重新认证逻辑；管理员延迟请求体问题也在既有验证记录中提到。

待验证的确定性时序：

1. 登录并创建问卷。
2. 向保存草稿接口发送有效认证头、`Expect: 100-continue` 和请求体长度，暂不发送正文。
3. 收到 `100 Continue`，确认处理器已通过认证并开始等待正文。
4. 另一连接退出当前 Session，确认 `/api/auth/me` 返回 401。
5. 再发送原保存请求正文，检查是否仍返回 200 并写入新草稿。

**尚未执行到该时序，不应写成已成功复现。** 最终报告应明确这是撤销过程中已经进入处理器的迟到请求，不是声称撤销之后新发起的请求也能通过认证。

修复方向：在统一写入边界取得相关锁后，使用同一事务重新验证原 Session，并核对主体身份；需要保留现有锁顺序。

### C. 题目反序列化静默丢弃未知字段（候选 P2）

位置：`backend/src/domain.rs:62` 的自定义 `Question::deserialize`、`:77` 的 `Fields` 与 `:96` 的 `QuestionKind`。

依据：外层请求及 `Document` 有 `deny_unknown_fields`，但单道题目先转换为 `Value`，内部扁平字段与题型枚举没有严格拒绝多余字段。此处与技能 `serde-deny-unknown-fields` 及仓库严格请求约定不一致，且可能丢失用户意图中的校验规则。

待验证示例：完整数字题中设置 `integer: false`、`minimum: 5`、`max: 10`，把下界 `min` 拼错为 `minimum`。检查是否创建成功且返回 `min: null`；发布后提交数值 3，检查是否被接受。应当在输入阶段明确拒绝拼错字段。

**尚未取得示例的实际运行结果。** 修复时需要处理 `flatten` 与严格字段校验的组合，不应未经验证地机械添加属性。

### D. 中间件提前拒绝时没有添加统一安全响应头（候选 P3）

位置：`backend/src/security.rs:76`、`:85`、`:90` 与 `:96`。

依据：CSRF 失败直接 `return Err(...)`，限流失败通过 `?` 提前返回；`Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff` 仅在 `next.run(request).await` 之后插入，所以提前生成的 403、429 不经过该逻辑。

待验证：检查缺少 CSRF 头的写请求，以及超过临时认证限额的请求响应头。此项属于响应契约一致性，优先级低于列表功能和认证问题，不应夸大为已经发生敏感数据泄露。

修复方向：将安全头放在覆盖成功与错误响应的外层，或先把拒绝结果转换成响应再统一补头。

## 5. 已排除或暂不作为问题的事项

- 限流使用的同步互斥锁没有跨越 `await`；不应机械建议替换为异步锁。
- Argon2 运算已经放入 `spawn_blocking`。
- 数值答案使用 `f64` 及裸整数精度检查是明确记录的约定；不能把不支持任意精度金额运算当作缺陷。
- 成功幂等重试允许跨关闭、下架及切换版本，是明确业务要求。
- 报表的单条 SELECT 固定结果可以提供该次统计或导出的一致快照，不应仅因未显式开启事务就判定不一致。
- 报表目前全量加载答卷并同步处理，存在规模方面的后续评估空间，但本次没有压测证据，且项目明确限制 MVP 验证范围，尚未将其列为确定缺陷。
- 不建议为了满足技能的通用推荐，擅自加入 workspace 依赖继承、发布配置、额外测试框架或 CI。

## 6. 临时复现环境与中断状态

临时目录：`F:\RustWorkX\target\backend-review-20260915-0135`。

- `check.py`：已准备好的最小复现驱动，使用 Python 标准库、临时 PostgreSQL 和构建出的 API；不会打印 Cookie 或密码。
- `data\`：本次初始化的 PostgreSQL 18.1 空集群。
- `postgres.log`：临时数据库日志。
- `api.log`：计划使用的 API 日志；存档前检查时尚不存在。

第一次在沙箱内启动 PostgreSQL 失败：`pg_ctl: could not create restricted token: error code 87`。随后已获准在沙箱外运行该复现脚本。

截至 09:57 的最后一次检查：数据库日志显示临时 PostgreSQL 已于 09:54 启动并就绪；`postmaster.pid` 记录 PID `22632`、监听地址 `127.0.0.1`、端口 `61535`。统一执行会话编号为 `59709`，尚未收到脚本的 HTTP 检查输出。**这些 PID 和端口只作为当时记录，恢复时必须重新检查，不能直接据此终止进程。**

脚本可能停留在 `subprocess.run(pg_ctl start, capture_output=True)` 等待子进程管道关闭的阶段；这只是当前推测，未确认。恢复时先修正启动方式：后台启动应隐藏窗口，标准输出与错误写入本次临时文件，避免持有等待关闭的管道；不要直接重复启动脚本或创建第二个实例。

暂停收尾已完成：在获得沙箱外执行权限后，只匹配本次脚本路径，停止了 1 个 Python 驱动进程；对上述临时数据目录执行 `pg_ctl stop -m fast` 成功，退出码为 0，`postmaster.pid` 已消失。没有继续运行 HTTP 复现。

停止后的临时数据目录、脚本和日志保留，便于恢复。临时目录位于被忽略的 `target` 下，可能被 `cargo clean` 删除；即使脚本丢失，也可以依据第 4 节恢复最小验证。恢复时重新选择端口，不依赖先前的 PID 或端口。

## 7. 仓库与工具注意事项

开始审查前就存在以下用户工作区改动，均未由本次 review 修改：

```text
 M .gitignore
 M AGENTS.md
 M README.md
 M docs/backend-guide.md
 M docs/requirements-analysis.md
?? docs/frontend-guide.md
?? frontend/
```

当前沙箱账号与仓库所有者不同，普通 `git status` 会报告 dubious ownership。只对本次命令使用：

```powershell
git -c safe.directory=F:/RustWorkX status --short
```

没有修改全局 Git 配置。该命令还会提示无法读取用户全局 ignore 文件，但仍能列出仓库状态。

本机已发现的工具位置：Rust/Cargo 位于 `E:\.cargo\bin`，PostgreSQL 工具位于 `E:\pgsql\bin`，Python 位于 `E:\python\python.exe`。沙箱内 `Get-CimInstance Win32_Process` 被拒绝访问；临时服务清理可能需要继续使用沙箱外执行权限。

## 8. 恢复顺序

1. 阅读本存档及根目录指导，查看仓库状态，保留用户已有改动。
2. 先核实本次临时进程的暂停收尾记录；仅管理能够确认属于上述临时目录的进程。
3. 修正临时脚本的进程启动/输出处理，再进行第 4 节的最小复现；不扩大为全面联调或性能测试。
4. 优先验证分页问题，其次验证迟到写入与 Session 撤销，再验证未知字段及错误响应头。
5. 将实际状态码、错误码和简短复现结果补入审查结论；剔除无法成立或缺乏实质影响的候选项。
6. 停止临时实例，最终检查工作区，向用户提交包含严重程度、精确代码位置、触发条件、影响和修复建议的中文审查报告。

最终报告还需说明现有检查通过，以及真实运行验证的实际范围。当前任务仍是审查，不包含自动修复。

## 9. 恢复后的复现结果（2026-09-15 完成）

### 9.1 运行方式与证据

沿用第 6 节的临时环境：同一 `data\` 空集群（`review_admin`、trust 认证）加独立 API 实例，两者都用随机空闲端口，不接触本机 5432 服务，也不改动用户工作区文件。脚本已修正启动方式（`pg_ctl start` 改为标准输出写文件，不再持有未关闭的管道），并改为随机账号名避免与上次运行的记录冲突。

```powershell
python target\backend-review-20260915-0135\check.py   # 主复现
python target\backend-review-20260915-0135\probe.py   # 分页粒度与严格性细分
```

两条脚本都打印 JSON 行，运行结束会自行停止 API 与 PostgreSQL；结束时 `postmaster.pid` 已消失，没有遗留 python 或 questionnaire-api 进程。脚本和数据目录保留在 `target\`（被忽略），可重复执行。

### 9.2 四项候选项的实际结论

| 候选项 | 结论 | 关键证据 |
| --- | --- | --- |
| A 显式分页返回 400 | 复现，P1 | `/api/surveys`、`/api/admin/users`、`/api/admin/surveys`、`/api/admin/audits`、`/api/surveys/{id}/responses` 在带 `?page=1&page_size=20` 时返回 `400 invalid_query`；不带参数均 200；对照 `/api/surveys/{id}/versions`（直接提取 `Pagination`）带参仍 200 |
| B 撤销后迟到写入 | 复现，P2 | 迟到请求在正文前已认证；logout 204、`/api/auth/me` 401 之后发送正文，`PUT /api/surveys/{id}/draft` 仍返回 200，数据库中 `revision` 由 1 变 2、草稿标题被改写为「注销后迟到的修改」 |
| C 题目未知字段被丢弃 | 复现，P2 | 文本题 `max_lengthh`、单选题 `shuffle` 被静默丢弃（201）；数字题把 `min` 拼成 `minimum` 后落库为 `min: null`，随后提交越界值 3 返回 201。对照：`Choice`、`Answer`、`Document` 的多余字段、题型名拼错、`required` 拼错都返回 `400 invalid_json` |
| D 提前拒绝缺少安全头 | 复现，P3 | `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff` 出现在 200（健康检查）、400（公开提交参数错误）、401、404 上；CSRF 403 与限流 429 两个响应都没有这两个头 |

分页问题的粒度已进一步确认：扁平字段中的 `title`（字符串）与 `status`（枚举）正常，只有 `page`、`page_size` 这类数值字段失败，`limit=1` 这种未知参数被静默忽略。受影响的四个接口同时在 OpenAPI（`params(("page"=Option<u64>,Query), ...)`）和 `docs/backend-guide.md` 第 380 行声明的通用分页契约之内，前端 `DashboardView.vue`、`AdminView.vue`、`ReportsView.vue` 固定传入这两项，因此这些列表在真实浏览器里必然命中 400。

### 9.3 与既有记录的关系

`docs/requirements-analysis.md` §11.2 已记录「已禁用管理员的延迟请求体」修复：管理员路径改为持锁后在事务内重验 Session。本次 B 项是同一类问题的残留面：`auth.rs` 的 `authenticate` 文档注释、`submissions.rs`、`admin.rs`、`change_password` 都已遵守该约定，只有 `surveys.rs` 的所有者写路径仍只依赖 `lock_user` 的 `disabled` 检查。§12.3 已说明前端列表/分页未做浏览器闭环，这解释了 A 项为何没在前端联调中暴露。

## 10. 按 skill 规则逐类复查（2026-09-15，同日第二轮）

应要求改为按 rust-skills 的 26 个类别逐条核对源码，而不是只复核第 4 节的候选问题。方法：6 个只读子代理分工读取规则文件与模块全文（启动/边界、认证/安全、领域/serde、问卷、公开提交/报表、管理/实体/DDL），主审逐条复核其结论并在真实实例上复现关键项。**未修改任何业务代码**。

新增的、此前未记录的已复现问题：

| 规则 | 位置 | 结论 |
| --- | --- | --- |
| `num-cast-try-from`（数字词法分支） | `domain.rs:517-529` | 同一数学值的不同 JSON 写法判定相反：裸整数 `9007199254740993` 返回 422，指数写法 `9007199254740993e0` 返回 201 并静默落库为 `9007199254740992.0`；`1000000000000000000000` 被拒而 `1e21` 通过 |
| `own-mutex-interior` | `security.rs:44` | 限流窗口 `Mutex` 中毒后每次请求都 500，未用 `PoisonError::into_inner` 恢复 |
| `err-context-chain` | `config.rs:65-67`、`lib.rs` 多处 | `SESSION_TTL_SECONDS=abc` 实测只输出 `Error: invalid digit found in string`，不含变量名（对照 `COOKIE_SECURE` 有语境） |
| `pat-exhaustive-enum` | `surveys.rs:924-935` | 首个匹配臂把 `(Archive, Archived)` 覆盖为幂等成功，使紧随其后的 `survey_archived` 409 分支对该动作不可达；`action` 维度用 `_` 通配，新增动作不会被编译器拦住 |
| 响应头单一来源 | `reports.rs:452-460` | 导出接口重复插入 `no-store`/`nosniff`，与 `security::guard` 重复维护同一策略 |
| 认证时间侧信道 | `auth.rs:167-168` | 用户名不存在时直接返回，不执行 Argon2；与同文件注释「不区分用户名不存在、密码错误或账号已禁用」不一致（限流 60 次/分钟降低了可利用性） |
| 会话回收 | `auth.rs:199/294/479`、`admin.rs:138`、`schema.sql:15` | 没有任何语句按 `expires_at` 清理过期会话，`sessions_expiry` 索引不被任何查询使用 |
| `type-no-stringly` | `admin.rs:34-38/173-184`、`repository.rs:69-71`、`surveys.rs:764/788/1013` | 角色判权用字符串字面量；管理员视图自实现「到期即 closed」并声明 `status: String`；持久化状态字面量绕过 `SurveyStatus` |
| 管理列表 | `admin.rs:92-99/214-220/174` | `count` 与分页无快照事务；排序无匹配索引（`surveys_owner_created` 首列是 owner）；逐行整份 `Document` 解析只为取标题，一行异常即整页 500 |

已实测判定为非问题（不再记录为风险）：启动失败的 anyhow 错误链**不含** DATABASE_URL 口令（`grep` 计数 0）；无 `unsafe`、无 `unwrap/expect/panic`（唯一 `expect` 在 `reports.rs:41` 的编译期常量）；14 项类别（own/err/mem/async/conc/num/serde/pat/coll/perf/proj/lint/anti/name）逐条核对为合规，详见本轮报告。

## 11. 修复记录（2026-09-24）

分支 `feat/frontend-mvp`，按 rust-skills 相关规则修复：

| 项 | 规则 | 修复 |
| --- | --- | --- |
| A 显式分页 400 | `serde-custom-with` | `http.rs` 的 `Pagination` 两个字段改用 `optional_u64`，同时接受数字和纯十进制字符串；`flatten` 缓冲后的字符串也能解析，`+1`、`abc` 仍为 `400 invalid_query`，范围仍由 `bounds()` 返回 422 |
| B 撤销后迟到写入 | — | 新增 `repository::lock_session_user`（锁账号 → 同事务重验原 Session → 核对身份）；`lock_owned_survey` 与 `create_survey` 统一经过它，`submissions.rs` 的同类逻辑改为复用 |
| C 题目未知字段 | `serde-deny-unknown-fields`、`serde-flatten` | `Question` 反序列化不再用 `flatten`：取出公共字段后交给带 `deny_unknown_fields` 的 `QuestionKind`；新增领域测试 `misspelled_question_fields_are_rejected_instead_of_dropped` |
| D 提前拒绝缺安全头 | — | `security::guard` 拆出 `check`，拒绝结果先转成响应再统一补头；删除 `reports.rs` 导出接口的重复头 |
| 限流锁中毒 | `own-mutex-interior` | `lock().unwrap_or_else(PoisonError::into_inner)` |
| 配置错误无变量名 | `err-context-chain` | 三个数值变量解析失败时报出变量名 |

验证：`cargo fmt --check`、`cargo test`（8 个领域测试）、`cargo clippy -D warnings` 通过；`target\backend-review-20260915-0135\verify_fix.py` 在临时 PostgreSQL 18.1 上 21 项断言全部通过，包括迟到写入返回 `401 unauthorized` 且库中 revision 未变，以及 403/429 响应均带 `no-store`/`nosniff`。

尚未处理（需另行决定或属于较大改动）：数字词法分支（与 `backend-guide.md` 4.2 节的现行约定冲突，需确认契约）、认证时间侧信道、过期会话回收、`type-no-stringly` 字符串状态/角色、管理列表快照与索引、`change_status` 的通配匹配。
