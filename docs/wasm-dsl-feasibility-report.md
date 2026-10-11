# 统一前后端 DSL 处理的 WASM 复用可行性报告

> 调研日期:2026-10-11;基线:`dsl-survey-editor` 分支 HEAD `48eab76`。
> 定位:决策依据文档,只陈述证据与结论,不改动任何代码、配置或既有文档。
> 执行细节(阶段拆分、逐 Phase 验证、验收标准)以 `docs/wasm-dsl-unification-plan.md` 为准。

## 1. 结论

可行,条件成熟,建议采纳 `docs/wasm-dsl-unification-plan.md` 作为执行计划重启,预计 6–8 个工作日
(按该计划 Phase 0–5 分解;其中 Phase 0 工具链已在本机完成,无需重做)。

支撑判断的四项事实:

- DSL 核心六文件已是纯逻辑,无 DB/HTTP/tokio/sea-orm 依赖(§3 逐文件核实 use 语句)。
- 工具链已装:`wasm32-unknown-unknown` target 与 `wasm-pack 0.15.0` 均已在本机(2026-10-11 核实)。
- 共享契约已存在:`fixtures/survey-logic/` 六文件被双端测试消费,是 wasm 化后的语义安全网(§2)。
- 先前拆分尝试已验证到编译接线形态(§3),回滚为非技术性决定,无技术阻塞记录。

唯一真实的技术前置风险是 getrandom 在 wasm32 的编译(`Uuid::new_v4` 的底层依赖),对策已核实
可行,见 §4。

## 2. 现状:双实现的重复与漂移

同一套 DSL 语义存在两份手写实现,行数与文件对应关系如下(2026-10-11 实测):

| 语义层 | Rust(后端) | TS(前端镜像) |
|---|---|---|
| 表达式编译/三值求值 | `backend/src/logic.rs`(828 行) | `frontend/src/logic.ts`(511 行) |
| 文档规则/变量拓扑 | `backend/src/logic_document.rs`(449 行) | `frontend/src/document-logic.ts`(393 行) |
| 文本 DSL 解析 | `backend/src/logic_text.rs`(587 行,pest) | `frontend/src/logic-text.ts`(469 行,手写递归下降) |
| 流程编译/会话 | `backend/src/flow.rs`(963 行) | `frontend/src/flow-compiler.ts`(272 行)+ `frontend/src/flow.ts`(361 行) |
| 身份/数值契约 | `backend/src/identity.rs`(24 行)+ `domain.rs` 部分 | `frontend/src/value-contract.ts` + `numbers.ts` |

- TS 核心引擎 5 文件共 2 006 行;Rust 侧六文件共 3 631 行(`domain.rs` 715 行计入);文本语法
  另有 `backend/src/logic_text.pest`(65 行,grammar 规则)。
- 两端错误码集合一致:`LogicErrorCode` 共 18 个,TS 与 Rust 一一对应。
- 已发生的单边漂移:`formatLogicText`(文本 DSL 格式化,`frontend/src/logic-text.ts:383`)仅有
  TS 实现,后端 `backend/src/` 无 `format_logic_text` 对应(2026-10-11 grep 核实)——双实现模式
  已产生实际维护成本。
- 一致性机制:`fixtures/survey-logic/` 六文件(`documents/expressions/flows/submissions/texts/`
  `input-contracts.json`),后端 `backend/tests/` 以 `include_str!` 消费,前端
  `frontend/tests/*.test.mjs` 以 `readFileSync` 消费(node:test 运行)。该机制是 wasm 化后的
  语义安全网:fixtures 从「双实现对照」变为「单实现双端回归」,继续有效。

## 3. crate 复用的结构可行性

现有 workspace 只有 `backend` 一个成员(包名 `questionnaire-api`);DSL 六文件与 axum/sea-orm/tokio
同 crate,无法直接编译到 wasm —— 必须抽出共享 crate,这是「复用 crate」要求的核心改动。

拆分形态(先前尝试已验证到编译接线):

- 根 `Cargo.toml` `members = ["backend", "logic-core", "logic-wasm"]`。
- `logic-core/`(crate 名 `questionnaire-logic`)收编六文件 + `logic_text.pest`,依赖裁剪为
  `serde`、`serde_json`(保留 `arbitrary_precision` feature,仓库规约禁止移除)、
  `pest`/`pest_derive` 2.9、`uuid`、`thiserror`、`utoipa`。
- 后端零改动接线:`backend/src/lib.rs` 删六个 `mod` 声明,代之以
  `pub use questionnaire_logic as domain;` —— `questionnaire_api::domain::*` 路径不变,业务代码
  与 `backend/tests` 四目标的 `use questionnaire_api::domain::{…}` 均零改动。先前尝试的审计
  结论:业务代码只经 `domain::` 再导出访问核心(`Document`、`Answer`、`ValidationError`、
  `MAX_SUBMISSION_BYTES`、`FlowEvaluator`、`renew_ids`、`Question`、`QuestionKind`、`Choice`、
  `AnswerState`),无 `pub(crate)` 越界引用,可见性无需调整。

