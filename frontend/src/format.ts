//! 时间以 Asia/Shanghai 展示，空统计值和数字原文不被转换为零。
import { isLosslessNumber } from 'lossless-json'
import type { AnswerValue, Question, Status } from './types'

export const statusNames: Record<Status, string> = {
  unpublished: '草稿',
  collecting: '回收中',
  closed: '已关闭',
  archived: '已归档',
}
export const questionNames: Record<Question['type'], string> = {
  single_choice: '单选题',
  multiple_choice: '多选题',
  text: '单行文本',
  textarea: '多行文本',
  number: '数字题',
  rating: '评分题',
}
const shanghaiDateTime = new Intl.DateTimeFormat('zh-CN', {
  timeZone: 'Asia/Shanghai',
  dateStyle: 'medium',
  timeStyle: 'short',
})
export function dateTime(value: string | null | undefined) {
  if (!value) return '未设置'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? '未设置' : shanghaiDateTime.format(date)
}
export function exact(value: unknown): string {
  return value == null ? '暂无数据' : isLosslessNumber(value) ? value.toString() : String(value)
}
export function answerText(question: Question, answer: AnswerValue | undefined) {
  if (answer == null) return '未作答'
  if ('options' in question) {
    const values = Array.isArray(answer) ? answer : [answer]
    return values
      .map(
        (value) => question.options.find((option) => option.id === value)?.label ?? String(value),
      )
      .join('、')
  }
  return exact(answer)
}
export function localInput(value: string | null): string {
  if (!value) return ''
  const shanghai = new Date(new Date(value).getTime() + 8 * 3600_000)
  return shanghai.toISOString().slice(0, 16)
}
export function toTimestamp(value: string): string | null {
  return value ? `${value}:00+08:00` : null
}
export function shareLink(id: string) {
  return `${location.origin}/s/${encodeURIComponent(id)}`
}
