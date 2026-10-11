# dsl-survey-editor 分支 Code Review 报告

- **对比基准**：`main`（本地分支 `dsl-survey-editor`，HEAD 48eab76）
- **变更规模**：40 文件，+3782 / -1080
- **Review 方式**：7 个并行子代理（6 个代码质量切片 + 1 个安全全量切片），静态阅读 diff 与分支实际代码
- **日期**：2026-10-09

> **严重级别说明**：P1 = Major（合并前建议修复）、P2 = Minor、P3 = Nit。未发现 Critical。

## 总览

| 切片 | 结论 |
|---|---|
| 前端核心（EditorView/模板系统） | 正确，2×P3 |
| 前端组件（FlowEditor 等 6 个新组件） | 1×P1、1×P2、4×P3 |
| 前端逻辑（LogicEditor/文本同步） | 1×P2、1×P3 |
| 前端测试与样式 | 1×P2、5×P3 |
| 后端测试精简 + fixtures | 1×P1、2×P2、1×P3 |
| 文档与依赖 | 1×P2（分支名笔误） |
| 安全 | 无新引入安全问题，2 条信息级备注 |

## P1 — Major（合并前建议修复）

### 1. FlowEditor.addNode 未回接分支节点的 branches[].target

- 位置：`frontend/src/components/FlowEditor.vue:54-55`
- 触发路径：节点序列 [P1, B, P2, E]，分支 B 的条件出口与 otherwise 都指向 P2；在 B 后插入新页面 Px 后，otherwise 被改为 Px（Px.next=P2），但条件分支的 target 仍指向 P2。
- 后果：命中条件的填写者静默跳过 Px；logicDiagnostic/compileFlow 均不报错（所有出口仍指向存在的节点，Px 有 otherwise 入边），属于无诊断提示的静默分流偏差。
- 建议：插入节点时与 otherwise 同样回接 branches[].target。

### 2. validate_copy_draft 失去全部回归覆盖

- 位置：`backend/tests/domain.rs`（删除原 311-321 等段）
- 事实：生产复制端点 `backend/src/surveys.rs:806` 仍调用 `renew_ids()` + `validate_copy_draft()`；该函数承载独立契约——容量收紧前发布的历史问卷（文本题 max_length 可达 5000）必须仍能复制为可编辑草稿，超过历史容量（5001）必须拒绝。与 validate_draft/validate_publish（200/1000）不是同义重复。
- 后果：分支上该函数测试覆盖归零；若后续改动 limit 传参或把复制校验并入当前上限，生产复制/作答路径会在无测试变红的情况下破坏。
- 建议：补回一处精简回归——历史容量文档 validate_draft/validate_publish 拒绝、validate_copy_draft 接受、超历史容量拒绝。

## P2 — Minor

### 3. 逻辑失效后文本模式形成恢复死径（分支引入回归）

- 位置：`frontend/src/components/LogicTextEditor.vue:92-96`
- 触发：进入文本 DSL 模式 → 切到题目工作区删除被规则/流程引用的题目 → 返回逻辑工作区。questionSignature 变化且 formatLogicText 抛 invalid_reference → stale=true → pending 恒真。
- 后果：保存被阻止（合理），但「可视化配置」按钮（LogicEditor.vue:111）、「移除全部逻辑」（:348）均被禁用，「恢复当前规则文本」因 generate() 抛错走不通。用户只剩手工删文本中的失效引用或重载页面丢未保存修改。main 上同场景可自由切回可视化。
- 建议：「可视化配置」只在确有未应用文本编辑（text!==baseline）时禁用；或 restore() 失败时提供「放弃缓冲区并返回可视化」分支。

### 4. 入口页面选择器提供必然无效的选项

- 位置：`frontend/src/components/FlowEditor.vue:128-132`
- 事实：编译器（flow-compiler.ts:105）强制 entry = 首节点且为页面；诊断也报「入口必须是第一个页面」。选择器却列出全部页面，非首节点选项选中即报错、预览「流程无法加载」，无合法终态。
- 建议：非首节点选项加 disabled（SearchSelect 已支持），或改只读展示。

### 5. compileExpression 结果隔离与 dependencies 去重失去唯一直接覆盖

- 位置：`frontend/tests/logic.test.mjs`（删除原 39-41 段用例）
- 事实：dependencies 是生产消费接口（document-logic.ts:208/229/254、flow-compiler.ts:186 用于变量顺序与分支保护）；现有文档级测试只间接读 variableOrder，断言不到去重。
- 建议：保留最小用例断言 compiled.dependencies 去重 + 编译后变异源配置不影响 evaluate。

### 6. 后端两处覆盖缺口

- `backend/tests/domain.rs`：ensure_submission_budget 拒绝分支不再有直接测试（仅剩 domain.rs:360 成功路径）；建议在「满题数问卷」用例中补超预算断言（question_id 为 None）。
- 同文件：`Document.logic` 的 `#[serde(default, skip_serializing_if)]`「旧文档序列化不增加字段」契约失去唯一回归；建议在现有往返测试中断言序列化结果无 logic 键。

### 7. 文档分支名笔误

- 位置：`docs/requirements-analysis.md:521`
- §21 写「实施分支：feat/dsl-survey-editor」，实际分支名为 `dsl-survey-editor`（本地与 origin 均无 feat/ 前缀）。

