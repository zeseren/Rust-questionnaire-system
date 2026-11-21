# 问卷 DSL 与流程引擎规范

本文定义动态问卷系统的领域特定语言（DSL）与页面流程引擎：包括单一 Rust 真源架构、表达式类型与三值逻辑、计算变量与题目规则、前向无环页面流程（DAG）状态机、服务端路径重建以及文本 DSL 语法契约。

---

## 1. 单一真源架构与 WASM 边界

规则、流程与文本语法的编译和求值逻辑仅在 `logic-core/`（`questionnaire-logic`）中实现一份，前后端共享完全一致的领域语义：

```text
logic-core/src/
├── domain.rs           问卷文档模型、题型与答案校验、公共 API 出口
├── identity.rs         36 字节非零小写连字符 UUID 严格解析边界
├── logic.rs            表达式 AST、静态类型推导、三值逻辑求值与单表达式预算
├── logic_document.rs   CompilationContext、计算变量拓扑排序、题目规则与 ID 重写
├── flow.rs             前向页面流程编译、支配节点合流分析、FlowSession 与 FlowEvaluator
└── logic_text.rs/.pest 词法预检、pest Pratt 优先级解析、别名绑定与确定性格式化
```

- **后端接入**：`backend/src/lib.rs` 通过 `pub use questionnaire_logic::domain;` 直接调用原生 Rust 函数。
- **前端接入**：`logic-wasm/` 使用泛型 `Envelope<T>` 将结果单遍序列化为 JSON 字符串信封 `{"ok": …}` 或 `{"error": …}`，编译为 `frontend/src/wasm/pkg/` 入库产物，由 `frontend/src/wasm/index.ts` 封装为 `parseLogicText`、`inspectLogicText`、`formatLogicText`、`compileDocumentLogic` 与 `FlowSession`。
- **双端契约回归**：`fixtures/survey-logic/` 下的 6 份 JSON 样例（`expressions.json`、`documents.json`、`flows.json`、`submissions.json`、`texts.json`、`input-contracts.json`）由 `cargo test --locked --workspace` 全部读取核对；前端 `pnpm run test:regression` 的契约层读取其中 `flows.json`、`input-contracts.json` 与 `texts.json` 三份。

---

## 2. 表达式语言与三值逻辑

### 2.1 版本与静态类型

表达式包结构为：

```json
{ "dsl_version": 1, "semantics_version": 1, "expression": { "op": "boolean", "value": true } }
```

- `dsl_version` 与 `semantics_version` 仅接受整数 `1`。
- 编译期静态推导 5 种类型：`boolean`、`number`、`text`、`choice`（单选选项 UUID）、`choices`（多选选项 UUID 集合）。
- 运行时值在此基础上增加 `missing`（表示未回答、显式 `null` 或被流程/显隐规则跳过）。布尔上下文中的 `missing` 即为三值逻辑的 **unknown**。

### 2.2 AST 节点

节点以内部标签 `op` 区分，拒绝任何未知字段（`deny_unknown_fields`）：

| `op` | 字段 | 静态类型与语义 |
| --- | --- | --- |
| `boolean` / `number` / `text` | `value` | 布尔字面量、保留原文的有限数字字符串、文本常量 |
| `answer` / `answered` | `question_id` | 读取题目答案（返回对应题型或 `missing`）/ 判断题目是否已有效作答（返回 `boolean`） |
| `variable` | `variable_id` | 读取同一文档内声明的计算变量 |
| `option` | `option_id` | 选项常量（`choice`）；`option_id` 必须属于某道单选或多选题，并记录选项所属题目 |
| `selected` | `question_id`, `option_id` | 判断单选或多选题是否选中指定合法选项 |
| `contains_any` / `contains_all` | `question_id`, `option_ids` | 判断多选题是否包含非空无重复选项集合中的任一 / 全部选项 |
| `not` | `expression` | 布尔取反（`!unknown -> unknown`） |
| `all` / `any` | `expressions` | 非空布尔列表，从左向右短路求值 |
| `compare` | `comparison`, `left`, `right` | `eq` / `ne` 支持同类型 `boolean`、`number`、`text`；`lt` / `le` / `gt` / `ge` 仅支持 `number`；两侧 `choice` 的 `eq` / `ne` 需属于同一道题 |
| `coalesce` | `value`, `fallback` | 同类型空值合并：仅当 `value` 为 `missing` 时求值并返回 `fallback` |
| `length` | `value` | 返回 `text` 的 Unicode 标量个数或 `choices` 的选中项数量（`number`） |
| `matches` | `text`, `pattern` | 文本整串正则匹配（`boolean`）；`pattern` 是字面字符串，编译期以 `^(?:pattern)$` 预编译 |
| `arithmetic` | `operator`, `left`, `right` | 数字的 `add`、`subtract`、`multiply`、`divide`（除零或非有限结果报运行错误） |
| `round` / `floor` / `ceil` | `value`, `digits` | 数字按 `10^digits` 缩放后取整再缩放回来；`digits` 为 `0..=6` 的 JSON 整数 |

