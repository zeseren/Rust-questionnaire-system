# 问卷 DSL 与流程引擎设计说明

本文说明问卷规则与跳题的完整设计：表达式语言如何表达条件，文档级规则如何组织，页面流程如何在填写过程中推进，以及文本编辑、可视化配置和服务端提交如何共用同一份语义。

代码集中在三个 crate 与一个前端目录：

| 位置 | 职责 |
| --- | --- |
| `logic-core/`（包 `questionnaire-logic`） | DSL 唯一真源。`domain.rs` 问卷结构与容量校验、`identity.rs` 身份规则、`logic.rs` 表达式编译与求值、`logic_document.rs` 文档规则与计算变量、`logic_text.rs` + `logic_text.pest` 文本语法、`flow.rs` 页面流程与会话。不依赖 HTTP、数据库和文件系统，可编译到 `wasm32-unknown-unknown`。 |
| `logic-wasm/`（包 `logic-wasm`） | WASM 边界。收一个 JSON 字符串、返回一个 JSON 字符串，只做序列化转换，不含语义。 |
| `backend/`（包 `questionnaire-api`） | HTTP 接口与业务用例。通过 `pub use questionnaire_logic::domain;` 直接调用领域实现。 |
| `frontend/` | Vue 3 应用。经 `frontend/src/wasm/index.ts` 包装入库的 WASM 产物调用同一份实现，没有 TypeScript 镜像。 |

阅读路线：只想确认“某个条件能不能写”看第 2 节；关心规则与计算变量怎么组织看第 3 节；关心跳题、改答和提交看第 4 节；只关心文本编辑器里的语法看第 5 节。

## 架构与设计原则

### 单一真源，双端调用

规则、流程和文本语法的语义只有 `logic-core` 一份实现。

- **后端**在 `backend/src/lib.rs` 写 `pub use questionnaire_logic::domain;`，业务代码继续用 `questionnaire_api::domain::*` 路径，接入时没有改动既有调用点。
- **前端**把 `logic-core` 编成 WASM，经 `logic-wasm` 的字符串信封调用，规则引擎不再有第二份实现。此前 Rust 与 TypeScript 各写一套内核，同一份规则在两端的边界行为需要靠人工对照维持，`formatLogicText` 更是只存在于 TS 一侧，格式化结果会与后端解析漂移。统一之后，两端调用的是同一段代码，双端差异只剩“调用方式”，不再是“两套语义”。
- **纯度闸门**：`cargo build -p questionnaire-logic --target wasm32-unknown-unknown` 必须通过，保证领域层不引入 HTTP、数据库或文件系统依赖；uuid v4 的随机源在 wasm 目标下切到 `getrandom` 的 `wasm_js` 后端。

### WASM 边界只做信封

`logic-wasm` 的每个导出函数都是“JSON 字符串进、JSON 字符串出”：

- 成功：`{"ok": …}`
- 失败：`{"error": …}`

信封由泛型 `Envelope<T>` 生成，形状只在这一处定义，成功值直接序列化一次，不需要先转成中间 `Value` 再包装，也不额外复制一份结果。错误按 `LogicError` / `LogicTextError` 的既有字段展开，前端包装层把它还原成原来的错误类，因此组件里的错误处理逻辑与统一之前完全一致。

### 前端接线

`frontend/src/wasm/index.ts` 是前端唯一的引擎入口，顶层 `await loadEngine()` 完成实例化，对外导出 6 个能力：

| 导出 | 作用 |
| --- | --- |
| `parseLogicText` | 文本 DSL → 文档规则模型 |
| `formatLogicText` | 文档规则模型 → 确定性文本（唯一实现现在在 Rust） |
| `compileDocumentLogic` | 编译文档规则，返回变量执行顺序与依赖信息 |
| `compileExpression` | 编译单条表达式，供表达式编辑器即时校验 |
| `renewDocumentIds` | 复制问卷时重写全部身份 |
| `FlowSession` | 页面流程会话：`dispatch`（`change` / `next` / `previous`）、`snapshot`、`evaluateAnswers` |

