# 前端开发指南

`frontend/` 是一个 Vue 3 应用：Vue 3 + TypeScript + Vite + Pinia + Vue Router，包管理器固定为 **pnpm**（锁文件 `frontend/pnpm-lock.yaml`，请勿使用 npm 或重新生成 `package-lock.json`）。它的职责是把后端的 HTTP JSON 接口变成一套可用的问卷工作台，并且通过入库的 wasm 产物直接复用 `logic-core` 的规则与流程实现——DSL 语义在前端没有第二份实现。

首次运行请按 [README 第 3.1 节](../README.md#31-启动前端-mvp) 启动后端与前端；开发检查的取舍见 [README 第 7 节](../README.md#7-开发检查)。本文聚焦：前端有哪些流程、代码放在哪里、状态一致性怎么保证、wasm 引擎怎么接进来。

## 1. 核心业务流程

1. **工作台**（`/`、`/surveys`、`/archive`）：显示当前账号的问卷列表与数量，可按标题搜索、按状态筛选，归档问卷单独一页。列表项提供「继续编辑 / 查看答卷」「复制问卷」以及未发布草稿的删除入口。
2. **创建**：从空白问卷开始，或选一个模板。「满意度回访」带条件显隐与计算变量，「产品体验调研」带分页跳转与分支汇合，「活动报名」带动态必填与答案约束。模板每次创建都会生成一套全新的题目身份，条件与流程引用的是本次生成的题目，所以创建后可以放心修改。
3. **四合一编辑器**（`/surveys/:id/edit`）分为四个工作区：
   - **题目设计**：标题、调查说明、收集目的、结束语，以及六种题型（单选、多选、单行文本、多行文本、数字、评分）的配置。
   - **规则与跳题**：可视化配置（题目显隐与必填、计算变量、页面流程）与文本 DSL 两个入口，二者操作同一份规则模型。
   - **试填预览**：直接用当前内存中的草稿试填，体验显隐与跳转；不保存、不提交。
   - **回收设置**：填写模式（匿名 / 登录限次）、截止时间，以及关闭、恢复、归档、删除等状态操作。
   另有「预览已保存草稿」（`/surveys/:id/preview`）读取服务器上那一版草稿，用于确认保存在服务端的内容。两种预览都不写入答卷。
4. **发布与分享**：发布把已保存的草稿变成一个新版本；第一次发布后填写模式冻结。发布成功后可以复制填写链接 `/s/:id`，或直接打开新窗口。
5. **填写**（`/s/:id`）：匿名模式直接开始；登录限次模式先完成登录。配置了规则的问卷按页分步填写，无规则的问卷保持原来的单页表单。有规则的问卷完成后先进入核对页，必须显式确认才会真正提交。
6. **报表**（`/surveys/:id/reports`）：先选发布版本和时间范围，再点「应用筛选」；统计、答卷列表和 CSV 导出共用这一组已应用条件，浏览过程中新增的答卷不会混进当前视图。时间一律显示为北京时间（Asia/Shanghai）。
7. **平台管理**（`/admin`，仅管理员）：用户管理、问卷治理（下架 / 恢复，可查看治理详情中的当前已发布题面）和操作审计。每个治理动作都要填写原因，并与审计记录写在同一事务里。
8. **账户设置**（`/settings`）：修改密码（成功后所有设备失效）与外观主题。

前端的页面守卫只决定「入口是否展示」，资源的真实权限始终由后端校验。

## 2. 本地运行与代理配置

| 项目 | 配置 |
| --- | --- |
| 开发与预览地址 | `http://localhost:5173`（`strictPort`，端口被占用时直接退出，不自动换端口） |
| 后端默认地址 | `http://127.0.0.1:3000` |
| 后端根目录 `.env` | `PUBLIC_ORIGIN=http://localhost:5173`、`COOKIE_SECURE=false`，另配真实的 `DATABASE_URL` |
| 前端可选 `.env`（复制 `.env.example`） | `API_PROXY_TARGET=http://127.0.0.1:3000`，只由 Vite 服务器读取 |
| 安装 | `pnpm install --frozen-lockfile` |
| 开发 | `pnpm run dev` |
| 构建与预览 | `pnpm run build`，然后停止开发服务器再执行 `pnpm run preview` |

浏览器只请求同源 `/api`，Vite 把 `/api` 代理到后端，并保留原始 `Origin`（`changeOrigin: false`），来源校验仍然由后端执行。因此：

- 后端 `PUBLIC_ORIGIN` 必须与浏览器地址一致，否则写请求会得到 `403 csrf_rejected`；出现该错误时核对页面地址、后端 `.env` 和进程环境变量，改完要重启后端。
- 统一请求函数会携带 Cookie，写请求额外带上 `X-CSRF-Protection: 1`。
- 不要把后端密钥放进浏览器可见的变量；开发时也不需要关闭来源校验。

`dist/` 是构建产物，应用使用浏览器历史路由，需要由 HTTP 服务器提供。本项目只包含 Vite 的本机开发与预览，没有实现 Rust 托管静态文件或生产部署方案。

## 3. 目录结构与模块职责

### 3.1 核心模块

| 文件 | 职责 |
| --- | --- |
| `main.ts`、`App.vue` | 应用入口与工作区骨架；侧边栏 / 移动导航、全局提示、会话失效弹窗 |
| `router.ts`、`views/*.vue` | 路由表、入口守卫与站内返回地址；每个页面按需动态导入 |
| `api.ts` | `api()` / `download()` 请求封装、`ApiError`、中文错误文案、查询串拼接；30 秒截止覆盖到响应正文读完 |
| `json.ts` | 基于 `lossless-json` 的 `decode` / `encode` / `copy`，以及保留数字传输写法的 `copyPayload` 与 `encodeAnswers` |
| `types.ts` | 手写 API DTO 与领域类型（`SurveyDocument`、`QuestionStatistics`、`FlowSnapshot` 等） |
| `stores.ts` | Pinia store：`useAuth`（会话恢复、登录、退出）、`useAppearance`（浅色/深色/跟随系统）、`useNotice`（提示条） |
| `resource.ts` | `useResource`：加载态、错误、重试，并用序号丢弃过期的筛选响应 |
| `documents.ts`、`answers.ts` | 新题与复制题的身份生成、`TEXT_ANSWER_LIMITS`；无规则问卷的本地作答校验 |
| `value-contract.ts`、`numbers.ts`、`format.ts` | Unicode 空白/标量校验；数字输入的规范化与错误提示；状态与题型名称、时间与选项展示、分享链接 |
| `logic.ts` | `LogicError` / `LogicTextError` 与稳定错误码（编译与求值已全部迁到 wasm 引擎） |
| `wasm/index.ts`、`wasm/pkg/` | DSL 引擎包装与入库的 wasm 产物五件套（详见第 6 节） |
| `flow-tools.ts`、`flow-inputs.ts` | 流程编辑器的拓扑改写；填写会话的输入适配（界面原文 ↔ 引擎类型） |
| `editor-context.ts`、`editor-logic.ts` | 编辑区的共享禁用状态；把引擎错误映射为可定位的诊断 |
| `condition-builder.ts`、`logic-text-tools.ts` | 常用条件与 AST 互转、运算符切换；文本 DSL 的签名、别名绑定表与错误偏移换算 |
| `survey-templates.ts` | 三套内置模板的完整文档与独立身份生成 |

几个容易记混的点：

- `api.ts` **不再**重导出 `json.ts` 的内容（`copy` / `decode` / `encode`）；需要无损 JSON 的模块直接从 `./json` 导入。
- `numbers.ts` 只导出 `numberInput`（清洗输入）与 `numberError`（本地长度/格式提示）；数字的最终精度和范围由后端判定。
- `stores.ts` 的 `useAuth` 不对外暴露内部 `initialized` 标志——会话恢复是否完成只在 store 内部使用，调用方只需要 `user`、`restore()`、`login()`、`logout()`。
- 接口类型是手写 DTO；后端契约变更时需同步维护 `types.ts`。

### 3.2 视图与组件

| 路径 | 页面 |
| --- | --- |
| `/`、`/surveys`、`/archive` | 工作台、问卷列表、已归档（同一个 `DashboardView`） |
| `/login`、`/register` | 登录、注册 |
| `/settings` | 账户设置（密码、外观） |
| `/surveys/:id/edit` | 四工作区编辑器 |
| `/surveys/:id/preview` | 已保存草稿的预览 |
| `/surveys/:id/reports` | 按版本的统计、答卷详情与 CSV 导出 |
| `/admin` | 用户治理、问卷治理、操作审计 |
| `/s/:id` | 公开填写（登录限次模式先认证） |
| `/:pathMatch(.*)*` | 页面不存在 |

| 组件 | 用途 |
| --- | --- |
| `QuestionEditor.vue` | 六种题型的编辑与配置（选项数量、字数上限、数值范围等） |
| `SurveyFields.vue` | 无规则问卷的题目控件，预览与正式填写共用，含文本字数与上限显示 |
| `FlowFields.vue`、`FlowOverview.vue`、`FlowTarget.vue` | 分步填写与页面流程：当前页渲染、进度、出口选择与可定位的流程概览 |
| `LogicEditor.vue`、`ExpressionEditor.vue` | 规则与计算变量的配置；复杂表达式使用高级编辑入口 |
| `ConditionBuilder.vue` | 可视化条件构建器（题目 / 比较方式 / 选项或阈值） |
| `LogicTextEditor.vue`、`DslCodeEditor.vue` | 文本 DSL 缓冲区与 CodeMirror 6 编辑器（高亮、补全、诊断定位） |
| `DraftPreview.vue` | 当前草稿的内存试填 |
| `QuestionStats.vue` | 选项占比、评分分布、数字摘要与文本回答 |
| `SearchSelect.vue` | 按需引入的 Element Plus 搜索选择器（长题目列表可搜索） |
| `SurveyTemplatePicker.vue` | 创建入口的模板选择 |
| `AuthForm.vue`、`Modal.vue`、`Pagination.vue`、`ErrorNotice.vue`、`LoadingState.vue`、`Icon.vue` / `Icons.vue` | 登录表单、原生 `dialog` 封装、分页、错误提示、加载态与本地图标符号 |

## 4. 编辑器、填写与状态一致性

### 4.1 文本字数与数字原文

文本题的字符数按 Unicode 标量值计数。新题默认使用上限（单行 200、多行 1000，见 `documents.ts` 的 `TEXT_ANSWER_LIMITS`），发布者可以在编辑器里调低；填写页实时显示「当前字数 / 该题上限」，不做静默截断。本地校验用 `value-contract.ts` 的 `isBlank` / `validText`，与后端规则保持一致：`U+0085` 算空白、`U+FEFF` 算内容，含 NUL 或孤立代理项的输入直接拒绝。

数字答案用 `lossless-json` 发送和读取，不会先把大整数舍入再发送：普通输入保留原文，流程引擎规范化后的结果用指数形式传输，避免 `1e20` 被 JS 展开成超出范围的裸整数。题面、选项和规则文本都通过 Vue 插值渲染，不会执行里面的 HTML。

### 4.2 草稿修订号与冲突处理

编辑器单独记录「已加载的 `revision`」。保存或发布冲突（`409 revision_conflict`）时不会丢掉本地内容：界面保留当前编辑，提供「下载当前草稿」和确认后重新加载两个选择。回收设置的响应不会推进本地草稿修订号，否则会把其他窗口的编辑冲突掩盖掉。离开有未保存改动的编辑页会先提示。

### 4.3 填写会话

填写与预览共用 `FlowFields.vue` + `FlowInputs` + wasm 的 `FlowSession`：状态保存在 wasm 线性内存里，界面只拿到快照。只显示当前页可见的题目，动态必填在进入下一步时校验并把焦点移到第一处错误；回退沿有效历史前进，改答会立刻清除不再适用的答案。完成后进入核对页，必须明确确认才提交；预览只展示校验结果，不写入任何数据。没有规则的问卷继续使用原有的单页行为。

### 4.4 提交、幂等重试与超时

每次有效填写生成一个 UUIDv4 幂等标识，随答案一起提交：

- 请求超时、服务端错误或响应不完整时，页面**冻结**原答案、版本和幂等键，只允许通过「重试原提交」获取结果；登录限次模式还会核对是否仍是原账号。重新登录不会清空页面输入。
- 明确的题目校验错误（`422`）允许修改后重新提交。
- 如果后端返回 `version_changed`（问卷已发布新版本），页面保留旧输入并提示重新加载题面核对，**不会**把答案自动映射到新题面。

30 秒截止覆盖到响应正文消费结束：`api.ts` 在读完 `text()` / `blob()` 之后才清除定时器。所以「服务端已提交、但正文传输卡住」会落到「尚未确认操作结果」的提示，而不是无限等待或误报失败。

会话与填写内容都只存在内存中，只有主题偏好写入 `localStorage`。刷新或关闭页面会丢失尚未保存或确认的内容——当前没有离线草稿或跨刷新提交恢复功能。

### 4.5 可视化规则与文本 DSL 的同步

两种编辑方式操作同一份规则模型，不会各存一份：

- 可视化条件构建器（`condition-builder.ts`）在常用条件下把表单转成 AST；同一类运算符之间切换时会保留已填的数字阈值、文本或选项身份，只有操作数类型变化才初始化新值。
- 文本 DSL 编辑器（`LogicTextEditor.vue`）用 CodeMirror 6 提供行号、高亮、补全、查找、撤销与引用速查。输入后延迟 300 毫秒调用 wasm 引擎做解析与编译检查，错误会标明行、列和规则路径，并可一键定位，但不会覆盖当前规则。
- 文本先显式 `bind` 可读别名到 UUID，再声明变量、规则与节点；完整语法见[文本语法契约](survey-dsl-plan.md#文本语法契约阶段-6)。
- 未应用的文本会阻止保存和切回可视化工作区；切到题目、试填或回收设置时缓冲区保留，试填仍使用上次应用的规则。题目身份、题型或规则变化后会提示核对引用。
- 「恢复当前规则文本」放弃缓冲区改动并重新生成；也可导出 `.dsl` 文件，或导入当前问卷的规则文本（导入上限 1 MiB，导入后仍需应用）。「整理文本」会先确认，然后统一别名并移除注释。
- 生成文本使用确定性的 `q1`、`q1_o1`、`v1`、`n1` 别名，但**UUID 才是身份**；格式化由 wasm 引擎完成，输出与服务端逐字节一致。
- 流程编辑只允许前往后续节点，分支按条件顺序匹配并保留默认出口，入口必须是第一个页面；包含「筛选结束」（`screen_out`）的问卷只能预览，暂不能发布。

## 5. 视觉规范与按需组件

界面沿用 Fluent 2 视觉：控件 4px、卡片 8px、弹窗 12px 圆角，分级阴影，带轻微色调的 Mica 基底，细分隔线和清晰的键盘焦点；移动导航在浏览器支持 `backdrop-filter` 时叠加亚克力。三种主题（浅色 / 深色 / 跟随系统）共享同一组 CSS 变量。

- 响应式断点为 1600 / 1190 / 900 / 760 / 540：编辑器、工作台与填写页在窄屏下调整布局，并适配 `prefers-reduced-motion` 与 `forced-colors`。
- 导航和业务弹窗使用原生 `dialog`（`Modal.vue` 统一封装焦点与关闭行为）。
- 样式分三层：`styles.css` 放设计令牌与框架级样式，`application.css` 放业务布局，`editor.css` 放编辑工作区，组件库和代码编辑器共用同一套令牌。

组件按需引入，不做全局注册：

- Element Plus 只在 `SearchSelect.vue` 中使用 `ElSelect` / `ElOption`（2.14.7），并同时引入这两个组件的样式，用于长题目列表的搜索选择。
- CodeMirror 6 只在 `LogicTextEditor.vue` 里用 `defineAsyncComponent` 延迟加载 `DslCodeEditor.vue`，不进入初始包。
- 页面框架、原生弹窗、填写控件和轻量统计图继续沿用项目自有的实现与样式。

## 6. WASM DSL 引擎接入

### 6.1 单一实现架构

表达式、文档规则、流程会话与文本解析只有一份 Rust 实现：

```text
logic-core/（questionnaire-logic，纯领域）
        │  wasm-pack build --target bundler
        ▼
logic-wasm/（logic-wasm，JSON 字符串边界）
        │  入库产物 frontend/src/wasm/pkg/
        ▼
frontend/src/wasm/index.ts（TypeScript 包装）
        ▼
编辑器、条件构建器、填写会话、报表侧校验
```

前端不再有 TS 镜像实现；两端共用 `fixtures/survey-logic/` 里的人工期望样例来确认语义一致。

### 6.2 边界约定

导出全是同步函数，入参是 JSON 文本，出参是 `{"ok": …}` 或 `{"error": {code, path, question_id, line?, column?}}`。包装层只做三件事：

1. 用 `json.ts` 的无损编解码把入参压成 JSON 字符串（`encode` / `encodeAnswers`）；
2. 把错误信封还原成 `logic.ts` 里既有的 `LogicError` / `LogicTextError`；
3. 把领域层已规范化为有限 `f64` 的数字还原成 TS 的 `number`（`plainNumbers`）。

`wasm/index.ts` 对外共六个导出：

| 导出 | 用途 |
| --- | --- |
| `parseLogicText(source, questions)` | 解析文本 DSL；语法或语义错误抛出带行列号的 `LogicTextError` |
| `formatLogicText(logic, questions)` | 把规则格式化成文本 DSL，输出与服务端逐字节一致 |
| `compileDocumentLogic(logic, questions)` | 保存前的诊断预检；返回 `variableOrder` 与 `evaluate(answers)` |
| `compileExpression(raw, questions)` | 编译单个表达式，返回 `valueType`、`dependencies` 与 `evaluate(answers)` |
| `renewDocumentIds(document)` | 复制问卷或规则时重写身份并同步全部引用 |
| `FlowSession` | 有状态填写会话：`snapshot()` / `dispatch(event)` / `evaluateAnswers(answers)` |

引擎就绪由模块顶层的 `await loadEngine()` 保证——模块求值结束时引擎已经可用，导入方可以直接同步调用，因此包装层**没有**也需要 `ready` 这样的就绪 Promise 导出。

### 6.3 数字精度与运行环境

- 精度策略：`encodeAnswers` 把已规范化的 JS `number` 用指数形式递给引擎，避免把浮点数的十进制显示值当成新的原始裸整数重新校验；`LosslessNumber` 则保留用户输入原文，超范围或无法精确表示的原始整数仍会被核心拒绝。包装只把领域层已规范化的结果转回 `number`，不会恢复客户端此前已经丢失的精度。
- 浏览器 / Vite 走 wasm-pack 的 bundler 胶水（`pkg/logic_wasm.js`）。
- `node --test` 走 `pkg/logic_wasm_bg.js` 加 `readFileSync` 手动实例化：bundler 产物按打包器契约生成，既不提供 `initSync`，其中的 `.wasm` 静态导入也只有打包器能解析，所以包装层自己提供导入表。
- 入库产物为 `frontend/src/wasm/pkg/` 下的五件套：`logic_wasm.js`、`logic_wasm_bg.js`、`logic_wasm_bg.wasm`、`logic_wasm.d.ts`、`logic_wasm_bg.wasm.d.ts`。

### 6.4 重建产物与测试

改动 `logic-core/` 或 `logic-wasm/` 之后，在 `frontend/` 运行：

```powershell
pnpm run wasm:build
```

它等价于 `wasm-pack build ../logic-wasm --target bundler --release --no-opt`，再由 `scripts/copy-wasm.mjs` 同步这五件套；需要 Rust 工具链、`wasm32-unknown-unknown` target 和 wasm-pack 0.15。**只改前端时直接使用入库产物，不需要 Rust 工具链。**

`frontend/tests/` 下的 `logic`、`logic-text`、`document-logic`、`flow`、`flow-inputs`、`answers`、`editor` 等文件都从 `src/wasm/index.ts` 导入，加载真实入库的 `.wasm`，不使用 TS 镜像或引擎替身；共享样例之外还保留了编辑器操作数切换、规范化数字复用等缺陷回归。注意 Rust 原生目标不导出 WASM 的 `FlowSession` 类，原生测试不能代替这个类的 JS/WASM 边界验证。

按改动选择检查项（完整表格见 [README 第 7 节](../README.md#7-开发检查)）：纯前端逻辑或测试运行相关文件，例如 `pnpm run test:regression tests/editor.test.mjs`；组件或类型改动按需运行 `pnpm run typecheck`；依赖或打包改动用 `pnpm run build`（已含类型检查）。历史验证记录见[需求分析第 9 章](requirements-analysis.md#9-架构演进历次关键决策与验证总结)。
