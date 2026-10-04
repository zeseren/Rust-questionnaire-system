//! DTO 与后端字段一一对应，数字答案保留原始精度。
import type { LosslessNumber } from 'lossless-json'
import type { DocumentLogic } from './document-logic'

export type ExactNumber = number | LosslessNumber
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
  share_url: string | null
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
  privacy: {
    mode_notice: string
    data_visibility: string
    sensitive_data_notice: string
    public_link_notice: string
  }
}
export type AnswerValue = string | string[] | ExactNumber | null
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
}
export interface ResponseDetail extends Omit<ReportSelection, 'from' | 'to'>, ResponseSummary {
  document: SurveyDocument
  answers: Record<string, AnswerValue>
}
export interface OptionFrequency {
  option_id: string
  label: string
  count: number
  proportion: ExactNumber | null
}
export type QuestionStatistics = { question_id: string; title: string; answered_count: number } & (
  | {
      type: 'single_choice' | 'multiple_choice'
      options: OptionFrequency[]
      proportion_note?: string
    }
  | { type: 'text' | 'textarea'; answers: (ResponseSummary & { value: string })[] }
  | { type: 'number'; mean: ExactNumber | null; min: ExactNumber | null; max: ExactNumber | null }
  | {
      type: 'rating'
      min: number
      max: number
      mean: ExactNumber | null
      distribution: { value: number; count: number }[]
    }
)
export interface Statistics extends ReportSelection {
  total_count: number
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