Vite 走 wasm-pack 的 bundler 胶水，`node --test` 走 `logic_wasm_bg.js` + `readFileSync` 手动实例化，两条路径共用同一份入库产物 `frontend/src/wasm/pkg/`。数字经 `frontend/src/json.ts` 的 `encode` / `decode` 传输，统一为有限 `f64`。改动 `logic-core/` 或 `logic-wasm/` 后在 `frontend/` 运行 `pnpm run wasm:build` 重建产物；只改前端时无需 Rust 工具链。

### 双端共读回归样例

`fixtures/survey-logic/` 是双端共同的契约来源：

| 文件 | 内容 |
| --- | --- |
| `expressions.json` | 表达式语义：短路、类型、精度、Unicode、容量边界 |
| `documents.json` | 文档规则与计算变量：拓扑、循环、前向引用、复制 |
| `flows.json` | 流程拓扑与事件序列：分支、合流、改答清理 |
| `submissions.json` | 服务端最终重建：路径外答案、必填、筛选结束 |
| `texts.json` | 文本语法与格式化往返 |
| `input-contracts.json` | 身份与空白输入的接受集 |

这些文件的期望值由人工指定，不由实现自行生成；Rust 回归测试与前端 `node --test` 各自读同一份文件。语义统一之后，样例的职责从“两个实现互相印证”变成“确认 WASM 边界两端看到同样的结果”，因此每份样例都同时约束原生路径（`cargo test --locked --workspace`）和 WASM 路径（`pnpm run test:regression`）。

## 表达式与三值逻辑

### 表达式包与版本

单条表达式以 JSON 对象提交：

```json
{ "dsl_version": 1, "semantics_version": 1, "expression": { "op": "boolean", "value": true } }
```

- 两个版本号都必须存在于实现中，未知版本直接拒绝，不允许“忽略后按当前版本猜”。
- 版本只接受整数 `1`；`1.0`、`1e0`、`true` 和 `"1"` 都不是有效版本号。
- 多处输入同时无效时不承诺诊断顺序；单处错误的分类与定位由共享样例约束。

### 静态类型与运行值

编译阶段确定 5 种静态类型：`boolean`、`number`、`text`、`choice`（单选身份）、`choices`（多选集合）。

运行值在此基础上多一个 `missing`：表示该题当前没有适用答案——没答、显式 `null`，或被流程层排除。缺失值保留原表达式的静态类型，因此布尔位置的 `missing` 就是三值逻辑中的 unknown。选项 UUID 与普通文本不会互相混用：选择类型只能用专用选择运算符判断。

### AST 节点

节点用 `op` 字段区分，未知字段一律拒绝。共 11 类：

| 节点 | 字段 | 类型规则 |
| --- | --- | --- |
| `boolean` / `number` / `text` | `value` | 分别是布尔、数字原文（字符串）、文本常量 |
| `answer` / `answered` | `question_id` | 返回该题类型的值或缺失；`answered` 返回布尔 |
| `variable` | `variable_id` | 计算变量；必须在同一文档中声明，不能冒充题目 ID |
| `selected` | `question_id`, `option_id` | 单选或多选，选项必须属于该题 |
| `contains_any` / `contains_all` | `question_id`, `option_ids` | 多选；选项集合非空、无重复且都属于该题 |
| `not` | `expression` | 布尔取反 |
| `all` / `any` | `expressions` | 非空布尔列表，从左向右短路 |
| `compare` | `comparison`, `left`, `right` | `eq`/`ne` 支持同类型的布尔、数字、文本；`lt`/`le`/`gt`/`ge` 仅数字 |
| `coalesce` | `value`, `fallback` | 同类型；只有缺失时才用替代值 |
| `length` | `value` | 文本的 Unicode 标量个数，或多选集合大小 |
| `arithmetic` | `operator`, `left`, `right` | 数字的 `add`/`subtract`/`multiply`/`divide` |

### 引用身份

配置身份与引用统一要求**非零、小写连字符 UUID**：题目、选项、计算变量、规则归属、流程入口/节点/出口/页面题目、文本绑定都走同一套规则。

