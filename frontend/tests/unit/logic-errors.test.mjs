//! 领域错误：错误码、中文文案与文本定位后缀都是界面依赖的稳定契约。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicError, LogicTextError } from '../../src/logic.ts'

const messages = {
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
  invalid_flow: '流程结构或题目归属无效',
  unavailable_dependency: '规则必须显式处理分支答案缺失',
  invalid_event: '当前状态不允许此事件',
  required_answer: '请回答当前必填题目',
  constraint_failed: '答案不满足自定义约束',
  unknown_condition: '条件尚不能确定',
  inapplicable_answer: '提交包含隐藏或未进入分支的题目，请核对当前路径',
  screened_out: '筛选结束不产生正式答卷',
}

test('每种错误码映射唯一中文文案并保留定位', () => {
  assert.equal(Object.keys(messages).length, 18)
  for (const [code, message] of Object.entries(messages)) {
    const error = new LogicError(code, '$.logic.rules[0]')
    assert.equal(error.name, 'LogicError')
    assert.ok(error instanceof Error)
    assert.equal(error.code, code)
    assert.equal(error.message, message)
    assert.equal(error.path, '$.logic.rules[0]')
    assert.equal(error.question_id, null, '默认不关联题目')
  }
})

test('LogicError 透传关联题目', () => {
  const error = new LogicError('required_answer', '$.answers', 'question-1')
  assert.equal(error.question_id, 'question-1')
  assert.equal(error.message, messages.required_answer)
})

test('LogicTextError 追加从 1 开始的行列后缀', () => {
  const error = new LogicTextError('invalid_syntax', 3, 7, '$.logic', 'question-1')
  assert.equal(error.name, 'LogicTextError')
  assert.ok(error instanceof Error)
  assert.equal(error.code, 'invalid_syntax')
  assert.equal(error.line, 3)
  assert.equal(error.column, 7)
  assert.equal(error.path, '$.logic')
  assert.equal(error.question_id, 'question-1')
  assert.equal(error.message, '规则结构或字段无效（第 3 行，第 7 列）')
})

test('LogicTextError 保留解析器给出的自定义消息', () => {
  const error = new LogicTextError('invalid_reference', 1, 2, '$.logic', null, '未知别名 choose')
  assert.equal(error.message, '未知别名 choose（第 1 行，第 2 列）')
  assert.equal(error.code, 'invalid_reference')
  assert.equal(error.question_id, null)
})
