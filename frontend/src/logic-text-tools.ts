//! 文本工作台的展示辅助：绑定名称与行列到 UTF-16 偏移换算。
//! 绑定扫描与语法诊断都由 wasm 引擎的 `inspect_logic_text` 一次完成，前端不再镜像文法。
import type { Question } from './types.ts'
import type { LogicTextBinding } from './wasm/index.ts'

/** 绑定加当前问卷里的可读名称，供引用速查列表展示。 */
export interface TextBinding extends LogicTextBinding {
  label: string
}

/** 绑定身份在当前问卷中的名称；已删除或未命名的身份给出明确占位。 */
export function textBindingLabel(
  binding: LogicTextBinding,
  questions: readonly Question[],
): string {
  if (binding.category === 'variable') return '计算变量'
  if (binding.category === 'node') return '流程节点'
  if (binding.category === 'question')
    return questions.find((question) => question.id === binding.id)?.title || '未命名或已删除题目'
  for (const question of questions) {
    if (!('options' in question)) continue
    const option = question.options.find((item) => item.id === binding.id)
    if (option) return option.label || '未命名选项'
  }
  return '未命名或已删除选项'
}

/** 把 1 起始的行列号换算为 JS 字符串（UTF-16）偏移，供代码编辑器定位。 */
export function textErrorOffset(source: string, line: number, column: number): number {
  const lines = source.split('\n')
  const row = line - 1
  const start = lines.slice(0, row).reduce((length, text) => length + text.length + 1, 0)
  const lineText = lines[row] ?? ''
  return start + [...lineText].slice(0, column - 1).join('').length
}
