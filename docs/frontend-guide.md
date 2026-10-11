# 前端 MVP 使用与代码阅读指南

`frontend/` 是连接现有 Rust API 的 Vue 3 应用。完整安装步骤见 [README](../README.md#31-启动前端-mvp)。前端依赖 PostgreSQL 和后端服务；`prototype.html` 与 `fluent-prototype.html` 仅保留为风格原型（后者对应当前 Fluent 2 视觉），不被应用引用。

## 1. 从创建到回收

1. 打开 `http://localhost:5173`，注册并登录。工作台显示当前账号的实际问卷数量，可按标题和状态筛选。
2. 新建问卷时选择空白问卷，或使用“满意度回访”“产品体验调研”“活动报名”模板。模板已配置条件或分支，每次创建生成独立身份；创建后可修改内容。
3. 编辑页分为“题目设计”“规则与跳题”“试填预览”“回收设置”。在题目设计中填写说明、收集目的和六种题型，也可从题目卡片直接进入规则。用当前草稿试填显隐和跳转，再设置回收方式并保存；“预览已保存草稿”仍读取服务器保存的版本。两种预览都不写入答卷。
4. 点击“发布问卷”。首次发布后填写模式固定，后续发布生成新版本。复制填写链接 `/s/:id`，在填写页提交真实答卷。
5. 打开“答卷与统计”，选择发布版本和北京时间范围，点击“应用筛选”。统计、答卷列表和 CSV 使用同一组已应用条件；历史版本不会合并到当前版本。
6. 在编辑页关闭、恢复或归档回收。归档前需要确认，归档后可看历史或复制为新草稿；只有未发布草稿可以删除。

普通用户的权限由后端校验。通过后端 `create-admin` 创建的管理员登录后可进入平台管理，查看用户、公开题面和审计，并填写原因执行治理操作。

## 2. 运行配置

| 项目 | 配置 |
| --- | --- |
| 前端开发与预览地址 | `http://localhost:5173`，端口被占用时退出，不自动换端口 |
| 后端默认地址 | `http://127.0.0.1:3000` |
| 后端根目录 `.env` | `PUBLIC_ORIGIN=http://localhost:5173`、`COOKIE_SECURE=false`，另配真实 `DATABASE_URL` |
| 前端可选 `.env` | `API_PROXY_TARGET=http://127.0.0.1:3000`，只供 Vite 服务器读取 |
| 开发 | `pnpm install --frozen-lockfile` 后执行 `pnpm run dev` |
| 构建与预览 | `pnpm run build`，停止开发服务器后执行 `pnpm run preview` |

浏览器只请求同源 `/api`。Vite 保留 `Origin`，统一请求函数携带 Cookie，写请求额外携带 `X-CSRF-Protection: 1`。不要将后端密钥放到浏览器变量中，也不要为开发关闭来源校验。若出现 `csrf_rejected`，核对页面地址、后端 `PUBLIC_ORIGIN` 及进程环境变量，并重启后端。

`dist/` 是构建输出，Vue 使用浏览器历史路由。实际应用需通过 HTTP 服务器提供；这里仅提供 Vite 本机开发和预览，未实现 Rust 静态文件托管或部署方案。

## 3. 代码结构与页面

| 文件 | 职责 |
| --- | --- |
| `main.ts`、`App.vue` | 单个 Pinia/Router 入口、工作区布局、移动导航、会话恢复弹窗 |
| `router.ts` | 路由、登录/管理员入口守卫、站内返回地址 |
| `api.ts`、`types.ts` | 请求、中文错误、CSV 下载、无损 JSON 及手写 API DTO |
| `stores.ts` | 登录态、通知、浅色/深色/跟随系统偏好 |
| `resource.ts` | 加载与重试，丢弃过时的筛选响应 |
| `documents.ts`、`answers.ts`、`format.ts` | 题目身份、动态作答校验、数字与北京时间展示 |
| `value-contract.ts`、`numbers.ts`、`json.ts` | Unicode 空白/标量、配置 UUID、数字原文与精度契约、无损 JSON 编解码（含 `encodeAnswers`） |
| `logic.ts` | 稳定错误码与 `LogicError` / `LogicTextError` 错误类；编译与求值已迁到 wasm 引擎 |
| `wasm/index.ts`、`wasm/pkg/` | DSL 引擎包装（信封解码、错误映射、无损数字）与入库的 wasm 产物五件套 |
| `flow-tools.ts` | 流程编辑器的拓扑改写（插入节点并回接所有指向原后继的出口）；语义校验仍交给 wasm 引擎 |
| `flow-inputs.ts`、`components/FlowFields.vue` | 原始输入适配与分步填写；会话状态来自 wasm 引擎 |
| `components/QuestionEditor.vue` | 六种题型的编辑与配置 |
| `survey-templates.ts`、`components/SurveyTemplatePicker.vue` | 创建入口的完整问卷模板及独立身份生成 |
| `condition-builder.ts`、`editor-logic.ts`、`components/ConditionBuilder.vue` | 常用条件与 AST 的转换、编辑器规则同步判定；复杂表达式保留高级编辑入口 |
| `components/LogicEditor.vue`、`components/FlowEditor.vue`、`components/FlowOverview.vue` | 题目规则、计算变量、分页分支及可定位的流程概览 |
| `components/LogicTextEditor.vue`、`components/DslCodeEditor.vue`、`logic-text-tools.ts` | 文本缓冲区、CodeMirror 高亮补全、引用与诊断定位 |
| `components/SearchSelect.vue`、`editor-context.ts` | 按需引入的 Element Plus 搜索选择器及共享编辑锁 |
| `components/DraftPreview.vue` | 无需保存即可试填当前草稿，复用已有填写运行时 |
| `components/SurveyFields.vue` | 预览和真实填写共享的题目控件 |
| `components/QuestionStats.vue` | 选项占比、评分分布、数字摘要和文本回答 |
| `styles.css`、`application.css`、`editor.css` | Fluent 2 设计令牌、业务布局与编辑工作区；组件库和代码编辑器共用主题 |

| 路径 | 页面 |
| --- | --- |
| `/`、`/surveys`、`/archive` | 工作台、问卷列表、已归档 |
| `/login`、`/register`、`/settings` | 登录、注册、账号与外观设置 |
| `/surveys/:id/edit`、`/surveys/:id/preview` | 编辑和已保存草稿预览 |
| `/s/:id` | 公开填写，登录限次模式先认证 |
| `/surveys/:id/reports` | 按版本统计、答卷详情、CSV 导出 |
| `/admin` | 用户治理、问卷治理、审计 |

## 4. 输入与一致性

文本题用于简短作答：单行默认且最多 200 字，多行默认且最多 1000 字，发布者可以在编辑器中调低上限。字符计数沿用 Unicode 标量值，填写页显示当前字数与该题上限，不静默截断。题数最多 100，全部按上限作答也能容纳在 1 MiB 的提交限制内；大段材料附件不在 MVP 范围。已有草稿若超过新上限，需要手动调低后保存或发布；已发布版本继续使用原有的题目上限。

编辑器单独记录已加载的 `revision`。回收设置响应不会推进本地草稿修订号；保存或发布冲突时保留本地内容，提供“下载当前草稿”和确认后重新加载。发布前读取后端预览校验结果，真正发布时仍提交修订号。离开未保存的编辑页会提示。

填写页使用本次加载的 `version_id`。如果服务器返回 `version_changed`，页面保留旧输入并提供新窗口链接，不把答案自动映射到新题面。

每次有效填写生成一个 UUIDv4 幂等标识。请求超时、服务错误或响应不完整时，锁定原答案与标识，通过“重试原提交”确认结果。登录限次提交还核对原账号；重新登录不会清空页面输入。明确的题目校验错误允许修改后重新提交。30 秒截止覆盖到响应正文消费结束：`api.ts` 在读完 `text()`/`blob()` 之后才清除定时器，因此“服务端已提交但正文传输卡住”会落到“尚未确认操作结果”的提示，而不是无限等待。

数字答案使用 `lossless-json` 发送和读取，保留输入精度，不会先将大整数舍入再发送。传输表示最多 64 个 ASCII 字符，包含负号、小数点和指数；与后端一致，省略的正指数符号先补齐（如 `1e3` → `1e+3`），再检查长度。超限定位到对应题目，最终精度和范围由后端校验。空统计展示“暂无数据”，合法的 `0` 仍显示为零。文本通过 Vue 插值展示，不执行题面或答案中的 HTML。

调查内容、答案和幂等标识仅在内存中，只有主题偏好保存在 `localStorage`。刷新或关闭页面会丢失尚未保存/确认的内容，当前没有离线草稿或跨刷新提交恢复功能。修改密码成功后按后端契约退出，所有设备需重新登录。

## 5. 视觉与当前边界

沿用已确认的 Fluent 2 视觉：控件 4px、卡片 8px、弹窗 12px 圆角，分级阴影，带轻微色调的 Mica 基底与纯色磁贴，细分隔线和清晰的键盘焦点；移动导航在浏览器支持 `backdrop-filter` 时叠加亚克力。三种主题共享 CSS 变量，窄屏下编辑器、工作台与填写页调整布局（断点 1600/1190/900/760/540），并适配 `prefers-reduced-motion` 与 `forced-colors`；导航和业务弹窗使用原生 `dialog`。

编辑器按需使用 Element Plus 2.14.7 的 `ElSelect` / `ElOption`，长题目列表支持搜索；CodeMirror 6 仅在进入文本模式时加载。两者使用 `editor.css` 适配现有 Fluent 令牌，不全局注册组件库。页面框架、原生弹窗、填写控件和轻量统计图继续沿用现有实现。题目排序通过上移/下移完成，统计文本逐批展开。API 类型为手写 DTO，后端契约变更时需同步维护。视觉约束以当前样式代码和 [Element Plus 可行性评估](element-plus-feasibility.md) 为准。

检查按改动选择：纯逻辑或测试调整可运行 `pnpm run test:regression tests/editor.test.mjs` 等相关文件；组件和类型调整按需运行 `pnpm run typecheck`；依赖或打包调整使用 `pnpm run build`，它已包含类型检查。改动 `logic-core/` 或 `logic-wasm/` 时先在 `frontend/` 运行 `pnpm run wasm:build` 重建入库产物，再跑受影响的两端测试；只改前端时直接用 `wasm/pkg/` 里已有的产物，无需 Rust 工具链。纯样式按需核对页面，格式化只处理改动文件。已通过的同一份代码不因提交而重新检查，前端变更不触发 Rust 检查。完整规则见 [README 第 7 节](../README.md#7-开发检查)。

前端包管理器固定为 pnpm（版本见 `frontend/package.json` 的 `packageManager`），锁文件为 `frontend/pnpm-lock.yaml`；不要使用 npm 或重建 `package-lock.json`。自动化测试沿用 Node 内置运行器；实际联调证据与未覆盖范围见 [实施记录](requirements-analysis.md#12-前端-mvp-实施与验证记录)。

DSL 基础计划已完成，报表页展示适用未答与规则跳过。编辑页的“规则与跳题”以可视化配置为默认入口，分为题目规则、计算变量和页面流程。常用条件可直接选择题目、比较方式和选项，支持全部/任一条件组合；显隐与动态必填只列出前面的题目，答案约束还可引用本题。变量、嵌套计算等表达式继续使用高级配置，切换不会自动改写已有表达式。点击“定位配置”会打开对应分类并聚焦错误的变量、规则或流程节点；删除题目、选项或节点后不会静默放宽依赖。

配置流程可从“每题一页”或一个页面开始，再添加分支；流程概览展示真实出口，点击节点即可定位配置。在“题目归属”中分配题目，每题恰好属于一页，页面拼接顺序须与题目顺序一致。出口只允许前往后续节点；分支按条件顺序匹配，并保留默认出口。入口须为第一个页面。新建题目自动加入最后一个页面，复制单题保留规则和页面归属；移动题目或节点后需要重新检查配置。包含筛选结束 `screen_out` 的问卷只能预览，暂不可发布。

“文本 DSL”与可视化配置使用同一规则模型，提供行号、高亮、补全、查找、撤销与引用速查，输入后延迟 300 毫秒运行 wasm 引擎的解析和编译检查（`logic-core` 的唯一实现）。文本先显式 `bind` 可读别名到 UUID，再声明变量、规则与节点；完整语法见 [文本语法契约](survey-dsl-plan.md#文本语法契约阶段-6)。点击“应用文本规则”或按 Ctrl / ⌘ + Enter 后，再保存草稿。错误标明行、列与规则路径，支持定位，不覆盖当前规则。Ctrl + Space 触发补全，Tab 默认移到下一控件。

尚未应用的文本会阻止保存和切回可视化；切换到题目、试填或回收工作区时保留文本，试填使用上次应用的规则。题目身份、题型或规则变化后会提示重新核对引用。可用“恢复当前规则文本”放弃缓冲区修改，也可导出 `.dsl` 文件；导入当前问卷的规则文本后仍需应用，文件最多 1 MiB。生成文本使用确定性的 `q1`、`q1_o1`、`v1`、`n1` 等别名，UUID 才是身份；格式化由 wasm 引擎调用后端同一份 `format_logic_text` 完成；“整理文本”会先确认，再统一别名并移除注释。

预览和正式填写共用 `FlowFields.vue`、`FlowInputs` 和 `FlowSession`，只显示当前页面的可见题目，标记动态必填，下一步校验当前页并聚焦错误；回退沿有效历史，改答即时清除不再适用的答案。完成后展示核对页，必须明确确认才提交；预览只显示校验结果。正式提交结果未知时冻结答案、版本和幂等键，重试原载荷。`copyPayload` 保留提交数字原文，避免 `1e20` 在冻结时展开成超范围裸整数。无规则问卷保持原有单页行为。

## 6. DSL 引擎（wasm）

表达式、文档规则、流程与文本解析只有一份 Rust 实现：`logic-core`（`questionnaire-logic`）编到 wasm，`logic-wasm` 以 JSON 字符串信封导出，`frontend/src/wasm/index.ts` 负责包装。前端不再有 TS 镜像实现；两侧共用 `fixtures/survey-logic/` 的人工期望。

边界约定：导出全是同步函数，入参是 JSON 文本，出参是 `{"ok":…}` 或 `{"error":{code,path,question_id,line?,column?}}`。包装只做三件事——用 `json.ts` 的无损编解码把入参压成 JSON 字符串、把错误信封还原成既有错误类、把求值结果里已被领域层规范化为有限 `f64` 的数字还原成 TS `number`。

| 导出 | 用途 |
| --- | --- |
| `parseLogicText(source, questions)` | 解析文本 DSL；语法/语义错误抛带行列的 `LogicTextError` |
| `formatLogicText(logic, questions)` | 格式化回文本；输出与后端逐字节一致 |
| `compileDocumentLogic(logic, questions)` | 保存前诊断预检；返回 `variableOrder` 与 `evaluate(answers)` |
| `compileExpression(raw, questions)` | 表达式类型与依赖提示；返回 `valueType` / `dependencies` / `evaluate(answers)` |
| `renewDocumentIds(document)` | 复制问卷或规则时重写身份并同步全部引用 |
| `FlowSession` | 有状态填写会话：`snapshot()` / `dispatch(event)` / `evaluateAnswers(answers)` |
| `ready` | 引擎就绪 Promise；包装在模块顶层 await 它，导入方可以直接同步调用 |

- 错误类仍是 UI 契约：`logic.ts` 只保留 `LogicError` / `LogicTextError` 与稳定错误码文案，界面按 `code` 分支、按 `message` 展示。
- 数字精度：入参经 `json.ts` 的 `encodeAnswers` 把 JS `number` 还原成最短词法表示（普通写法与指数写法取更短者），`1e20` 不会在过边界前被展开成超范围裸整数；出参只把领域层已规范化的结果转回 `number`。
- 运行环境：Vite/浏览器走 wasm-pack 的 bundler 胶水（`pkg/logic_wasm.js`）；`node --test` 走 `pkg/logic_wasm_bg.js` + `readFileSync` 手动实例化——bundler 产物按打包器契约生成，既不提供 `initSync`，其 `.wasm` 静态导入也只有打包器能解析。
- 产物与重建：`frontend/src/wasm/pkg/` 的五件套（`logic_wasm.js`、`logic_wasm_bg.js`、`logic_wasm_bg.wasm`、`logic_wasm.d.ts`、`logic_wasm_bg.wasm.d.ts`）入库。改动 `logic-core/` 或 `logic-wasm/` 后在 `frontend/` 运行 `pnpm run wasm:build`（即 `wasm-pack build ../logic-wasm --target bundler --release --no-opt` 后由 `scripts/copy-wasm.mjs` 同步五件套）；需要 Rust 工具链 + `wasm32-unknown-unknown` target + wasm-pack 0.15。只改前端时直接用入库产物，不需要 Rust。
- 测试：`frontend/tests/` 中的 `logic`、`logic-text`、`document-logic`、`flow`、`flow-inputs` 等文件改从 `src/wasm/index.ts` 导入，断言不变。共享样例仍是唯一语义契约，只是从「双实现对照」变成「单实现双端回归」；`node --test` 与浏览器共用同一份包装。