- Rust DTO 在反序列化边界检查，非规范写法直接返回 `400 invalid_json`，AST 引用则在编译时拒绝。
- 身份判断用无分配的字节检查：长度必须为 36，字符只允许 `0-9`、`a-f`、`-`，且不能是全零 UUID。
- 选项**答案值**同样只接受非零、小写连字符 UUID，不再接受大写、无连字符、花括号或 `urn:uuid:` 写法；提交答案与改答事件里的 `question_id` 使用同一套严格身份规则。
- 共享样例逐个扰动各配置入口，避免其他字段的校验把某个入口的缺口盖住。

### 三值逻辑

布尔位置的 `missing` 记为 unknown，短路规则如下：

| 表达式 | 结果 |
| --- | --- |
| `not unknown` | `unknown` |
| `false && unknown` | `false` |
| `true && unknown` | `unknown` |
| `true \|\| unknown` | `true` |
| `false \|\| unknown` | `unknown` |

其余规则：

- 普通比较、算术、长度和选择判断一律传播缺失；`answered` 只看规范化答案是否存在，`coalesce` 显式消除缺失。
- 布尔列表从左向右短路，但编译器仍然检查列表里的每一条分支。
- **运行错误不是 unknown**。除零、溢出、类型不匹配必须返回结构化错误，不能悄悄降级成缺失。
- 求值前统一检查所有被引用的上下文值：即使某条分支会被短路，无效的上下文也不能通过；纯计算分支的运行错误只在真的执行到它时才触发。
- 空白文本与空多选的业务规范化由问卷层负责，原始错误输入不会默默变成缺失。

### 数值与文本

- 数字沿用现有答案的有限 IEEE 754 binary64 语义，不承诺任意精度小数。数字常量按有限 `f64` 解析，正负零等价（负零归一为 `0.0`）；超出双精度精确整数范围的数值按 `f64` 舍入。
- 数字常量原文最多 64 字节（含正指数的补号），不能带首尾空白。
- 比较是对规范化数值的精确比较，不隐式加容差；需要近似时由出题人自己写区间。
- 算术每一步都检查有限性，除零报错。
- 文本比较区分大小写，不自动 trim、不做 Unicode 归一化；长度按 Unicode 标量计算；拒绝 NUL 与非 Unicode 标量输入。文本空白按 Unicode `White_Space` 判定，与 Rust 的 `str::trim` 一致。

### 资源上限

| 约束 | 上限 |
| --- | --- |
| 单表达式节点数 | 256 |
| 单表达式嵌套深度 | 16（根算 1 层） |
| 数字常量原文 | 64 字节 |
| 文本常量 | 5000 个 Unicode 标量 |
| 单个选择集合 | 50 项 |

这些是**单条表达式**的限制；文档级只限制计算变量与题目规则的数量，表达式本身不再累计节点或字节预算。

## 文档规则与计算变量

### 结构

`Document.logic` 可缺省，序列化时省略；没有 `logic` 的问卷走单页流程。规则包包含四个必需字段，未知字段拒绝：

| 字段 | 内容 |
| --- | --- |
| `dsl_version` / `semantics_version` | 版本号，规则同上 |
| `variables` | 计算变量数组 |
| `rules` | 题目规则数组 |
| `flow` | 可选；页面流程定义，缺省时使用内存中的单页流程 |

- 计算变量：稳定 `id`、声明的 `value_type`、`expression`。
- 题目规则：目标 `question_id`，以及 `visible_if`、`required_if`、`validate` 中至少一个布尔表达式；可选字段的 `null` 等价于缺省。
- 同一个题目不能重复声明规则；变量身份不能与题目、选项或其他变量冲突。

### 变量依赖与拓扑排序

编译顺序是固定的三步：

1. 先绑定所有变量的类型；
2. 再检查每条表达式的结果与声明类型相符；
3. 最后做确定性拓扑排序——同一时刻就绪的变量取 UUID 最小者，因此同样的输入永远得到同样的执行顺序。

每个变量会被展开成**传递的**题目依赖集合，于是多层变量包装也无法绕过依赖检查：

