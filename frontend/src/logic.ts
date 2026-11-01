//! 前后端共用的稳定错误分类与诊断文案；编译与求值已迁到 wasm 引擎。
//! 界面按 `code` 分支、按 `message` 展示，两者都是 UI 契约，不能随实现漂移。

export type LogicErrorCode =
  | 'invalid_syntax'
  | 'unsupported_version'
  | 'limit_exceeded'
  | 'invalid_reference'
  | 'type_mismatch'
  | 'invalid_number'
  | 'invalid_answer'
  | 'arithmetic_error'
  | 'dependency_cycle'
  | 'forward_reference'
  | 'invalid_flow'
  | 'unavailable_dependency'
  | 'invalid_event'
  | 'required_answer'
  | 'constraint_failed'
  | 'unknown_condition'
  | 'inapplicable_answer'
  | 'screened_out'

const messages: Record<LogicErrorCode, string> = {
  invalid_syntax: '规则结构或字段无效',
  unsupported_version: '不支持此规则版本',
  limit_exceeded: '规则超出容量限制',
  invalid_reference: '规则引用的题目或选项无效',
  type_mismatch: '规则的操作数类型不匹配',
  invalid_number: '规则数字无效或无法精确表示',
  invalid_answer: '规则上下文包含无效答案',
  arithmetic_error: '规则计算发生除零或数值溢出',
  dependency_cycle: '计算变量存在循环依赖',
  forward_reference: '题目规则不能依赖自身或后续题目',
  invalid_flow: '流程结构无效、不可达或包含回跳',
  unavailable_dependency: '此依赖可能被分支绕过，请明确处理缺失值',
  invalid_event: '当前填写状态不接受此操作',
  required_answer: '此题为必填项',
  constraint_failed: '答案不符合自定义约束',
  unknown_condition: '条件尚不能确定，请补全相关答案',
  inapplicable_answer: '提交包含隐藏或未进入分支的题目，请核对当前路径',
  screened_out: '筛选结束不产生正式答卷',
}

export class LogicError extends Error {
  readonly code: LogicErrorCode
  readonly path: string
  readonly question_id: string | null

  constructor(
    code: LogicErrorCode,
    path: string,
    questionId: string | null = null,
    message: string = messages[code],
  ) {
    super(message)
    this.name = 'LogicError'
    this.code = code
    this.path = path
    this.question_id = questionId
  }
}

/** 文本 DSL 错误：在领域错误之外附带从 1 开始的行列号，供编辑器定位。 */
export class LogicTextError extends Error {
  readonly code: LogicErrorCode
  readonly line: number
  readonly column: number
  readonly path: string
  readonly question_id: string | null
  constructor(
    code: LogicErrorCode,
    line: number,
    column: number,
    path: string,
    question_id: string | null,
    message: string = messages[code],
  ) {
    super(`${message}（第 ${line} 行，第 ${column} 列）`)
    this.name = 'LogicTextError'
    this.code = code
    this.line = line
    this.column = column
    this.path = path
    this.question_id = question_id
  }
}