### 2.3 三值逻辑与求值规则

| 运算 | 结果 |
| --- | --- |
| `not unknown` | `unknown` |
| `false && unknown` | `false` |
| `true && unknown` | `unknown` |
| `true \|\| unknown` | `true` |
| `false \|\| unknown` | `unknown` |

- **缺失传播与消除**：算术、比较、`length`、`selected`、`contains_*`、`matches`、`round` / `floor` / `ceil` 遇到 `missing` 操作数时返回 `missing`；仅 `answered(q)` 与 `coalesce(val, fallback)` 可显式消除 `missing`。
- **选项比较域**：`choice` 类型节点携带所属题目（`answer(q)` 取其题目，`option` 字面量取其选项所属题目，`if` / `coalesce` 两支同题时继承、否则未知，`variable` 引用未知）。`compare` 的 `eq` / `ne` 两侧都是 `choice` 且所属题目均已知但不同时报 `type_mismatch`。
- **整串正则**：`matches(text, pattern)` 的 `pattern` 先按原文单独编译校验，再加锚 `^(?:pattern)$` 编译用于求值，防止 `a)|(b` 这类不平衡模式被包裹后“合法化”；最长 256 个字符、`RegexBuilder` 设 `size_limit`（1 MiB）；超长报 `limit_exceeded`，非法正则报 `invalid_syntax`。`(?x)` 模式下以 `#` 注释结尾会吞掉锚点，应避免。`text` 缺失时结果为 `missing`。
- **数字取整**：`round` 远离零、`floor` 向下、`ceil` 向上；按 `10^digits` 缩放后取整再除回，`digits` 必填且为 `0..=6` 的 JSON 整数，越界或非整数报 `invalid_syntax`。已落在目标精度上的值（如 `floor(0.29, 2)`）原样返回，不受缩放误差影响；`round` 按二进制值判断进位（`round(1.005, 2) = 1`）。缩放溢出报 `arithmetic_error`，结果统一做 `canonical_zero`。
- **运行错误不降级**：除零、算术溢出或非法上下文值立即返回结构化 `LogicError`，绝不静默降级为 `missing`。
- **单表达式预算**：最多 `256` 个节点、最大嵌套深度 `16`、数字字面量原文最多 `64` 字节、文本字面量最多 `5000` 个 Unicode 标量、选择集合最多 `50` 项。

---

## 3. 文档规则与计算变量

`Document.logic` 结构包含 `dsl_version`、`semantics_version`、`variables`（最多 100 个）、`rules`（最多 100 组）、可选的 `flow` 以及可选的 `aliases`：

- `aliases` 是文本 DSL 的展示别名映射（`{ "<uuid>": "alias" }`），键为题目、选项、变量或流程节点 ID；空映射不参与序列化。
- 编译器只校验别名本身：合法标识符（ASCII 字母或 `_` 开头，后续为字母、数字或 `_`）、不是保留字、各值互不重复，违例报 `invalid_reference`（路径 `$.logic.aliases`）。已删除实体的别名不校验，由格式化器忽略。
- 复制问卷时 `renew_ids` 同步改写题目、选项、变量与流程节点身份，并据旧到新映射重写 `aliases` 的键；映射不到的条目丢弃。