## P3 — Nit（择要）

- `ConditionBuilder.vue:56-64`：切换同族比较运算符（大于→小于）无条件重置已输入比较值；建议同族（eq/ne/gt/ge/lt/le 之间）沿用旧值。
- `ConditionBuilder.vue:33-39`：切到「始终成立/不成立」静默丢弃全部已配条件，切回不可恢复；建议暂存 conditions。
- `ConditionBuilder.vue:85-93`：简洁/高级视图随表达式可解析性在编辑中途自动跳变，建议对强制高级模式加锁存。
- `DraftPreview.vue:71`：无流程路径改答后红色错误提示不清除（只清 feedback），与 FlowFields 路径不一致。
- `survey-templates.ts:121-122`：模板工厂末尾 else 分支缺穷尽性保护（建议 else if + never 断言）。
- `survey-templates.ts:29` + `editor.css`：registration 模板 color:'blue' 缺 `.template-icon.blue` 类，图标回退默认配色，三卡片视觉不一致。
- `LogicTextEditor.vue:231-232`：引用列表恰好 60 条时误显示「显示前 60 项」截断提示；应以过滤后总数 > 60 为条件。
- `LogicTextEditor.vue:117-140`：导入 ~1 MiB DSL 文件时主线程同步解析短暂卡顿（自致、有界：UI 1MiB + 解析器 1MiB/65536 token/深度 64 预算）。
- `editor.css:40、252、442、290`：四处硬编码圆角（4px / 8px 8px 0 0）未走 --radius-control / --radius-card 令牌。
- `backend/tests/logic_text.rs:69-79`：合并后的长链预算用例把 20000 长数字链挂在 `let v: boolean` 下（原为 number），依赖解析期预算先于类型检查才成立；建议恢复 number 前缀或加注释。
- `frontend/tests/flow-inputs.test.mjs:19-21`：65 位超长裸整数用例被移除（解析路径与保留用例相同，损失小）。
- `frontend/tests/document-logic.test.mjs:31-34`：duplicateSurveyDocument 无 logic / 无 flow 两分支失去唯一覆盖；该函数本分支无生产调用方（整卷复制走后端），可考虑删除死代码。
- `frontend/tests/logic-text.test.mjs:45`：过滤条件用 expected.value 真值当「可编译」代理，建议改 !item.expected.error。
- `Icons.vue:131-132`：i-settings 滑块圆点 fill="var(--surface)" 依赖呈现属性 var() 支持（现代浏览器均支持，仅旧引擎回退黑色）。

## 安全 Review（无新引入安全问题）

九个攻击面全部核查闭合：

1. **XSS**：无 v-html/innerHTML/document.write/srcdoc；内容均经 Vue 插值自动转义；Icon `:href="#i-${name}"` 仅引用静态常量精灵图。
2. **动态样式**：新增 :style 绑定均为服务端数值夹取后的纯数值 width/height。
3. **URL 拼接**：shareLink 用 location.origin + encodeURIComponent；_blank 链接均带 rel="noopener"；download 属性仅命名。
4. **敏感信息**：无 console.* 输出；无令牌落盘。
5. **localStorage**：仅存 'wenxu-theme' 且白名单校验；无 sessionStorage/document.cookie 操作。
6. **postMessage/事件监听**：无 postMessage；session-expired 自定义事件与 beforeunload 均成对解绑。
7. **CSRF/会话**：api.ts 请求层未改动，X-CSRF-Protection 与 credentials: same-origin 保持。
8. **开放重定向**：router.ts safeReturn 限制站内路径并排除 /login、/register。
9. **文件导入**：.dsl 导入 1 MiB 上限、解析器多层预算（1MiB 字节/65536 token/深度 64）、纯正则无回溯灾难、仅用户自选文件。

**依赖供应链**静态核查通过：@codemirror/* 7 包 + @lezer/highlight + element-plus 2.14.7 全部精确版本锁定，lockfile 全带 sha512 integrity，无 git/tarball/link 来源、无 patchedDependencies/overrides、无安装期脚本执行面（pnpm 12 默认拦截构建脚本）。离线环境无法比对 CVE 公告库，可补跑 `pnpm audit`。

## 其余结论（未发现问题）

- 文档与命令记录核实无误（除 #7 分支名）：package.json 9 个新依赖全部实际使用、lockfile 同步、README/frontend-guide 命令有效、§21/§22 数字准确、锚点有效。
- fixtures/survey-logic 的 old→plain 重命名全仓一致，无残留 "old" 引用，语义与后端一致。
- flow.rs 删除的 entry 不等断言被逐节点 contains 涵盖；logic.rs 删除的编译器实现细节测试合理；数值精度测试重写等价且更严（全部从 JSON 原文解析）。
- 组件 props/emits 契约与消费者完全吻合；失效引用（删题目/删节点）经显式占位与「移除失效题目引用」暴露并由诊断定位；可访问性（aria-label、sr-only、aria-pressed、role=status）到位。
- 可视化↔文本双向同步签名机制推导正确：题目改名、改选项标签不误判 stale，结构/语义变更正确置位；textErrorOffset 码点→UTF-16 换算与解析器一致。
- editor.test.mjs 新增 5 用例均测真实行为；editor.css 100 个类均有使用方、颜色全令牌化且明暗双主题有定义。
