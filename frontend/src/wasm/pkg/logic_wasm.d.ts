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
 * 1. 堆分配地址稳定：移动 `Box<Document>` 只搬移指针而不移动堆上的 `Document`，
 *    且本 crate 从不重写或提前释放 `document` 字段；
 * 2. 字段声明顺序 `session` 在 `document` 之前，Drop 时先释放会话再释放文档，
 *    借用永远不悬空；
 * 3. 会话只在 `new` / `dispatch` / `snapshot` / `evaluate_answers` 内使用，
 *    期间 `self` 可变/不可变借用始终有效。
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
     * 从最终答案重建整份答卷：不读取也不修改会话当前的历史与输入。
     *
     * 复用核心 `FlowEvaluator::evaluate_answers`（与提交路径同一段代码），因此
     * 到达 complete 只代表可提交，不代表已经保存。
     *
     * # 错误
     * 未知/重复题目、不适用答案、校验失败、未决条件或筛选结束时返回 error 信封。
     */
    evaluate_answers(answers_json: string): string;
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
 *
 * 成功信封同时返回 `variable_order`：计算变量的拓扑顺序，供前端在编译期展示
 * 与断言使用（`evaluate_document_logic` 只负责按答案求值）。
 */
export function compile_document_logic(logic_json: string, questions_json: string): string;

/**
 * 编译单个表达式包：`{"value_type":…,"dependencies":[…]}`，供表达式编辑器提示。
 */
export function compile_expression(raw_json: string, questions_json: string): string;

/**
 * 整份规则的求值：编译后按答案上下文计算全部计算变量与规则结果。
 *
 * # 错误
 * 编译失败或求值中出现类型、引用、除零、溢出错误时返回 error 信封。
 */
export function evaluate_document_logic(logic_json: string, questions_json: string, answers_json: string): string;

/**
 * 单个表达式求值：入参与 `compile_expression` 一致，另收已按问卷规则规范化的答案上下文。
 *
 * 计算变量的解析需要回调，跨 JSON 边界无法表达；因此与核心
 * `CompiledExpression::evaluate` 一样只接受答案，变量引用在编译期即被拒绝。
 *
 * # 错误
 * 入参非法、编译失败或引用答案的类型/数值不合法时返回 error 信封。
 */
export function evaluate_expression(raw_json: string, questions_json: string, answers_json: string): string;

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
