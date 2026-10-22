# 动态问卷调查系统需求与架构规格说明

本文档定义「问序（Wenxu）」动态问卷调查系统的完整需求规格、技术架构、业务规则、数据与隐私边界、非功能性指标及验收标准。

快速启动与部署指南见 [README](../README.md)，代码导读见 [后端开发指南](backend-guide.md) 与 [前端开发指南](frontend-guide.md)，规则与流程语义见 [DSL 与流程引擎规范](survey-dsl-plan.md)。

---

## 1. 项目定位与核心技术决策

### 1.1 系统目标

面向课程教学反馈、社团活动报名与一般研究调研，构建前后端分离的 Web 动态问卷系统：发布者自主编纂题面与跳题规则、发布不可变版本并管理回收生命周期；受访者通过公开链接在桌面或移动设备上分步作答；发布者查看分版本三态统计并导出安全 CSV；平台管理员治理用户与违规问卷并留存审计轨迹。

### 1.2 核心技术栈与架构决策

| 维度 | 选型与架构决策 |
| --- | --- |
| 后端运行时与框架 | Rust（stable ≥ 1.94，2024 edition）、Axum 0.8、Tokio 1.53；严格三层架构（HTTP 边界 → 业务用例 → 数据访问） |
| 数据库与持久化 | PostgreSQL 18；常规 CRUD 与事务锁使用 SeaORM 2.0，统计快照查询使用共享连接池上的 SQLx 0.9；唯一 DDL 位于 `backend/src/schema.sql` |
| 领域规则引擎 | `logic-core`（`questionnaire-logic`）纯 Rust 单一真源；后端直接引用，前端经 `logic-wasm` JSON 字符串信封调用入库 WASM 产物 |
| 前端工程与状态 | Vue 3.5 + TypeScript 5.9 + Vite 8 + Vue Router + Pinia；包管理器固定为 pnpm（`frontend/pnpm-lock.yaml`） |
| 视觉体系与组件 | **Editorial Craft（纸本与松烟墨）** 视觉规范（宣纸浅色 `#f5f2eb` / 玄墨深色 `#121210`、中文衬线阶梯、等宽 `tabular-nums` 数据、朱砂印泥点睛）；按需引入 Element Plus 搜索选择器与懒加载 CodeMirror 6 文本 DSL 控制台 |
| 认证与安全 | HttpOnly + SameSite=Lax Cookie Session（数据库仅存 SHA-256 摘要）；Argon2id 密码哈希；写请求强制同源 `Origin` + `X-CSRF-Protection: 1`；单实例固定窗口限流 |
| 接口文档 | 基于 `utoipa` 6.0 + `utoipa-axum` 0.3 从 handler 注解单一来源自动生成 `/api/openapi.json` |

---

## 2. 角色权限矩阵

发布者与受访者是同一套普通注册账号在不同业务场景下的身份，不设互斥角色。

| 业务能力 | 未登录访客 | 普通注册用户（`user`） | 平台管理员（`admin`） |
| --- | --- | --- | --- |
| 填写公开匿名问卷（`anonymous`） | 允许（满足回收条件时） | 允许（仍按匿名处理，不关联账号） | 允许（仍按匿名处理） |
| 填写登录限次问卷（`login_once`） | 需先登录 | 允许（每账号跨版本限交一次） | 允许（每账号跨版本限交一次） |
| 创建、编辑、发布和管理问卷 | 禁止 | 仅限本人创建的问卷 | 仅限本人创建的问卷（不越权编辑他人问卷） |
| 查看答卷详情、分版本统计、导出 CSV | 禁止 | 仅限本人创建的问卷 | 仅限本人创建的问卷（治理接口不暴露他人答卷） |
| 禁用/启用账号、下架/恢复问卷、查看审计 | 禁止 | 禁止 | 允许（必填原因，同事务写入审计表） |

---

## 3. 核心功能需求（FR）