1. **计算变量与确定性拓扑排序**：
   - 每个变量声明 `id`、`value_type` 与 `expression`。
   - 编译器构建传递依赖图，拒绝循环依赖（`DependencyCycle`）；同时就绪的变量按 UUID 字典序从小到大排列，确保求值顺序完全确定。
2. **前向引用约束（`ForwardReference`）**：
   - 题目的 `visible_if`（显示条件）与 `required_if`（动态必填）只能直接或经由变量间接引用**题面顺序严格在前**的题目。
   - 题目的 `validate`（答案约束）可额外引用**当前题自身**，但同样禁止引用后续题目。
3. **规则执行语义**：
   - `visible_if` 求值为 `true` 时显示，`false` 或 `unknown` 时隐藏；隐藏题不要求必填。
   - 实际必填性为 `question.required || required_if`；`required_if` 求值为 `unknown` 时视为「不必填」，不再阻断继续。`validate` 与流程 `branch` 的 `unknown` 行为保持不变。
   - 可选题未作答时不执行 `validate`；已作答题目先在含自身答案的临时上下文中执行 `validate`，通过后才向后续规则与变量公开该答案。

---

## 4. 页面流程（DAG）与会话状态机

### 4.1 节点与拓扑约束

`flow` 由入口节点 ID `entry` 与按拓扑顺序排列的 `nodes`（最多 256 个）组成：

| 节点类型 | 结构字段 | 语义 |
| --- | --- | --- |
| `page` | `id`, `question_ids`, `next` | 页面节点：展示本页内可见题目，当前页无校验错误时按 `next` 推进 |
| `branch` | `id`, `branches` (`1..=16` 条 `{when, target}`), `otherwise` | 条件分支：按顺序匹配第一条求值为 `true` 的 `when`；遇到 `unknown` 报错停止；全为 `false` 走 `otherwise` |
| `end` | `id`, `outcome` (`complete` \| `screen_out`) | 结束节点：`complete` 表示可提交；含 `screen_out` 的草稿仅支持预览试填，禁止发布 |

- **前向无环图（DAG）**：`entry` 必须是首个节点且为 `page`；所有跳转边（`next`、`target`、`otherwise`）只能指向数组中下标更大的后续节点。
- **题目全覆盖且保序**：所有 `page` 节点的 `question_ids` 按节点顺序拼接后，必须恰好等于 `Document.questions` 的题目 ID 序列（不重、不漏、不乱序）。
- **合流支配检查（Dominator Check）**：编译器计算每个节点的必经前置节点（dominators）。若某表达式引用的题目并非当前位置的必经前置页题目，则必须使用 `answered(q)`、`coalesce(...)` 或从左向右的布尔短路守卫（如 `all(answered(q), ...)`）显式保护，否则编译期报 `UnavailableDependency`。

### 4.2 会话事件与服务端路径重建

- **交互式会话（`FlowSession`）**：
  - `change(question_id, value)`：仅允许修改当前页可见题；就地更新输入后从入口重算有效路径，自动清理不再适用分支上的输入并记录在快照 `cleared` 中。
  - `next`：校验当前页可见题的必填与 `validate` 约束，通过后沿有效边推进至下一非空可见页或 `end` 节点，并记录访问历史 `history`。
  - `previous`：沿 `history` 回退至上一实际访问页面。
- **服务端提交重建（`FlowEvaluator`）**：
  - 服务端不信任客户端上传的路径或历史，仅接收 `{ version_id, idempotency_key, answers }`。
  - 从入口按历史版本规则逐步推进：拒绝任何隐藏题或未途经分支题目的答案（即使为 `null`），仅将已校验通过的路径内答案纳入上下文，最终生成规范化 `answers` 与覆盖全卷题目的三态表 `question_states`（`answered` / `unanswered` / `skipped`）。

---

## 5. 文本 DSL 语法契约与确定性格式化