- 变量不能有环（`DependencyCycle`）。
- `visible_if` 与 `required_if` 只能依赖严格前置的题目；`validate` 可以读当前题，但不能读后续题。
- 通过多层变量隐藏的自引用和前向引用同样会被拒绝（`ForwardReference`）。
- 删除变量、题目或选项，或重排题目导致引用失效时，保存直接失败并给出诊断，绝不自动清除规则。

变量计算结果按原生类型传给下游表达式，不转成答案 JSON 再解析一遍。编译结果保留 unknown，并提供变量执行顺序，以及在给定规范化答案上下文上的变量值与规则值。

### 题目规则的执行语义

- 显示条件为 `true` 才显示；隐藏题不必填。显示条件为 `false` 或 unknown 时隐藏。
- 静态必填与动态必填做逻辑或：静态必填为 `true` 时短路动态表达式，动态条件无法取消静态必填。动态必填为 unknown 时阻止继续，最终校验时必须给出明确诊断。
- 空的可选答案不运行自定义约束；已填答案先加入当前题约束的临时上下文，通过后才供下游使用。
- 显示条件的计算错误会保留题目并报错，不会因为“算不出来”就悄悄跳过。

### 文档级限制

| 约束 | 上限 |
| --- | --- |
| 计算变量 | 100 |
| 题目规则 | 100 组 |

单条表达式各自受限，文档不再对全部表达式的节点总数或序列化字节数另设预算。

### 复制与引用重写

- **整卷复制**做两遍 ID 映射：先为题目和选项生成新身份，再用映射重写题目、选项、变量、规则目标与流程节点里的全部引用。
- **单题复制**同时复制该题的目标规则，保留外部依赖，只重写直接指向自身的答案与选项引用；共享的计算变量不克隆，仍指向原变量。
- 复制不会改写文本常量里长得像 UUID 的字符串。

## 页面流程与会话状态机

### 流程结构

`flow` 包含入口 `entry` 和按拓扑顺序排列的 `nodes`，节点是三种封闭结构之一：

| 类型 | 字段 | 行为 |
| --- | --- | --- |
| `page` | `id`, `question_ids`, `next` | 顺序处理本页题目，`next` 事件通过后才离开 |
| `branch` | `id`, `branches`, `otherwise` | `branches` 是有序的 `{when, target}`，取第一个 `true`；unknown 或计算错误会阻止更低优先级的分支和默认出口 |
| `end` | `id`, `outcome` | `complete` 表示可提交，`screen_out` 表示筛选结束；两者本身都不产生保存或账号额度副作用 |

拓扑约束：

- 入口必须是数组中的第一个节点，且必须是页面。
- 所有边只能指向数组中的后续节点，即前向无环图；有限前向图加上“非终点都有出口”，保证任一路径最终都会到达结束节点。
- 最多 256 个节点；分支节点含 1—16 个条件，默认出口 `otherwise` 必须存在。
- 节点身份不能与题目、选项、变量或其他节点重复。
- 每页至少一道题，按节点顺序拼接所有页面的题目必须**恰好等于**文档题目数组一次，不重复、不遗漏、不重新排序。
- 规则导致整页隐藏时，`next` 会跳过该页。
- 直接的永假条件、直接遮蔽后续条件的真条件会被报错；任意表达式之间的恒真、互斥和不可达性不做一般性证明。

### 合流与缺失值保护

编译器计算每个节点必经的前置节点集合（dominators）。分支只能读取此前页面的题目——即使外面包了默认值，也不能引用未来题目。

在合流位置引用某条分支独有的答案，必须显式消除缺失，当前认可三种写法：

1. `answered(q)` 先判断是否存在；
2. `coalesce(value, fallback)` 给出替代值；
3. 从左到右的布尔短路守卫，例如 `all(answered(q), compare(answer(q), ...))` 或 `any(not(answered(q)), ...)`。

先读取再补检查**不算**保护（`UnavailableDependency`）。变量按拓扑顺序生成直接读取摘要，不反复展开变量 AST，因此通过变量间接引用同样会被检查到。这是保守检查，不尝试推导任意表达式之间的蕴含关系。

### 会话事件

`FlowSession` 绑定一份不可变文档，状态只能通过事件改变；Rust 借用文档，前端复制文档，对外快照与内部状态各自独立。

