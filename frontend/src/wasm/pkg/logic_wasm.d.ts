/* tslint:disable */
/* eslint-disable */

/**
 * 有状态的填写会话：文档与状态留在 wasm 线性内存，避免每次改答重编译。
 *
 * 核心的 `FlowSession` 在构造时接管 `Document` 的所有权，导出类只需持有
 * 会话本身，借用生命周期与指针稳定性都由核心类型自行保证。
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
     * 解析文档、接管所有权并创建空会话；失败时抛出携带 `{"error":…}` 信封 JSON
     * 的字符串（与其它导出同一信封形状）。
     */
    constructor(document_json: string);
    /**
     * 只读快照，序列化为 `{"ok":snapshot}`；借用会话集合，避免每次改答深拷贝。
     */
    snapshot(): string;
}

/**
 * 保存前的诊断预检：编译整份规则，只关心错误里的 `path` / `question_id`。
 *
 * 成功信封同时返回 `variable_order`：计算变量的拓扑顺序，供前端在编译期展示
 * 与断言使用。
 */
export function compile_document_logic(logic_json: string, questions_json: string): string;

/**
 * 把规则 JSON 格式化回文本 DSL。
 */
export function format_logic_text(logic_json: string, questions_json: string): string;

/**
 * 巡检文本 DSL：成功信封返回 `{bindings,error,error_offset,line,column}`。
 *
 * 解析失败时错误保留在成功值内部，而不是整体变成 error 信封，这样前端右侧速查栏
 * 可以同时展示已扫描到的绑定；只有 `questions_json` 本身非法才返回边界错误。
 */
export function inspect_logic_text(source: string, questions_json: string): string;

/**
 * 解析文本 DSL；失败信封额外携带 `line` / `column`。
 */
export function parse_logic_text(source: string, questions_json: string): string;
