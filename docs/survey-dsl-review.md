# feat/survey-dsl 分支评审（相对 main）

> 本文第 1—7 节保留最初静态评审的原始判断和当时行号，不代表当前缺陷清单。2026-10-07 已经执行代码核查与输入契约修复；以第 8 节的复核处置及[验证记录](requirements-analysis.md#19-dsl-输入契约修复与局部去重2026-10-07)为准。

- 评审日期：2026-10-07
- 评审对象：分支 `feat/survey-dsl` 相对 `main` 的全部改动（52 个跟踪文件，+12701 / −72；另有未跟踪的 `frontend/pnpm-lock.yaml`）
- 评审方式：静态评审（`git diff main...HEAD` + 逐行阅读新增模块）。**未运行** cargo / pnpm 任何构建或测试命令，构建与测试结果由提交者提供
- 评审重点：代码风格与可维护性（可读性、重复实现、命名、常量与语义的单一真源、文档漂移），不重新判定功能正确性

## 1. 结论摘要

结构、错误处理与文档纪律良好：新增 2840 行后端代码中没有 `unwrap()`、`unsafe`、`#[allow(...)]`、`panic!`/`todo!`，也没有数字 `as` 转换；每个文件都有 `//!` 不变量声明，`LogicError` 采用稳定错误码加 `&'static str` 文案，`# 错误` / `# Errors` 段齐全。

可维护性债务集中在三处：

1. **同一份 AST 结构知识在 3 个遍历点重复**，新增表达式变体时会静默漏改；
2. **Rust 与 TypeScript 双实现之间没有任何共享代码、常量模块或生成物**，19 项跨语言常量只靠注释互指；
3. **仓库级文档与锁文件未跟上分支**（`AGENTS.md`、`frontend/pnpm-lock.yaml`）。

按严重度统计：后端 12 项、跨端 8 项、前端 13 项、仓库与流程 5 项；其中 2 项标注 `[INFERENCE]`（静态不可确证，见第 6 节）。

## 2. 后端 Rust

| 严重 | 位置 | 问题 | 修复 |
|---|---|---|---|
| 中高 | `logic.rs:455`、`logic_document.rs:442`、`flow.rs:416` | 子节点键集合 3 处硬编码：预算遍历 `["expression","left","right","fallback","value"]`、身份重写 `[…,"value","fallback"]`、缺失分析默认臂 `[…,"value"]`。新增一个带子字段的 `Expression` 变体时三处同时漏掉：深度预算被绕过（深层 JSON 递归反序列化与析构爆栈）、复制文档遗留旧 UUID、分支缺失分析漏报 | 抽单一 `children(&Value) -> impl Iterator<Item = &Value>` 供三处复用；或改为在已反序列化的 `Expression` 上遍历，由编译器穷举兜底。规则：`pat-exhaustive-enum`、`type-no-stringly` |
| 中 | `flow.rs:398-405`、`flow.rs:416` | 保守缺失分析里唯一的两个 fail-open 出口：未知 `op` 落入 `_ =>` 通用臂；未知变量 ID 经 `unwrap_or_default()` 返回空集合（等于不要求任何读取，支配集检查被放宽） | 未知 `op` / 未知变量改为显式失败或 `debug_assert`：不可达即 bug，不应给默认值。规则：`err-expect-bugs-only`、`pat-exhaustive-enum` |
| 中 | `flow.rs:362-366` | 常量分支判定读原始 JSON（`branch.when["op"] == "boolean"`、`branch.when["value"]`），同文件其余位置都使用编译结果。`all([true])`、`coalesce(true,false)` 等恒真常量不会触发“常量分支只能放最后” | 在 `CompiledExpression` 暴露常量折叠结果（如 `const_value() -> Option<LogicValue>`），用编译产物判定。规则：`type-no-stringly` |
| 中 | `logic_document.rs:404` 与 `flow.rs:355-357` | 错误路径重定位两套实现：`relocate()` 与 flow 内联 `failure.path.replacen("$.expression", &path, 1)`；字符串前缀替换只在字面量出现一次时正确 | `relocate` 提升为 `logic.rs` 的 `pub(crate)` 并统一调用；更彻底的做法是在编译期透传 path 前缀。规则：`proj-pub-crate-internal` |
| 中低 | `logic.rs:231,246,248`、`flow.rs:424` | 布尔位承担语义：`Binding.variable`、`Selected(Uuid, Vec<Uuid>, bool)`、`Logical(Vec<Node>, bool)`、`presence_guards(raw, truth: bool)`，调用点看不出第三参是 any 还是 all | `enum Match { Any, All }`、`enum Kind { Question, Variable }`、`enum Polarity`。规则：`type-enum-states` |
| 中低 | `logic.rs:501-521,527-546`；`logic_text.rs:285,349` | 先算 `answered` / `variable` / `all` / `multiple` 四个前置 `matches!`，再在合并臂（`Answer \| Answered \| Variable`、`ContainsAny \| ContainsAll`）内靠这些变量区分；文本侧先 `match op` 再在臂内 `if op == "selected"` 二次判断 | 拆成显式分支臂，重复逻辑用闭包。规则：`pat-exhaustive-enum` |
| 中低 | `domain.rs:544-547` 与 `domain.rs:552-572` | 答卷规范化双路径：有 `logic` 走 `FlowEvaluator`，无 `logic` 走旧循环，两套需长期同步。`flow.rs:186` 已有 `CompiledFlow::implicit` 单页流程，本可只保留一条路径 | 无 `logic` 文档也走隐式流程并删除旧循环；需先做行为等价验证 |
| 低 | `domain.rs:544`（调用方 `submissions.rs:267`）；`reports.rs:594,660` | 每个写请求与每个报表请求都重新编译整个文档（≤100 变量、100 规则、4096 节点），且经 `json!` 包装再反序列化（`logic_document.rs:250`、`flow.rs:353`），无缓存 | 先按 `perf-profile-first` 压测确认是否为热点；确认后再按 `version_id` 缓存编译结果 |
| 低 | `logic_document.rs:218` 与 `logic.rs:348` | 同一表达式的预算遍历执行两遍；字节数用 `expression.to_string().len()` 分配整串 | 预算结果传入编译入口；改用 `serde_json::to_writer` 计数写入。规则：`mem-write-over-format` |
| 低 | `flow.rs:204`、`flow.rs:186-197` | `Uuid::nil()` 作为“分支无身份”的哨兵，而 `compile_flow` 又拒绝 nil 节点 ID；隐式流程用 nil 与 `from_u128(1)` 造虚拟身份 | 改用 `Option<Uuid>`。规则：`type-option-nullable` |
| 低 | `logic.rs:362`、`logic_text.rs:585-586`、`flow.rs:539-540` | 版本号 `1` 三处字面量 | 定义 `const DSL_VERSION` 单点复用 |
| 低 | `logic_document.rs:201,221` | `>100 \|\| >100`、`>4096 \|\| >262_144` 为内联魔法数（对比 `logic.rs:16-17` 已具名） | 提为具名常量 |

## 3. 跨端重复（Rust ↔ TypeScript）

两端各自完整实现同一套 DSL：Rust 是运行时与持久化权威（HTTP 只接受 AST，提交时从入口重建路径），TypeScript 负责编辑、文本格式化与预览填写，其结果被服务端重新编译校验。共享的只有 `fixtures/survey-logic/*.json`，没有任何共享代码、跨语言常量模块或生成物。

| 严重 | 位置 | 问题 |
|---|---|---|
| 高 | `logic.rs:478` + `domain.rs:705` 与 `logic.ts:139,142`、`answers.ts:5`、`flow-inputs.ts:30` | 数字规则 6 份拷贝、两套长度计法（原始字面量长度 / `Number::as_str()` 长度 / 先补 `+` 再判）。`1e+20` 一类字面量在 64 字节边界两侧是否同判，取决于 serde_json `arbitrary_precision` 是否保留 `+`，静态不可确证，见第 6 节 |
| 高 | `domain.rs:637` 与 `flow.ts:341`、`answers.ts:12` | 空白与空答案三套判定：Rust `trim()`（Unicode White_Space，含 U+0085）、TS `\p{White_Space}`（注释自认与 JS trim 不同）、`String.trim()`（不含 U+0085）。只含 U+0085 的文本在表单算已答、在引擎与服务端算空 |
| 高 | `logic.rs:75-95` 与 `logic.ts:33-52` | 18 个错误码一致，但用户可见文案已分叉 6 条（`invalid_flow`、`unavailable_dependency`、`invalid_event`、`required_answer`、`unknown_condition`、`screened_out`）。前端展示 TS 文案、服务端返回 Rust 文案，同一错误会出现两种说法 |
| 中 | 19 项跨语言常量 | 256 / 16 / 100 / 50 / 5000 / 64 / 4096 / 262144 / 1 MiB / 65536 / 64 / 64 等两端各写一份，仅靠注释互指（`answers.ts:4,6`、`domain.rs:706`），没有编译期或测试期保护 |
| 中 | `logic_document.rs:220` 与 `document-logic.ts:137,396-398` | 预算字节数由两套序列化器各自计算（`to_string().len()` 与 `TextEncoder(JSON.stringify)`），非 ASCII 与数字转义差异会让边界差 1 |
| 中 | `logic-text.ts:386-473` | 文本格式化仅前端实现；Rust 解析器是该文本方言的第二个消费者，却只被前端自往返覆盖（`frontend/tests/logic-text.test.mjs:44-48`）。建议把格式化输出写入 `texts.json`，Rust 侧断言可解析 |
| 中 | `logic_text.rs` 全文 | `parse_logic_text` 无生产调用方（仅 `backend/tests/logic_text.rs`）。600 行解析器与 fixture 目前只服务一致性校验，需明确其定位：接真实入口，或删除 |
| 低 | 测试内双份期望值 | `backend/tests/logic.rs:52-70` 与 `frontend/tests/logic.test.mjs:31-44` 的边界元组逐字重复；`backend/tests/logic.rs:151-166` 与 `frontend/tests/document-logic.test.mjs:57-72` 重复；`frontend/tests/answers.test.mjs` 与 `backend/tests/domain.rs:497-517` 各写数字边界。违反 `docs/survey-dsl-plan.md:110-113` 的 fixture 唯一真源约定 |

## 4. 前端 TS / Vue

| 严重 | 位置 | 问题 | 修复 |
|---|---|---|---|
| 中 | `logic.ts:126`、`document-logic.ts:72`、`flow-compiler.ts:53`、`logic-text.ts:257` | UUID 正则 4 份、nil UUID 两种写法；而 `flow.ts:311` 又接受 32 位简单形式与大小写，形成同一仓库两套 UUID 接受集 | 抽 `requireUuid`，统一 nil 判定与大小写策略 |
| 中 | `logic.ts:139`、`flow-inputs.ts:30`、`answers.ts:67` | 数字正则与 64 上限三份，未复用 `answers.ts:5` 已有具名常量 | 共享模块导出 `MAX_NUMBER_JSON_BYTES` 与 `NUMBER_JSON_PATTERN` |
| 中 | `editor-logic.ts:25-46` 与 `flow-compiler.ts:102-151` | 流程拓扑校验双实现（入口首页、空页、归属顺序、前向出口、可达性各写一遍） | 编译错误携带节点索引，编辑期只做文案与 DOM 定位 |
| 中 | `documents.ts:27,33` | `duplicateQuestion(question, document?)` 的 `document` 为可选参数，漏传时静默跳过规则与页面归属复制 | 改为必传（唯一调用点 `EditorView.vue:339` 已传 `draft`），删除 `if (document)` |
| 低 | `logic.ts:133`（未导出）、`logic-text.ts:129`、`flow.ts:335` | `validText` 未导出，同一 Unicode 标量正则内联 3 处 | 导出并复用 |
| 低 | `api.ts:4` 与 `document-logic.ts:3`、`documents.ts:2`、`flow-inputs.ts:5`、`flow.ts:8`、`EditorView.vue:5`、`FillView.vue:6` | `copy` / `decode` / `encode` 存在双导入入口，扩展名混用（`./json.ts`、`../json`、`../api`） | 二选一：删再导出，或统一走单一入口 |
| 低 | `flow-inputs.ts:49` 与 `flow.ts:112,151` | 排序约定两套：`localeCompare`（依赖 locale）与默认码元序 | 统一默认 `.sort()` 并注释排序目的 |
| 低 | `LogicEditor.vue:249-254`、`:383-389` | 模板内联多语句 `@click`，与同文件 `addVariable` / `addNode` 已提取函数不一致 | 抽 `addRule()` 与 `addBranch(node)` |
| 低 | `flow-compiler.ts:64` 与 `LogicEditor.vue:416`；`logic.ts:155` 与 `ExpressionEditor.vue:255,295` | 限额字面量在编译器与组件各写一份 | 导出 `LOGIC_LIMITS` 常量模块 |
| 低 | `editor-logic.ts` | 唯一无测试的新纯 TS 模块，且未进 `test:regression` 清单；`:58-60` 用正则反解 `LogicError.path` 定位 DOM | 新增 `tests/editor-logic.test.mjs`；编译错误改为结构化 `section/index` |
| 低 | `FillView.vue:81,227` 与 `FlowFields.vue:132` | `emit('submit', answers)` 的载荷被丢弃，`submit()` 再经 `expose` 重算一次 | 改为 `submit(flowAnswers?: Answer[])` |
| 低 | `ExpressionEditor.vue:260`、`:188,194` | `node[key!]` 非空断言与 `as string[]`，源于 `children` 推断为 `string[][]` | 将 `children` 声明为类型化 `computed` |
| 低 | `document-logic.ts:190` | 空调用触发版本检查且无注释（Rust 侧 `logic_document.rs:252` 有中文注释） | 补注释或把版本检查显式化 |

## 5. 仓库与流程

| 严重 | 位置 | 问题 |
|---|---|---|
| 中 | `frontend/pnpm-lock.yaml` | 未纳入版本控制（`git status` 为 `??`，`.gitignore` 也未忽略），同时删除了 `package-lock.json`：全新克隆无法执行 `pnpm install --frozen-lockfile`。必须 `git add` |
| 中 | `AGENTS.md`（工作区 diff 仅 +5 / −1） | 新模块与测试目标未同步：模块表仍无 `flow` / `logic` / `logic_document` / `logic_text`；“`backend/tests/` — `domain.rs` is currently the entire suite”与“one integration target”已过期（README 已更新，AGENTS.md 未更新） |
| 低 | `frontend/package.json:11` | `test:regression` 逐文件枚举测试清单，新增用例必须同步改脚本 |
| 低 | 测试函数名 | 新文件全中文（`tests/logic.rs`、`flow.rs`、`logic_text.rs`），`tests/domain.rs` 大多英文、个别中文；仓库未规定，建议在 AGENTS.md 明确一条 |
| 低 | `domain.rs:14-25` | 转发 4 个子系统后 `domain` 从“纯领域模块”变为门面，且 `flow.rs:12` / `logic.rs:11` 反向 `use crate::domain` 形成模块环，`reports.rs:32` 经 `domain::FlowEvaluator` 间接依赖 flow。可接受，但建议或在 `lib.rs` 直接 `pub mod logic; pub mod flow;`，或在本文件记录“domain 是对外契约门面” |

## 6. 不确定性与验证边界

- 本次未运行构建、测试、格式或 Clippy，所有结论来自静态阅读；`file:line` 均对应当前工作区内容。
- `[INFERENCE]` 项一：`1e+20` 这类无符号正指数在 64 字节边界两侧是否同判，取决于 serde_json `arbitrary_precision` 保留的原文是否含 `+`（`domain.rs:706` 的前端注释假定需要补 `+`）。建议补一条 fixture 用例直接锁定该边界。
- `[INFERENCE]` 项二：TS 侧对文档 JSON 的 UUID 接受集（`document-logic.ts:69-74` 只接受小写连字符形式）与 Rust `Uuid` 反序列化是否一致，未运行验证。
- 未评估的功能性风险不在本次范围：真实浏览器与 PostgreSQL 的联调结论以 `docs/survey-dsl-plan.md` 的阶段 5、6 记录为准。

## 7. 建议处理顺序

1. **P0（零风险）**：`git add frontend/pnpm-lock.yaml`；同步 `AGENTS.md` 的模块表与测试章节。
2. **P0 收敛三处 AST 键集合**（第 2 节第 1 项）：一个 `children()` 助手加一致性断言，消除最危险的静默漏改点。
3. **P1 跨端共享表入 fixture**（第 3 节）：错误码到文案表放 `fixtures/survey-logic/messages.json`，限额常量放 `limits.json`，并把现有双份硬编码期望值搬回 fixture，落实 `docs/survey-dsl-plan.md:110-113` 的约定。
4. **P1 前端中风险去重**：UUID 助手、数字正则与上限、流程拓扑校验（`editor-logic.ts` 与 `flow-compiler.ts`），以及 `duplicateQuestion` 必传参数。
5. **P2 需要决策或压测**：`parse_logic_text` 的去留；答卷规范化单路径化；编译结果按 `version_id` 缓存（先压测）。

## 8. 复核与处置（2026-10-07）

本轮优先处理复现出的输入契约差异，再做与其直接相关的局部去重；原评审中的等级和建议不全部采纳。

| 原判断 | 复核与本轮处置 |
| --- | --- |
| 普通表单和流程的空白判断不同 | 属实。统一使用 Unicode `White_Space`，修复 U+0085 与 U+FEFF 的必填、省略和进度差异；非空白答案保留原文。 |
| Rust / TS 的 UUID 接受集可能不同 | 已复现。Rust 默认 `Uuid` 反序列化会规范化大写、简单形式和花括号；现在配置字段、答案/事件题目身份严格要求非零、小写连字符 UUID，AST 引用和文本绑定使用同一规则。选项答案值的规范化单独保留。共享样例逐字段覆盖，并拒绝尾随换行。 |
| 数字校验重复、长度边界可能漂移 | 前端数字原文规则已收敛到 `numbers.ts`；Rust 本就共用 `finite_number`，并非六套完整校验。正指数补 `+` 的 64 字节边界实测一致；另补共享回归拒绝数字常量尾随换行。 |
| pnpm 锁未跟踪、AGENTS 测试清单过时 | 属实。完成 pnpm 锁文件纳入版本控制，保留 pnpm 12 生成的两份 YAML 文档；同步模块、独立测试目标与当前无历史答卷兼容前提。冻结安装也在没有 `node_modules` 的临时目录验证。 |
| 小范围重复与不明确的接口 | 已统一前端 UUID/Unicode 标量判断、Rust 错误路径重定位；单题复制必须传入问卷；提交答案排序采用默认码元序；填写页直接消费流程组件发出的答案。 |
| 双端无共享实现属于缺陷、Rust 文本入口应删除 | 不采纳。Rust 与 TypeScript 独立实现、共用语义样例是用户要求。Rust 文本入口是公开领域契约，HTTP 继续接收 AST；没有 HTTP 调用方不代表无效代码。 |
| AST 遍历存在当前可利用的静默放行、应优先统一 | 现有 `required_reads` 只接收已经编译并完成变量拓扑检查的 AST；未证实当前输入能绕过检查。预算、引用重写和必需读取的目的不同，尤其缺失保护和短路语义不同，本轮保留各自遍历，不新增通用访问器。 |
| 常量折叠、JSON 字节数误差、缓存和编辑器测试 | 通用常量折叠不是当前契约；JSON 字节预算相差一字节没有被验证。缓存需要性能证据；不因模块缺测试就扩展测试工程。编辑定位与编译诊断的整体重构留在本轮之外。 |
| 测试中文约定缺失 | 不属实，仓库原有约定已要求中文；本轮只是明确新增回归的语言和独立目录要求。 |

本轮没有引入共享生成器、历史格式兼容分支、数据库迁移或新测试依赖。验证范围和结果见[需求与验证记录第 19 节](requirements-analysis.md#19-dsl-输入契约修复与局部去重2026-10-07)；第 1—7 节的“未运行测试”仅描述最初静态评审。
