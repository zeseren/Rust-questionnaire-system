//! 固定身份的问卷构造器：单元测试不依赖随机 UUID，断言可以直接引用常量。

/** 全部为合法的 v4 UUID，同一构造器多次调用复用同一身份。 */
export const FIXED_IDS = Object.freeze({
  question: Object.freeze({
    single_choice: '00000000-0000-4000-8000-000000000001',
    multiple_choice: '00000000-0000-4000-8000-000000000002',
    text: '00000000-0000-4000-8000-000000000003',
    textarea: '00000000-0000-4000-8000-000000000004',
    number: '00000000-0000-4000-8000-000000000005',
    rating: '00000000-0000-4000-8000-000000000006',
  }),
  option: Object.freeze({
    a: '00000000-0000-4000-8000-000000000101',
    b: '00000000-0000-4000-8000-000000000102',
    c: '00000000-0000-4000-8000-000000000103',
    d: '00000000-0000-4000-8000-000000000104',
  }),
})

export function singleChoiceQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.single_choice,
    title: '单选题',
    required: true,
    type: 'single_choice',
    options: [
      { id: FIXED_IDS.option.a, label: '甲' },
      { id: FIXED_IDS.option.b, label: '乙' },
    ],
    ...overrides,
  }
}

export function multipleChoiceQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.multiple_choice,
    title: '多选题',
    required: true,
    type: 'multiple_choice',
    options: [
      { id: FIXED_IDS.option.c, label: '丙' },
      { id: FIXED_IDS.option.d, label: '丁' },
    ],
    min_choices: 1,
    max_choices: 2,
    ...overrides,
  }
}

export function textQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.text,
    title: '单行文本',
    required: true,
    type: 'text',
    max_length: 200,
    ...overrides,
  }
}

export function textareaQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.textarea,
    title: '多行文本',
    required: true,
    type: 'textarea',
    max_length: 1000,
    ...overrides,
  }
}

export function numberQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.number,
    title: '数字题',
    required: true,
    type: 'number',
    integer: false,
    min: null,
    max: null,
    ...overrides,
  }
}

export function ratingQuestion(overrides = {}) {
  return {
    id: FIXED_IDS.question.rating,
    title: '评分题',
    required: true,
    type: 'rating',
    min: 1,
    max: 5,
    min_label: '很不满意',
    max_label: '非常满意',
    ...overrides,
  }
}

/** 最小合法问卷：只固定与逻辑无关的文案字段。 */
export function surveyDocument(questions, overrides = {}) {
  return {
    title: '测试问卷',
    description: '用于前端领域测试。',
    purpose: '验证模块契约。',
    thank_you: '感谢参与。',
    questions,
    ...overrides,
  }
}

/** 明确表达测试意图的深拷贝包装。 */
export function cloneDocument(value) {
  return structuredClone(value)
}
