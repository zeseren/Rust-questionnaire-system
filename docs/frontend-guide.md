# 前端开发指南

`frontend/` 是问序（Wenxu）的 Vue 3 前端工作台，基于 **Vue 3 + TypeScript + Vite + Pinia + Vue Router** 构建，包管理器固定为 **pnpm**（锁文件 `frontend/pnpm-lock.yaml`）。前端通过入库的 WebAssembly 产物（`src/wasm/pkg/`）直接调用 Rust `logic-core` 规则与流程引擎，并采用 **Editorial Craft（纸本与松烟墨）** 视觉设计体系。

---

## 1. 核心业务流程与路由

| 路由路径 | 视图组件 | 核心流程与职责 |
| --- | --- | --- |
| `/`、`/surveys`、`/archive` | `DashboardView.vue` | **工作台与案卷库**：展示核心指标账本栏（全部、回收中、草稿、已归档）、案卷列表/卡片视图切换、按状态与标题搜索、新建问卷（空白或三套内置规则模板）、复制问卷 |
| `/login`、`/register` | `AuthView.vue` | **认证入口**：普通用户注册、全角色统一登录（管理员登录后侧边栏自动展示「平台管理」入口），支持安全站内跳转 `redirect` |
| `/surveys/:id/edit` | `EditorView.vue` | **四工作区编纂台**：<br>1. **题目设计**：问卷标题、说明、收集目的、结束语及六类题型配置；<br>2. **规则与跳题**：可视化条件/变量/页面流程编辑与松烟墨文本 DSL 控制台双向切换；<br>3. **试填预览**：基于内存会话直接试填当前未保存草稿；<br>4. **回收设置**：填写模式（公开匿名 / 登录限次）、截止时间、发布、关闭/恢复、归档、删除草稿与分享链接复制 |
| `/surveys/:id/preview` | `PreviewView.vue` | **服务端草稿预览**：读取服务端已保存草稿与发布校验结果，支持桌面/手机宽度切换，不产生正式答卷 |
| `/s/:id` | `FillView.vue` | **受访者沉浸填写页**：公开匿名直接作答，登录限次模式先认证；有规则问卷通过 WASM `FlowSession` 分步渲染、回退自动清理失效分支答案、完成后进入核对页确认提交；内置 UUIDv4 幂等重试保护 |
| `/surveys/:id/reports` | `ReportsView.vue` | **分版本报表与导出**：按发布版本与北京时间 `[from, to)` 范围筛选；展示回收汇总、自然日趋势、各题三态计数（已答 / 适用未答 / 规则跳过）、选项占比、评分分布、数值统计、答卷明细弹窗与 CSV 下载 |
| `/admin` | `AdminView.vue` | **平台治理**（仅管理员）：用户分页列表与禁用/启用、问卷治理列表与下架/恢复（可查看当前已发布题面，不暴露他人答卷）、操作审计日志列表 |
| `/settings` | `SettingsView.vue` | **账户设置**：查看账户角色、切换外观主题（浅色 / 深色 / 跟随系统）、修改密码（成功后全设备会话失效并跳转登录） |
| `/:pathMatch(.*)*` | `NotFoundView.vue` | **404 页面**：提供返回工作台出口 |

> 前端路由守卫（`router.ts`）仅控制界面导航体验，所有接口的数据归属与角色权限均由后端严格裁决。

---

## 2. 本地运行、代理与构建

| 操作 | 命令 / 配置 | 说明 |
| --- | --- | --- |
| 安装依赖 | `pnpm install --frozen-lockfile` | 严格遵循 `pnpm-lock.yaml`，禁止使用 npm/yarn |
| 启动开发服 | `pnpm run dev` | 固定监听 `http://localhost:5173`（`strictPort: true`） |
| 后端同源配置 | 根目录 `.env` 设 `PUBLIC_ORIGIN=http://localhost:5173` | Vite 将 `/api` 代理至后端（默认 `http://127.0.0.1:3000`）并保留原始 `Origin` 与 Cookie |
| 自定义代理目标 | `frontend/.env` 设 `API_PROXY_TARGET=http://...` | 仅由 Vite 服务端读取，不暴露给浏览器包 |
| 类型检查与构建 | `pnpm run build` | 依次执行 `vue-tsc --noEmit` 与 `vite build`，产物输出至 `frontend/dist/` |
| 本地预览构建产物 | `pnpm run preview` | 需先停止 `dev`，同样监听 `http://localhost:5173` 并代理 `/api` |
| 回归测试与格式化 | `pnpm run test:regression`、`pnpm run format:check` | Node 内置测试运行器递归执行 `tests/{contract,unit,integration}/` 下的分层测试；Prettier 检查代码风格 |

