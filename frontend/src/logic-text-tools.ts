//! 编辑辅助只提供引用提示与位置换算，接受文本的依据仍是统一解析器。
import type { Question } from './types'
import type { DocumentLogic } from './document-logic'
import { formatLogicText } from './logic-text.ts'
import { encode } from './json.ts'

/** 服务端字段顺序和可选空值不应让已应用的文本重新变成待应用状态。 */
export function logicTextSignature(logic: DocumentLogic, questions: readonly Question[]): string {
  try {
    return formatLogicText(logic, questions)
  } catch {
    return encode(logic)
  }
}

export interface TextBinding {
  category: 'question' | 'option' | 'variable' | 'node'
  alias: string
  id: string
  label: string
}
export function textBindings(source: string, questions: readonly Question[]): TextBinding[] {
  const result: TextBinding[] = []
  const questionNames = new Map(questions.map((question) => [question.id, question.title]))
  const optionNames = new Map(
    questions.flatMap((question) =>
      'options' in question
        ? question.options.map((option) => [option.id, option.label] as const)
        : [],
    ),
  )
  // 先跳过字符串及行注释，避免把示例文字当成真实声明。
  const pattern =
    /\/\/[^\n]*|"(?:\\.|[^"\\])*"|\bbind\s+(question|option|variable|node)\s+([A-Za-z_][A-Za-z_0-9]*)\s*=\s*"([^"]+)"\s*;/g
  for (const match of source.matchAll(pattern)) {
    if (!match[1]) continue
    const category = match[1] as TextBinding['category'],
      alias = match[2]!,
      id = match[3]!
    const label =
      category === 'question'
        ? questionNames.get(id) || '未命名或已删除题目'
        : category === 'option'
          ? optionNames.get(id) || '未命名或已删除选项'
          : category === 'variable'
            ? '计算变量'
            : '流程节点'
    result.push({ category, alias, id, label })
  }
  return result
}

export function textErrorOffset(source: string, line: number, column: number): number {
  const lines = source.split('\n')
  const row = Math.max(0, Math.min(line - 1, lines.length - 1))
  const start = lines.slice(0, row).reduce((length, text) => length + text.length + 1, 0)
  return start + [...lines[row]!].slice(0, Math.max(0, column - 1)).join('').length
}
