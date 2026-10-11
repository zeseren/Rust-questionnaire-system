/* tslint:disable */
/* eslint-disable */

/**
 * 有状态的填写会话：文档与状态留在 wasm 线性内存，避免每次改答重编译。
 *
 * 核心的 `FlowSession<'a>` 借用 `&'a Document`，而 wasm-bindgen 导出类只能持
 * owned 字段，两者不能直接组合。这里的取舍：文档放进 `Box<Document>`（地址
 * 稳定、随会话 Drop 一并释放，不使用 `Box::leak`——那会让每次创建会话都
 * 永久泄漏一份文档），再把该引用的生命周期安全地延伸为 `'static` 交给核心
 * 会话。安全性由三条不变量保证：
 *
 * 1. `Box` 分配后地址不再变化：`FlowSession` 本身被 wasm-bindgen 放在堆上且
 *    本 crate 从不移动 `document` 字段（无 `mem::replace`/移动构造）；
 * 2. 字段声明顺序 `session` 在 `document` 之前，Drop 时先拆掉会话再释放文档，
 *    借用永远不悬空；
 * 3. 会话只在 `new` / `dispatch` / `snapshot` 内使用，期间 `self` 可变/不可变
 *    借用始终有效。
 *
 * 仅在 wasm32 目标导出：wasm-bindgen 的类导出 wrapper 在原生目标会 panic
 * （“function not implemented on non-wasm32 targets”），纯函数无此限制。
 */
export class FlowSession {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * 应用一个流程事件；事件 JSON 反序列化失败或流程拒绝时返回 error 信封。
     */
    dispatch(event_json: string): string;
    /**
     * 绑定文档并创建空会话；失败时返回 JSON 错误字符串（非信封，见计划约定）。
     */
    constructor(document_json: string);
    /**
     * 只读快照，序列化为 `{"ok":snapshot}`。
     */
    snapshot(): string;
}

/**
 * 保存前的诊断预检：编译整份规则，只关心错误里的 `path` / `question_id`。
 */
export function compile_document_logic(logic_json: string, questions_json: string): string;

/**
 * 编译单个表达式包：`{"value_type":…,"dependencies":[…]}`，供表达式编辑器提示。
 */
export function compile_expression(raw_json: string, questions_json: string): string;

/**
 * 把规则 JSON 格式化回文本 DSL。
 */
export function format_logic_text(logic_json: string, questions_json: string): string;

/**
 * 解析文本 DSL；失败信封额外携带 `line` / `column`。
 */
export function parse_logic_text(source: string, questions_json: string): string;

/**
 * 重写整份问卷的身份（复制问卷/规则时使用）。
 */
export function renew_document_ids(document_json: string): string;
