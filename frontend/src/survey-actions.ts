//! 问卷生命周期操作的共享文案与请求；编辑器顶部栏与列表卡片使用同一份定义。
import { api } from './api'
import type { SurveyDetail } from './types'

export type SurveyLifecycleAction = 'publish' | 'close' | 'resume' | 'archive' | 'delete'
export type CardAction = Exclude<SurveyLifecycleAction, 'publish'>
export const SURVEY_ACTION_NAMES: Record<SurveyLifecycleAction, string> = {
  publish: '发布问卷',
  close: '关闭回收',
  resume: '恢复回收',
  archive: '归档问卷',
  delete: '删除草稿',
}
export const SURVEY_ACTION_DESCRIPTIONS: Record<SurveyLifecycleAction, string> = {
  publish:
    '将已保存的草稿发布为一个新版本。发布后，填写模式不能更改。已关闭的问卷发布新版本后仍保持关闭。',
  close: '关闭后停止接收新答卷，已有答卷和统计仍然保留。',
  resume: '恢复后重新接受当前版本的答卷。请确认截止时间尚未到达。',
  archive: '归档不可撤销。归档后不能编辑或再次发布，历史答卷和统计仍可查看，也可以复制为新问卷。',
  delete: '这份未发布草稿将被永久删除，无法恢复。',
}
/** 生命周期操作的 HTTP 调用；发布预检、删除跳转与列表刷新仍由调用方负责。 */
export async function deleteSurvey(id: string): Promise<void> {
  await api(`/api/surveys/${id}`, 'DELETE')
}
/** 状态迁移只允许 close/resume/archive；删除单独走 DELETE，避免误发 POST /delete。 */
export function transitionSurvey(
  id: string,
  action: Exclude<CardAction, 'delete'>,
): Promise<SurveyDetail> {
  return api<SurveyDetail>(`/api/surveys/${id}/${action}`, 'POST')
}
/** 操作成功后的统一提示；编辑器与列表共享同一文案。 */
export function surveyActionMessage(action: CardAction): string {
  return action === 'delete' ? '草稿已删除。' : `${SURVEY_ACTION_NAMES[action]}成功。`
}