| 编号 | 模块 | 功能与业务约束 |
| --- | --- | --- |
| **FR-01** | 账号与认证 | 用户名与密码注册、登录、退出；查看当前账号资料；修改密码后在同一事务内撤销该账号全部设备的已有 Session |
| **FR-02** | 问卷管理 | 创建草稿、分页查询本人问卷（按标题子串与状态筛选）、整卷复制为新草稿（重写全部配置 UUID）；未发布草稿可彻底删除，已发布问卷仅允许归档 |
| **FR-03** | 问卷与规则编纂 | 编辑标题、说明、收集目的、结束语；增删、复制与排序六类题型；通过可视化构建器或文本 DSL 配置计算变量、题目显隐（`visible_if`）、动态必填（`required_if`）、自定义约束（`validate`）与前向页面分支流程（`flow`）；基于 `revision` 乐观锁保存草稿 |
| **FR-04** | 预览与版本发布 | 支持内存实时试填与服务端已保存草稿预览（桌面/手机视口切换）；发布前执行完整结构、规则与流程拓扑校验；发布生成不可变版本快照，同内容重复发布幂等返回 `200 OK` |
| **FR-05** | 回收生命周期 | 首次发布前设定填写模式（`anonymous` 或 `login_once`），发布后模式冻结；设置或调整截止时间（`deadline`）；手动关闭、显式恢复回收、不可撤销归档 |
| **FR-06** | 受访者在线填写 | 展示调查说明、收集目的與隐私可见范围提示；按 WASM `FlowSession` 逐页分步渲染、回退沿历史前进、改答立即清理失效分支输入；抵达终点后展示核对清单 |
| **FR-07** | 幂等与原子提交 | 服务端按历史版本规则从入口重建有效路径，拒绝路径外、重复或越界答案；完整答卷原子入库；基于 UUIDv4 `idempotency_key` 实现同载荷幂等重放与异载荷 `409` 冲突拒绝 |
| **FR-08** | 答卷明细查看 | 发布者按版本与 `[from, to)` 时间区间分页查看答卷列表及单份详情（附原版本题面与每题 `answered` / `unanswered` / `skipped` 状态） |
| **FR-09** | 分版本统计分析 | 按版本与时间区间统计总答卷数、北京时间自然日回收趋势、每题三态计数、选择题选项占比、评分稀疏分布与均值、数字题均值/最值及文本回答列表 |
| **FR-10** | 安全 CSV 导出 | 导出选定版本与时间范围内的 UTF-8 CSV（规则版本附每题状态列，多选存 JSON 数组）；对 `=`、`+`、`-`、`@` 开头字段自动前置 `'` 防护电子表格公式注入 |
| **FR-11** | 平台治理与审计 | 管理员分页查看用户、问卷元数据（及已发布题面）与审计记录；禁用/启用用户或下架/恢复问卷必须填写原因并同事务写入 `audits` 表 |
| **FR-12** | 隐私与知情提示 | 发布前强制要求填写收集目的；填写页明示匿名或登录限次规则；匿名模式绝不解析 Cookie 或记录账号 ID；日志与回执不泄露答案正文或秘密 |

### 3.1 六类题型与容量边界

| 题型 / 字段 | 结构与校验约束 | 容量上限 |
| --- | --- | --- |
| 单选题（`single_choice`） | 至少 2 个选项；有效答案为属于该题的单个选项 UUID | 每题最多 50 个选项；选项文字 ≤ 500 字符 |
| 多选题（`multiple_choice`） | 至少 2 个选项，`1 <= min_choices <= max_choices <= 选项数` | 无重复合法选项 UUID 数组 |
| 单行文本（`text`） | 禁止包含换行符；纯空白视为未回答 | `max_length` 默认且最高 `200` 个 Unicode 标量 |
| 多行文本（`textarea`） | 允许换行；纯空白视为未回答 | `max_length` 默认且最高 `1000` 个 Unicode 标量 |
| 数字题（`number`） | 有限双精度 `f64`，可选 `integer = true` 与闭区间 `[min, max]` | `-0.0` 归一为 `0.0`，`1` 与 `1.0` 等价 |
| 评分题（`rating`） | 整数闭区间 `[min, max]`（`min <= max`），可选两端文字说明 | 端点说明 ≤ 100 字符 |
| 问卷文档级容量 | 标题 ≤ 200，题目标题 ≤ 500，说明/收集目的各 ≤ 5000，结束语 ≤ 1000 | 每卷最多 100 题、100 个变量、100 组规则、256 个流程节点；单次请求体 ≤ `1 MiB` |

---

## 4. 关键业务规则（BR）

- **BR-01（可回收条件）**：新答卷入库必须同时满足：问卷状态为 `collecting`、未到 `deadline`、未归档、`taken_down == false`、发布者账号未禁用、提交 `version_id == current_version_id`。到期后自动视为 `closed`，延后或清空 `deadline` 后仍需显式调用 `resume` 才恢复回收。
- **BR-02（固定锁序与重验）**：所有问卷写事务固定按「先锁发布者 `users`（`FOR SHARE` / `FOR NO KEY UPDATE`）→ 再锁 `surveys`（`FOR UPDATE`）」执行，并在持锁后重新验证当前操作者 Session；公开读题面与各类列表/报表查询统一使用 `RepeatableRead + ReadOnly` 快照，读写互不阻塞。
- **BR-03（治理与业务状态正交）**：管理员解除下架（`taken_down = false`）或启用账号，绝不自动恢复已关闭、已到期或已归档的问卷；发布者恢复回收也无法绕过 `taken_down`。
- **BR-04（版本不可变与复制隔离）**：每次发布生成不可变 `versions` 记录；复制问卷通过 `renew_ids` 生成全新题目、选项、变量与节点 UUID，不继承历史版本、答卷或限次记录。
- **BR-05（幂等重放与跨版本限次）**：
  - 匿名幂等键作用域为 `(survey_id, "anonymous", key_hash)`；登录限次幂等键作用域为 `(survey_id, respondent_id, key_hash)`。
  - 登录限次由数据库部分唯一索引 `responses_once_per_account(survey_id, respondent_id)` 强制保证，同一账号跨所有发布版本最多成功提交一次。