| 事件 | 输入 | 处理 |
| --- | --- | --- |
| `change` | `question_id`, `value` | 只允许改当前页的可见题；`value` 用答案 JSON 类型，`null` 表示清空。从入口重算有效路径、答案与错误，一次原子更新里清理不再适用的输入 |
| `next` | 无 | 当前页有错误则拒绝；否则前进到下一个可见页面或终点，并写入实际访问历史 |
| `previous` | 无 | 弹出当前有效历史，回到上一实际访问页面；翻页本身不删除仍然适用的答案 |

关键细节：

- 无效输入保留在会话内部供后续改答，但快照的 `answers` 只包含有效的规范化答案。
- 改答会清除被隐藏或落到路径外的输入与错误，仍然适用的后续答案保留并重新校验；再回到旧分支时，已清除的答案不会恢复。
- 失败事件不修改任何状态。
- 快照包含 `current_node`、`outcome`、实际访问的 `history`、按当前答案推导的 `path`、路径内 `visible` 与动态 `required`、有效 `answers`、题目 `errors`、路径末端的 `route_error`，以及最近一次改答清除的 `cleared`。
- 预测路径不包含自动分支节点，可以提前出现终点，但 `outcome` 只有在 `next` 真的到达终点后才出现。推导路径是“按当前答案会怎么走”，不是实际浏览轨迹。
- 错误可能来自尚未访问的后续页面；`next` 只检查当前页，界面按当前页呈现。
- 未配置 `logic` 的旧文档使用内存中的单页流程，虚拟页面 ID 为全零 UUID、虚拟终点 ID 为 `00000000-0000-0000-0000-000000000001`，只存在于内存快照，不写入文档，也不影响版本指纹。

### 服务端重建与报表三态

`FlowEvaluator` 让交互事件和服务端最终提交共用同一套投影逻辑：

- 提交请求仍然只包含版本、幂等键与答案数组，不接受路径、访问历史或客户端自己的适用性判断。
- 服务端从入口按顺序重建：先拒绝重复身份和外来身份，只让**已校验且适用**的规范化答案进入上下文，路径外答案不能影响合流处的必填条件。隐藏或未进入分支的答案即使为 `null` 也拒绝，不在服务端静默清理。
- 重建结果包括规范化 `answers`、覆盖全部题目的 `question_states` 和推导 `path`。到达 `complete` 只代表“可以提交”，事务成功之后才算已提交。
- `question_states` 取三态：`answered`（已答）、`unanswered`（适用未答）、`skipped`（被显示规则或流程分支跳过）。统计满足 `applicable_count = answered_count + unanswered_count`，且 `total_count = applicable_count + skipped_count`；选择题占比仍以已答数作分母。
- 历史版本按保存在该版本的规则重建适用性；没有规则的历史版本每题都视为适用。

### 尚未开放的部分

`screen_out` 节点的语义在引擎中完整可用（会话会返回筛选结果），但**包含筛选结束的文档目前不能发布**：筛选结果是否持久化、是否占用登录限次额度还没有定论，所以发布校验会直接拒绝，提示改用普通完成节点。重复组与子流程同样需要先确定实例身份、答案键扩展和报表结构，暂不进入基础版本。

## 文本语法契约（阶段 6）

文本是同一份 AST 的另一种编辑方式：解析后进入完全相同的编译与执行路径，不执行脚本，不引入新的运行语义。

```text
dsl 1 semantics 1;
bind question age = "00000000-0000-4000-8000-000000000001";
bind variable adult = "00000000-0000-4000-8000-000000002001";
bind node intro = "00000000-0000-4000-8000-000000001001";
bind node finish = "00000000-0000-4000-8000-000000001002";
let adult: boolean = answer(age) >= 18;
validate age = answer(age) >= 0;
entry intro;
page intro [age] -> finish;
end finish = complete;
```

### 别名绑定

