//! DTO 与后端字段一一对应，数字统一使用原生 number（有限双精度浮点数）。
import type { LogicErrorCode } from './logic'

export type Status = 'unpublished' | 'collecting' | 'closed' | 'archived'
export type FillingMode = 'anonymous' | 'login_once'
export interface Choice {
  id: string
  label: string
}
interface QuestionBase {
  id: string
  title: string
  required: boolean
}
export type Question = QuestionBase &
  (
    | { type: 'single_choice'; options: Choice[] }
    | { type: 'multiple_choice'; options: Choice[]; min_choices: number; max_choices: number }
    | { type: 'text' | 'textarea'; max_length: number }
    | { type: 'number'; integer: boolean; min: number | null; max: number | null }
    | { type: 'rating'; min: number; max: number; min_label: string; max_label: string }
  )
export interface SurveyDocument {
  logic?: DocumentLogic | null
  title: string
  description: string
  purpose: string
  thank_you: string
  questions: Question[]
}
export interface User {
  id: string
  username: string
  role: 'user' | 'admin'
}
export interface Page<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}
export interface SurveySummary {
  id: string
  title: string
  status: Status
  mode: FillingMode | null
  deadline: string | null
  taken_down: boolean
  revision: number
  current_version_id: string | null
  created_at: string
  updated_at: string
}
export interface SurveyDetail extends Omit<SurveySummary, 'title'> {
  draft: SurveyDocument
}
export interface SurveyCounts {
  unpublished: number
  collecting: number
  closed: number
  archived: number
}
export interface Version {
  id: string
  number: number
  published_at: string
  is_current: boolean
}
export interface FieldError {
  question_id: string | null
  message: string
}
export interface Preview {
  survey_id: string
  revision: number
  document: SurveyDocument
  mode: FillingMode | null
  publishable: boolean
  validation_error: FieldError | null
}
export interface PublicSurvey {
  survey_id: string
  version_id: string
  version_number: number
  mode: FillingMode
  deadline: string | null
  document: SurveyDocument
  privacy: PrivacyNotice
}
/** 免登录的问卷元信息；不包含题面与任何答卷数据。 */
export interface PublicSurveyMeta {
  survey_id: string
  title: string
  description: string
  purpose: string
  mode: FillingMode
  deadline: string | null
  privacy: PrivacyNotice
}
export interface PrivacyNotice {
  mode_notice: string
  data_visibility: string
  sensitive_data_notice: string
  public_link_notice: string
}
export type AnswerValue = string | string[] | number | null
export interface Answer {
  question_id: string
  value: AnswerValue
}
export interface Submission {
  version_id: string
  idempotency_key: string
  answers: Answer[]
}
export interface Receipt {
  status: 'submitted'
  receipt_id: string
  /** 完整答卷或筛选结束；两者都返回回执，但不一定生成可统计答卷。 */
  outcome: FlowOutcome
}
export interface ReportSelection {
  version_id: string
  version_number: number
  timezone: string
  from: string | null
  to: string | null
}
export interface ResponseSummary {
  receipt_id: string
  submitted_at: string
  outcome: FlowOutcome
}
export interface ResponseDetail extends Omit<ReportSelection, 'from' | 'to'>, ResponseSummary {
  document: SurveyDocument
  answers: Record<string, AnswerValue>
  question_states: Record<string, AnswerState>
}
export interface OptionFrequency {
  option_id: string
  label: string
  count: number
  proportion: number | null
}
export type QuestionStatistics = {
  question_id: string
  title: string
  answered_count: number
  applicable_count: number
  unanswered_count: number
  skipped_count: number
} & (
  | {
      type: 'single_choice' | 'multiple_choice'
      options: OptionFrequency[]
      proportion_note?: string
    }
  | { type: 'text' | 'textarea' }
  | { type: 'number'; mean: number | null; min: number | null; max: number | null }
  | {
      type: 'rating'
      min: number
      max: number
      mean: number | null
      distribution: { value: number; count: number }[]
    }
)
export interface TextResponse {
  receipt_id: string
  submitted_at: string
  value: string
}
export interface Statistics extends ReportSelection {
  total_count: number
  complete_count: number
  screen_out_count: number
  trend: { date: string; count: number }[]
  questions: QuestionStatistics[]
}
export interface AdminUser extends User {
  disabled: boolean
  created_at: string
}
export interface AdminSurvey {
  id: string
  owner_id: string
  title: string
  status: Status
  taken_down: boolean
  deadline: string | null
  current_version_id: string | null
}
export interface Audit {
  id: string
  actor_id: string
  action: string
  target_id: string
  reason: string
  created_at: string
}

// —— 规则与流程 ——
// 与后端 `questionnaire-logic` 的 DTO 逐字段对应；引擎（wasm 包装）只在边界上编
// 解码这些结构，界面与测试直接消费它们，因此集中放在 DTO 模块。

/** 编译期确定的类型；缺失值沿用原表达式的静态类型。 */
export type LogicValueType = 'boolean' | 'number' | 'text' | 'choice' | 'choices'

/** 计算变量；`expression` 原文保留给引擎与格式化器，界面只按节点读写。 */
export interface ComputedVariable {
  id: string
  value_type: LogicValueType
  expression: unknown
}
/** 一道题最多一组规则；未配置的字段以键缺失表达。 */
export interface QuestionRule {
  question_id: string
  visible_if?: unknown
  required_if?: unknown
  validate?: unknown
}
export interface DocumentLogic {
  flow?: FlowDefinition | null
  dsl_version: number
  semantics_version: number
  variables: ComputedVariable[]
  rules: QuestionRule[]
  /** 文本 DSL 的展示别名：键为题目、选项、变量或流程节点 ID，值为合法标识符。 */
  aliases?: Record<string, string>
}

export type FlowOutcome = 'complete' | 'screen_out'
export interface FlowBranch {
  when: unknown
  target: string
}
export type FlowNode =
  | { type: 'page'; id: string; question_ids: string[]; next: string }
  | { type: 'branch'; id: string; branches: FlowBranch[]; otherwise: string }
  | { type: 'end'; id: string; outcome: FlowOutcome }
export interface FlowDefinition {
  entry: string
  nodes: FlowNode[]
}

/** 某题在最终有效路径上的适用性；跳过与可选未答不可混用。 */
export type AnswerState = 'answered' | 'unanswered' | 'skipped'
export type FlowEvent =
  | { type: 'change'; question_id: string; value: unknown }
  | { type: 'next' }
  | { type: 'previous' }
/** 会话只读快照；改动返回值不会影响会话状态。 */
export interface FlowSnapshot {
  current_node: string
  outcome: FlowOutcome | null
  history: string[]
  path: string[]
  visible: string[]
  required: string[]
  answers: Record<string, AnswerValue>
  errors: Record<string, LogicErrorCode>
  /** 每题可直接展示的修正说明；`errors` 仍保留稳定错误码供程序分支。 */
  error_messages: Record<string, string>
  route_error: LogicErrorCode | null
  cleared: string[]
}