- **BR-06（三态统计恒等式）**：带规则版本按提交所属版本的 `FlowEvaluator` 重建路径，每题状态分为 `answered`、`unanswered`、`skipped`，严格满足 `applicable_count = answered_count + unanswered_count` 与 `total_count = applicable_count + skipped_count`。

---

## 5. 非功能需求（NFR）与验收标准（AC）

1. **安全与协议一致性（NFR-01 ~ NFR-04 / AC-01 ~ AC-02, AC-13）**：
   - 服务端严格执行归属与角色校验；越权访问他人问卷管理接口统一返回 `404 not_found`。
   - 非读请求强制校验 `Origin == PUBLIC_ORIGIN` 与 `X-CSRF-Protection: 1`；所有响应携带 `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff`。
   - 内部 500 错误通过 `#[track_caller]` 记录位置并生成 16 位十六进制 `X-Error-Id`，绝不向外泄露数据库表结构或 SQL 文本。
2. **响应式与可用性（NFR-05 ~ NFR-07 / AC-03 ~ AC-04）**：
   - 采用 Editorial Craft（纸本与松烟墨）视觉体系，支持宣纸浅色、玄墨深色与跟随系统。
   - 受访者填写页在 `375px` 窄屏视口下无横向滚动条（`scrollWidth == clientWidth`）；所有交互控件具备可访问名称与键盘焦点指示。
3. **数据一致性与可恢复性（NFR-10 ~ NFR-12 / AC-05 ~ AC-12）**：
   - 并发提交、关闭、改密、禁用与版本发布在真实 PostgreSQL 18 事务与唯一约束下保持严格一致，无半成品答卷残留。
   - 支持通过 `pg_dump -Fc` 与 `pg_restore --exit-on-error` 备份并恢复至独立空库，恢复后版本关系与统计结果完全一致。

---

## 6. 验证覆盖总结

| 验证层级 | 覆盖范围与执行方式 |
| --- | --- |
| Rust 领域与 WASM 边界测试 | `cargo test --locked --workspace`：`logic-core/src/*.rs` 模块内单元测试、`logic-core/tests/{domain,logic,flow,logic_text}.rs` 领域契约集成测试（与 `fixtures/survey-logic/` 共读人工预期）、`logic-wasm/tests/smoke.rs` WASM JSON 信封边界测试，以及 `backend/src/tests/{config,error_and_http,security_and_router,auth_and_admin,surveys_and_submissions,reports}.rs` 模块化单元与 HTTP 边界测试；无需数据库即可验证题型边界、三值逻辑、变量拓扑、前向流程、文本 DSL、错误信封与路由安全 |
| 前端回归与类型/构建检查 | `pnpm run test:regression` 递归执行 `frontend/tests/{contract,unit,integration}/`：`contract/` 共读 `fixtures/survey-logic/` 并加载真实入库 WASM；`unit/` 覆盖 17 个模块（`value-contract`、`numbers`、`json`、`format`、`logic-errors`、`answers`、`documents`、`condition-builder`、`flow-tools`、`editor-logic`、`survey-templates`、`logic-text-tools`、`api`、`resource`、`stores`、`router`、`wasm-envelope`）；`integration/` 覆盖 `editor-workflow`、`flow-inputs`；共享 `frontend/tests/helpers/{fixtures,documents,browser}.mjs` 夹具缓存、固定身份构造器与浏览器环境桩。另有 `pnpm run build`（`vue-tsc --noEmit` + Vite 生产打包）与 `pnpm run format:check` |
| 真实服务与浏览器端到端验证 | 基于 PostgreSQL 18 实例、Rust API 与 Chromium 浏览器完成注册登录、四工作区编纂、可视化与文本 DSL 切换、草稿预览、分步填写与幂等提交、三态统计与 CSV 导出、管理员治理审计，以及浅色/深色主题与 `375px` 移动视口渲染验证 |