---

## 3. 目录结构与模块职责

```text
frontend/
├── index.html                  # 应用入口与首绘前主题设置脚本（同步 data-theme 与 theme-color）
├── vite.config.ts              # 固定 5173 端口与 /api 同源代理配置
├── scripts/copy-wasm.mjs       # 将 wasm-pack 产物五件套同步到 src/wasm/pkg/
├── tests/                      # 分层测试：contract/ 跨端契约、unit/ 纯函数与基础设施、integration/ 跨模块流程
│   └── helpers/                # fixtures.mjs 夹具缓存、documents.mjs 固定身份构造器、browser.mjs 环境桩
└── src/
    ├── main.ts / App.vue       # 应用挂载、左侧编辑部导航栏、顶栏、全局提示与会话过期弹窗
    ├── router.ts / stores.ts   # 路由守卫与 Pinia 状态（useAuth、useAppearance、useNotice）
    ├── api.ts / resource.ts    # Fetch 封装（含 CSRF 头与覆盖正文读取的 30s 超时）、带序号防竞态的异步资源加载器
    ├── json.ts / types.ts      # JSON 编解码助手与完整 API / 领域 TypeScript 类型定义
    ├── value-contract.ts       # Unicode 空白判定（U+0085 为空白、U+FEFF 为内容）与标量计数校验
    ├── numbers.ts / format.ts  # 数字输入清洗与本地格式提示；北京时间格式化、状态名与分享链接
    ├── documents.ts / answers.ts # 新题与复制题身份生成、文本上限常量、无规则问卷本地作答校验
    ├── survey-templates.ts     # 三套内置规则问卷模板（满意度回访、产品体验调研、活动报名）
    ├── logic.ts                # LogicError / LogicTextError 错误类型与中文文案映射
    ├── editor-context.ts / editor-logic.ts # 编辑器禁用态注入与引擎编译错误到 UI 节点的定位映射
    ├── condition-builder.ts    # 可视化简洁条件与表达式 AST 双向转换（同类运算符切换保留原操作数）
    ├── flow-tools.ts / flow-inputs.ts # 页面流程节点插入（自动回接后继出口）与填写会话输入适配器
    ├── logic-text-tools.ts     # 文本 DSL 签名比对、可读别名绑定提取与 Unicode 行列偏移换算
    ├── wasm/                   # index.ts 引擎包装层与 pkg/ 入库 WASM 五件套
    ├── styles.css              # Editorial Craft 设计令牌、字体阶梯、侧边栏、刊头与概览卡片样式
    ├── application.css         # 表单控件、面板、公开填写纸张舞台、报表图表、治理表格与认证页样式
    ├── editor.css              # 四工作区切换栏、可视化规则卡片、流程概览节点与松烟墨 DSL 控制台样式
    ├── views/                  # 9 个路由页面组件
    └── components/             # 19 个业务与基础 UI 组件
```

---

## 4. 状态一致性与关键交互设计

### 4.1 文本长度与数字语义

- **Unicode 标量计数**：文本长度按 `[...text].length`（Unicode 标量值）统计，单行文本默认上限 200、多行文本默认上限 1000（发布者可按题调低）。`value-contract.ts` 与后端共享同一空白标准（`U+0085` 视为空白，`U+FEFF` 视为有效字符），拒绝含 `U+0000` 或孤立代理项的输入。
- **有限 `f64` 数字**：前后端统一使用原生 JSON 数字与标准 JS `number`（IEEE 754 binary64 有限值），`1` 与 `1.0`、正零与负零等价（负零归一为 `0.0`）。

### 4.2 草稿修订号（`revision`）与冲突保护

- `EditorView.vue` 独立维护当前已加载草稿的 `revision`。修改回收设置（`PATCH /collection`）不会覆盖本地草稿修订号，防止掩盖其他窗口的并发草稿修改。
- 当保存或发布遇到 `409 revision_conflict` 时，界面不会丢弃当前未保存内容，而是提供 **「下载当前草稿（JSON）」** 与 **「重新加载服务器最新内容」** 两个显式操作。

### 4.3 可视化规则与松烟墨文本 DSL 双向同步

- 可视化配置（`LogicEditor.vue`）与文本 DSL（`LogicTextEditor.vue`）共享同一份 `DocumentLogic` 模型。
- 在文本 DSL 模式下，CodeMirror 6 编辑器延迟 300ms 调用 WASM `parseLogicText` 进行实时语法与语义诊断，错误精确定位到行、列与 AST 路径。
- **未应用保护**：文本缓冲区存在未应用的修改或语法错误时，不会覆盖当前规则模型，但会阻止误保存草稿或切回可视化模式，并在离开页面前触发提醒。

