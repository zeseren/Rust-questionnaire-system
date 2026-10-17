//! 模板每次生成新的配置身份，条件与流程始终引用本次生成的题目。
import { newQuestion } from './documents.ts'
import type { DocumentLogic, Question, SurveyDocument } from './types.ts'

export const surveyTemplates = [
  {
    id: 'feedback',
    title: '满意度回访',
    description: '低分时追问原因，让每一次反馈都有重点。',
    detail: '3 道题 · 条件显隐 · 计算变量',
    icon: 'spark',
    color: 'violet',
  },
  {
    id: 'routing',
    title: '产品体验调研',
    description: '根据使用经历分流，再回到共同的结束页。',
    detail: '4 道题 · 分页跳转 · 分支汇合',
    icon: 'grid',
    color: 'teal',
  },
  {
    id: 'registration',
    title: '活动报名',
    description: '需要接送时收集地点，并校验参加人数。',
    detail: '3 道题 · 动态必填 · 答案约束',
    icon: 'document',
    color: 'blue',
  },
] as const
export type SurveyTemplateId = (typeof surveyTemplates)[number]['id']

function question<T extends Question['type']>(type: T, title: string) {
  const created = newQuestion(type)
  created.title = title
  return created
}
function choice(title: string, labels: string[]) {
  const result = question('single_choice', title)
  result.options = labels.map((label) => ({ id: crypto.randomUUID(), label }))
  return result
}

export function createSurveyTemplate(template: SurveyTemplateId | null): SurveyDocument {
  const document: SurveyDocument = {
    title: '',
    description: '',
    purpose: '',
    thank_you: '感谢你的认真回答。',
    questions: [],
  }
  if (!template) return document
  const meta = surveyTemplates.find((item) => item.id === template)
  if (!meta) throw new Error('未知问卷模板')
  document.title = meta.title
  const logic: DocumentLogic = {
    dsl_version: 1,
    semantics_version: 1,
    variables: [],
    rules: [],
  }
  document.logic = logic
  if (template === 'feedback') {
    const score = question('rating', '你对本次服务的整体体验满意吗？')
    const reason = {
      ...newQuestion('textarea'),
      title: '哪些地方还可以做得更好？',
      required: false,
    }
    const recommend = choice('你愿意将我们的服务推荐给朋友吗？', ['愿意', '暂时不会'])
    const lowScore = crypto.randomUUID()
    document.description = '用一分钟分享你的感受，帮助我们把服务做得更好。'
    document.purpose = '了解服务满意度及改进建议，仅用于服务质量分析。'
    document.questions = [score, reason, recommend]
    logic.variables.push({
      id: lowScore,
      value_type: 'boolean',
      expression: {
        op: 'compare',
        comparison: 'le',
        left: { op: 'answer', question_id: score.id },
        right: { op: 'number', value: '3' },
      },
    })
    logic.rules.push({
      question_id: reason.id,
      visible_if: { op: 'variable', variable_id: lowScore },
      required_if: { op: 'variable', variable_id: lowScore },
    })
  } else if (template === 'routing') {
    const used = choice('你是否使用过我们的产品？', ['使用过', '还没有'])
    const experience = { ...newQuestion('textarea'), title: '使用过程中，什么最让你印象深刻？' }
    const expectation = { ...newQuestion('textarea'), title: '你最希望产品帮你解决什么问题？' }
    const interest = question('rating', '你对继续了解这款产品有多大兴趣？')
    interest.min_label = '暂时没有兴趣'
    interest.max_label = '非常感兴趣'
    document.description = '无论是否使用过，都欢迎分享你的体验与期待。'
    document.purpose = '收集产品使用反馈及需求，用于产品改进和规划。'
    document.questions = [used, experience, expectation, interest]
    const entry = crypto.randomUUID()
    const branch = crypto.randomUUID()
    const returning = crypto.randomUUID()
    const newcomer = crypto.randomUUID()
    const join = crypto.randomUUID()
    const end = crypto.randomUUID()
    logic.flow = {
      entry,
      nodes: [
        { type: 'page', id: entry, question_ids: [used.id], next: branch },
        {
          type: 'branch',
          id: branch,
          branches: [
            {
              when: {
                op: 'selected',
                question_id: used.id,
                option_id: used.options[0]?.id ?? '',
              },
              target: returning,
            },
          ],
          otherwise: newcomer,
        },
        { type: 'page', id: returning, question_ids: [experience.id], next: join },
        { type: 'page', id: newcomer, question_ids: [expectation.id], next: join },
        { type: 'page', id: join, question_ids: [interest.id], next: end },
        { type: 'end', id: end, outcome: 'complete' },
      ],
    }
  } else if (template === 'registration') {
    const count = question('number', '本次共有几位参加者？')
    count.integer = true
    count.min = 1
    count.max = 10
    const transport = choice('是否需要安排接送？', ['需要接送', '自行前往'])
    const transportOption = transport.options[0]?.id ?? ''
    const address = { ...newQuestion('text'), title: '请填写方便上车的地点', required: false }
    document.description = '请告诉我们参加人数与出行安排，以便提前准备。'
    document.purpose = '收集活动人数及接送需求，仅用于本次活动安排。'
    document.questions = [count, transport, address]
    logic.rules = [
      {
        question_id: count.id,
        validate: {
          op: 'compare',
          comparison: 'ge',
          left: { op: 'answer', question_id: count.id },
          right: { op: 'number', value: '1' },
        },
      },
      {
        question_id: address.id,
        visible_if: {
          op: 'selected',
          question_id: transport.id,
          option_id: transportOption,
        },
        required_if: {
          op: 'selected',
          question_id: transport.id,
          option_id: transportOption,
        },
      },
    ]
  } else {
    const exhaustive: never = template
    return exhaustive
  }
  return document
}