- 所有 `bind` 集中写在声明之前。
- 别名是字母或下划线开头的标识符，全文唯一。
- 类别为 `question`、`option`、`variable`、`node`，必须用字符串显式绑定标准小写 UUID。
- 别名不依赖题目标题或位置，因此改标题、调顺序都不会改变文本的含义。
- 复制问卷仍由 AST 的身份映射负责；格式化文本会按新身份重新生成绑定。

### 变量与题目规则

```text
let name: boolean|number|text|choice|choices = expr;
visible  question = expr;
required question = expr;
validate question = expr;
```

同一个题目的同类规则不能重复声明。

### 流程语句

```text
entry name;
page name [q1, q2] -> next;
branch name { when expr -> target; else -> fallback; }
end name = complete|screen_out;
```

- 默认出口 `else` 必须放在最后。
- 只要存在节点，就必须且只能声明一个 `entry`；其余约束（前向拓扑、题目归属、每页非空）与第 4 节完全一致。

### 表达式与优先级

可用的函数与引用：`answer(q)`、`answered(q)`、`selected(q,o)`、`contains_any(q,[o1,o2])`、`contains_all(q,[o1,o2])`、`variable(v)`、`all(...)`、`any(...)`、`coalesce(value,fallback)`、`length(value)`；变量也可以直接写别名。布尔、JSON 字符串和保留原文的 JSON 数字都能作为字面量。

中缀运算符按优先级从低到高结合，同级左结合，括号显式分组：

| 优先级 | 运算符 |
| --- | --- |
| 1（最低） | `\|\|` |
| 2 | `&&` |
| 3 | `==` `!=` `<` `<=` `>` `>=` |
| 4 | `+` `-` |
| 5 | `*` `/` |
| 6（最高） | 前缀 `!` |

- 负号只用于数字常量；表达式取负写 `0 - expr`。
- 数字常量也可以写成 `number("原文")`，保证格式化往返时保留词法表示。
- 支持 `//` 行注释。注释和自定义别名不进入持久模型。

### 实现与限制

- 文法集中在 `logic_text.pest`（`pest` / `pest_derive 2.9`），运算优先级交给 `PrattParser`，不再手写优先级表。
- 先跑一遍**无递归**的词法入口做容量与嵌套检查，再进入完整文法：

| 约束 | 上限 |
| --- | --- |
| 文本大小 | 1 MiB UTF-8 |
| 解析嵌套深度 | 64（括号、函数调用、前缀 `!` 共用同一预算） |

- 生成出来的 AST 继续受更严格的限制约束：单表达式 16 层深度、256 个节点。长左结合中缀链在构造 AST 的阶段就会被拒绝，避免析构深树耗尽栈。

### 错误定位

- 语法错误保留行、列（列按 Unicode 标量计数）与稳定分类，不回显 pest 的原始输入片段。
- 语义错误保留 AST 路径，并定位到对应的声明。
- 编辑中的错误文本不会替换当前规则模型：只有点击“应用文本规则”并编译成功后，新文本才参与草稿保存；未应用的文本计入离开提醒并阻止误保存。

### 确定性格式化

`format_logic_text` 是唯一实现，输出确定性的别名与绑定顺序：

- 布尔列表统一用函数形式（`all(...)` / `any(...)`）书写；
- 保留原 AST 的分组和短路顺序；
- 数字常量保留原文，不做十进制展开；
- 往返满足 `parse(format(parse(x))) ≡ parse(x)`，回归中用 `texts.json` 的 `format_cases` 逐字节比对。

## 参考资料

设计时参考了以下几套语义模型的处理方式：

- [CEL：编译、类型检查、执行分离](https://cel.dev/overview/cel-overview)
- [SCXML：状态、事件与确定性转换语义](https://www.w3.org/TR/scxml/)
- [ODK XForms：显示、必填、约束、计算](https://getodk.github.io/xforms-spec/)
- [JsonLogic：跨语言真假值差异](https://jsonlogic.com/truthy.html)

实施状态、测试覆盖与浏览器/数据库联调范围见[需求分析中的演进与验证总结](requirements-analysis.md#9-架构演进历次关键决策与验证总结)；如何运行与阅读代码见 [README 第 7 节](../README.md#7-开发检查)、[后端指南](backend-guide.md) 与 [前端指南](frontend-guide.md)。