文本 DSL 与可视化配置共享同一份 `DocumentLogic` AST，文法定义于 `logic-core/src/logic_text.pest`（上限 `1 MiB`、最大解析嵌套深度 `64`）。表达式位置可直接裸写绑定别名：变量别名解析为 `variable` 节点，选项别名解析为 `option` 节点；题目、流程节点等其它类别裸写报 `invalid_reference`，题目需写成 `answer(q)` / `answered(q)`。

```text
dsl 1 semantics 1;
bind question q1 = "01928f10-0000-4000-8000-000000000001";
bind question q2 = "01928f10-0000-4000-8000-000000000002";
bind option q2_o1 = "01928f10-0000-4000-8000-000000000021";
bind variable v1 = "01928f10-0000-4000-8000-000000002001";
bind node n1 = "01928f10-0000-4000-8000-000000003001";
bind node finish = "01928f10-0000-4000-8000-000000003002";

// 文本长度足够且为纯数字时展开单选题追问
let v1: number = round(length(answer(q1)) * number("0.1"), 2);
visible q2 = v1 > number("3") && matches(answer(q1), "^[0-9]+$");
required q2 = v1 > number("3");
validate q2 = answer(q2) == q2_o1;

entry n1;
page n1 [q1, q2] -> finish;
end finish = complete;
```

- **绑定与声明顺序**：所有 `bind <question|option|variable|node> <alias> = "<uuid>";` 位于文件头部，随后声明 `let`、`visible`、`required`、`validate` 以及 `entry`、`page`、`branch`、`end`。
- **别名与保留字**：别名必须是合法标识符（ASCII 字母或 `_` 开头，后续为字母、数字或 `_`），不能与保留字重名：`dsl`、`semantics`、`bind`、`question`、`option`、`variable`、`node`、`let`、`visible`、`required`、`validate`、`entry`、`page`、`branch`、`when`、`if`、`then`、`else`、`end`、`complete`、`screen_out`、`true`、`false`、`boolean`、`number`、`text`、`choice`、`choices`、`answer`、`answered`、`selected`、`contains_any`、`contains_all`、`contains`、`starts_with`、`matches`、`coalesce`、`length`、`all`、`any`、`round`、`floor`、`ceil`。`parse` 产出的 `aliases` 只保存格式化器无法自行还原的 `bind` 别名：既不是该实体的默认名，也不是默认名让位于其它自定义别名后追加的后缀名，保证 `parse(format(parse(t))) == parse(t)`。
- **运算符优先级（PrattParser，从低到高）**：
  1. `||`
  2. `&&`
  3. `==` `!=` `<` `<=` `>` `>=`
  4. `+` `-`
  5. `*` `/`
  6. 前缀 `!`
- **同运算符链扁平化**：`a && b && c` / `a || b || c` 生成单个 `all` / `any` 节点；左操作数已是同名节点时直接追加右操作数。预算按 `depth = max(左.depth, 右.depth + 1)`、`nodes = 左.nodes + 右.nodes` 逐次检查。
- **条件选择**：三元条件必须显式闭合：`if 条件 then 真值 else 假值 end`，缺少 `end` 报语法错误。
- **内置函数**：`call` 只接受内置函数名。`matches(文本, "正则")` 的第二个参数必须是字符串字面量；`round(x)` / `round(x, 2)`、`floor(...)`、`ceil(...)` 的 `digits` 必须是非负整数字面量（省略即 0），JSON 始终写出 `digits`。变量与选项在表达式位置直接裸写别名。
- **确定性格式化（`format_logic_text`）**：唯一实现在 `logic-core/src/logic_text.rs`。别名优先取 `logic.aliases` 中该 ID 的值，否则按位置生成默认名 `q{n}`、`q{n}_o{m}`、`v{n}`、`n{n}`；默认名与已占用别名冲突时追加 `_2`、`_3`… 直到不冲突（已删除实体的别名忽略）。`variable` / `option` 写裸别名；`if` 写 `if c then a else b end`；`all` / `any` 仍写函数形式；`matches` / `round` / `floor` / `ceil` 写回对应函数，`digits` 为 0 时省略第二参数。保留数字常量原文，严格满足往返等价 `parse(format(parse(t))) == parse(t)`。