DSL 层纯度(已逐文件核实 use 语句):仅依赖 `std`、`serde`、`serde_json`、`pest`、`uuid`、
`thiserror`、`utoipa`(仅 `ToSchema` derive)。无 tokio/sea-orm/axum/chrono/getrandom 直接依赖。

`utoipa` 在 wasm32 的编译预期可行(派生宏产物为纯数据结构,无宿主 API 依赖),由执行计划的
Phase 1 纯度闸门(`cargo build -p questionnaire-logic --target wasm32-unknown-unknown`)兜底验证。

## 4. 唯一硬风险:`Uuid::new_v4` → getrandom(已核实对策)

调用点仅三处:

- `backend/src/domain.rs:473,480`(`renew_ids`)
- `backend/src/flow.rs:129`(虚拟节点 ID)
- `backend/src/logic_document.rs:378`

lock 图已核实:`uuid 1.27.0 → getrandom 0.4.3`,且 uuid 已携带 `js-sys`/`wasm-bindgen`
依赖(wasm 兼容路径已在依赖图中)。getrandom 0.4 提供 `wasm_js` feature。

对策(首选):`logic-wasm` 声明 `getrandom = { version = "0.4", features = ["wasm_js"] }`
(feature 合并生效),`.cargo/config.toml` 增加
`[target.wasm32-unknown-unknown] rustflags = ["--cfg", "getrandom_backend=\"wasm_js\""]`。
备选(不首选,语义微调):`flow.rs` 虚拟节点 ID 改确定性生成(该 ID 注释已声明不入指纹)。

## 5. 边界与集成可行性

wasm 边界用 JSON 字符串(信封 `{"ok":…} | {"error":…}`),不用 serde-wasm-bindgen:

- 天然保住数字精度(Rust `arbitrary_precision` ↔ JS `lossless-json`,两端既有约定不动)。
- 错误结构(`code/path/question_id/line/column`)无损穿透。
- 编辑器已有 300ms 防抖,解析微秒级,开销可忽略。

导出形态:

- `FlowSession` 为有状态类(`new(document_json)` / `dispatch(event_json)` / `snapshot()`),
  状态留 wasm 线性内存,避免每次改答重编译文档。
- `parse_logic_text` / `format_logic_text` / `compile_document_logic` / `compile_expression` /
  `renew_document_ids` 为无状态函数。

前端集成已核实无障碍:Vite 8.3.0 bundler 产物直接可用;`node --test`(Node 22 原生跑 .ts)经
`initSync(readFileSync(...))` 加载;仓库无 CI、前端开发者不装 Rust 工具链 → `.js` 胶水 + `.wasm`
产物入库,`frontend/package.json` 加可选 `wasm:build` 脚本。

删除收益:TS 镜像引擎约 −1 900 行(保留 `logic-text-tools.ts`、`condition-builder.ts`、
`editor-logic.ts`、`answers.ts` 及 `numbers.ts`/`value-contract.ts`/`json.ts` 等纯 UI/适配件);
`formatLogicText` 移植到 Rust 后单边漂移归零。

## 6. 成本与收益汇总

成本:6–8 个工作日(Phase 0 工具链已完成;Phase 1 抽 crate 1–2 天;Phase 2 格式化器移植 1 天;
Phase 3 wasm crate 1–2 天;Phase 4 前端换引擎删镜像 2 天;Phase 5 文档半天)。工具链已核实就绪:
`wasm32-unknown-unknown` target 与 `wasm-pack 0.15.0` 均已安装。

收益:DSL 语义单实现;双端回归从「双实现对照」简化为「单实现双端」;每次语义变更从双端同步
修改降为单侧修改 + 双端回归;消除已存在的格式化器漂移。

先前回滚非技术性失败:计划文档自述「按决定回滚、保留完整计划供后续重启」,无技术阻塞记录;
接线形态与踩坑清单(`git mv` 保历史、`lib.rs` 模块列表易误删 `mod entities;`)均已沉淀在
该文档 §2。

## 7. 与既有文档的关系

本报告为决策依据;执行细节(阶段拆分、逐 Phase 验证、验收标准)以
`docs/wasm-dsl-unification-plan.md` 为准,不在此重复。

## 8. 报告自身的验收标准

- 引用的每个行号、版本号、行数统计与仓库一致(撰写时已逐一核查:`wc -l`、`Cargo.lock` 依赖块、
  `wasm-pack --version`、`rustup target list --installed`、`formatLogicText` 与
  `format_logic_text` 双向 grep、`Uuid::new_v4` 调用点定位)。