### 4.4 填写会话与幂等提交重试

- **WASM 会话驱动**：带规则的问卷在填写页与预览页共用 `FlowFields.vue` + `FlowInputs` + WASM `FlowSession`。修改答案（`change`）会从入口重算有效路径并立即清理被跳过分支上的旧输入；抵达 `complete` 终点后展示核对清单，须显式确认才发起提交。
- **冻结载荷与幂等重放**：点击提交时生成一次 `crypto.randomUUID()` 作为 `idempotency_key`。若遇到网络中断、30 秒读取超时或 `5xx` 错误，页面将**冻结**原版本、原答案与原幂等键，仅展示「重试原提交」按钮（`login_once` 模式还会校验重试账号与首次提交账号一致），杜绝超时重发产生重复答卷。
- **版本更新保护**：若提交时返回 `409 version_changed`，页面保留用户已填内容并提示在新窗口打开最新题面核对，绝不把旧答案静默映射到新版本。

---

## 5. 视觉规范（Editorial Craft · 纸本与松烟墨）

界面设计以 `frontend/editorial-prototype.html` 为视觉基线，样式分为 `styles.css`、`application.css`、`editor.css` 三层：

1. **纸墨基底与经纬纹理**：
   - 宣纸浅色（`--bg-canvas: #f5f2eb`、`--bg-surface: #fdfcfa`、`--bg-subtle: #ede9df`）与玄墨深色（`--bg-canvas: #121210`、`--bg-surface: #1a1917`、`--bg-subtle: #22211e`）。
   - `body` 叠加 `32px × 32px` 极淡线性网格模拟编辑部稿纸秩序；`index.html` 首绘脚本在首次渲染前即同步 `data-theme` 与 `<meta name="theme-color">`，避免深色模式闪白。
2. **三级字体阶梯**：
   - `--font-serif`（中文衬线体）：用于品牌标题、页面刊头 `h1`、面板标题 `h2`、指标大数字及问卷题干。
   - `--font-mono`（等宽 `tabular-nums`）：用于眉题（`.eyebrow`）、案卷编号、修订号、回执编号、统计数值、时间戳及 DSL 代码。
   - `--font-sans`（无衬线体）：用于界面正文、表单输入与操作按钮。
3. **编辑部语义色盘**：
   - **朱砂印泥**（`--accent: #c84b31` / 深色 `#e4684e`）：品牌方印标志「问」、主操作按钮、必填徽章、选中项高亮与问卷纸张顶部标尺。
   - **松石绿**（`--jade: #1d634d` / 深色 `#46b593`）：「回收中」状态签、「WASM 已就绪」状态点、规则校验通过提示与流程结束节点。
   - **靛青**（`--indigo: #294c7a` / 深色 `#76a1de`）：计算变量、已绑定规则提示条、流程条件分支节点与报表趋势柱图。
   - **琥珀**（`--amber: #945d14` / 深色 `#e2a54f`）：草稿状态签与规则诊断提示。
   - **松烟墨控制台**（`--bg-ink-panel: #191816` / 深色 `#0d0d0c`）：专用于文本 DSL 代码工作台，配朱砂/靛青/松石绿语法高亮。
4. **响应式与无障碍**：
   - 覆盖 `1600px / 1190px / 980px / 900px / 760px / 540px` 断点，在 `375px` 手机视口下零横向溢出；支持键盘焦点环、`prefers-reduced-motion` 与 `forced-colors` 高对比度模式。
5. **交互与操作布局规范（Interaction & Action Placement Rules）**：
   - **对象与操作就近绑定**：实体级管理操作（删除草稿、重命名、导出、切换状态）绑定在入口卡片行（List Item Row）或编辑器顶部固定栏（Sticky Top Header）；禁止塞进分步表单的最后一步或页面流式末尾。
   - **全流程随时可退**：多步向导与长表单中，「保存草稿」「丢弃/删除草稿」「取消返回」属于全局逃生口，必须在每一步的顶栏或固定底栏左侧常驻可见，不能要求用户翻到流程终点才能删除或取消。
   - **主次与破坏性操作分离**：每个视图只允许 1 个最高视觉权重的主按钮（Primary CTA，置于主操作区右侧）；破坏性操作必须与主提交在空间上拉开距离（顶栏更多菜单、卡片操作列或固定栏最左端），使用低饱和度幽灵按钮（`button.text-button.danger-text`、`.ghost-danger` 等），静止态为 `--text-muted`，`:hover`/`:focus-visible` 时变为 `--rose` 与 `--rose-soft` 背景。
   - **写代码前先过动线自检**：生成任何带状态界面前，先用三行注释写明 `[入口在哪] -> [用户在第 1 步后悔了怎么删/退] -> [主提交通道在哪]`，并保留在对应 `.vue` 文件顶部第 1–3 行。

---

## 6. WASM DSL 引擎接入与重建

`frontend/src/wasm/index.ts` 在模块顶层通过 `await loadEngine()` 完成 WASM 实例化，对外同步暴露 6 个接口：

| 导出项 | 职责 |
| --- | --- |
| `parseLogicText(source, questions)` | 将文本 DSL 解析并编译为 `DocumentLogic`；失败抛出带行列号的 `LogicTextError` |
| `formatLogicText(logic, questions)` | 将 `DocumentLogic` 格式化为确定性文本 DSL（与后端逐字节一致） |
| `compileDocumentLogic(logic, questions)` | 校验整份规则模型，返回变量拓扑顺序 `variableOrder` 与求值器 |
| `compileExpression(raw, questions)` | 编译单条表达式，返回静态类型 `valueType`、依赖集合与求值函数 |
| `renewDocumentIds(document)` | 复制问卷时重写题目、选项、变量、规则与流程节点的全部 UUID 引用 |
| `FlowSession` | 有状态流程会话类：支持 `snapshot()`、`dispatch(event)` 与 `evaluateAnswers(answers)` |

- **双环境兼容**：Vite 浏览器构建使用 `pkg/logic_wasm.js` bundler 胶水；Node 测试环境（`node --test`）通过 `pkg/logic_wasm_bg.js` 配合 `readFileSync` 加载本地 `logic_wasm_bg.wasm`。
- **重建命令**：修改 `logic-core/` 或 `logic-wasm/` 后，在 `frontend/` 下运行 `pnpm run wasm:build` 即可重新编译并更新 `src/wasm/pkg/` 下的五件套入库产物。

---

## 7. 测试结构与约定

`frontend/tests/` 按依赖边界分层，`pnpm run test:regression`（`node --test`）会递归执行全部 `*.test.mjs`：

| 目录 / 文件 | 覆盖内容 |
| --- | --- |
| `tests/contract/` | 与 Rust 共读 `fixtures/survey-logic/` 的跨端语义契约：`documents`、`expressions`、`flows`、`input-contracts`、`submissions`、`texts` |
| `tests/unit/`（17 个模块） | 纯函数与基础设施契约：`value-contract`、`numbers`、`json`、`format`、`logic-errors`、`answers`、`documents`、`condition-builder`、`flow-tools`、`editor-logic`、`survey-templates`、`logic-text-tools`、`api`、`resource`、`stores`、`router`、`wasm-envelope` |
| `tests/integration/` | 跨模块流程：`editor-workflow`、`flow-inputs`，组合编辑器工具、`FlowSession` 与页面级输入适配 |
| `tests/helpers/fixtures.mjs` | `fixtures/survey-logic/` 的进程内只读缓存与命名夹具读取 |
| `tests/helpers/documents.mjs` | 固定 v4 UUID 的问卷/题目构造器，断言可直接引用常量 |
| `tests/helpers/browser.mjs` | 浏览器环境桩：`installBrowserEnv(t, options)` 按需安装 `window`/`document`/`localStorage`/`matchMedia`/`history`/`fetch`/`URL` 与定时器 |

其中 `unit/api`、`unit/resource`、`unit/stores`、`unit/router`、`unit/wasm-envelope` 分别覆盖传输契约与下载链路、加载序号竞态与卸载作废、Pinia 状态迁移、开放重定向防护与守卫/`afterEach` 行为、WASM 信封错误还原。

约定：

- 单元测试直接导入 `src/*.ts`（依赖 Node 原生类型擦除），不启动 Vite、不打包，也不 mock 被测模块本身；外部边界（`fetch`、本地存储、媒体查询、定时器）统一由环境桩替换。
- `installBrowserEnv(t, options)` 通过 `t.after(restore)` 自动卸载，禁止在测试间遗留全局变量；需要同步断言 30 秒截止或对象地址释放时传入 `timers: 'manual'`。
- 契约测试只断言与 Rust 共读夹具的语义；跨模块组合行为放进 `tests/integration/`，避免重复覆盖同一代码路径。
