//! 问卷编辑原语：新题默认值、身份重写与删除后的引用清理。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  TEXT_ANSWER_LIMITS,
  duplicateQuestion,
  duplicateSurveyDocument,
  newQuestion,
  removeQuestion,
} from '../../src/documents.ts'
import { compileDocumentLogic } from '../../src/wasm/index.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'
import {
  multipleChoiceQuestion,
  singleChoiceQuestion,
  surveyDocument,
  textQuestion,
  textareaQuestion,
} from '../helpers/documents.mjs'

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
const questionTypes = ['single_choice', 'multiple_choice', 'text', 'textarea', 'number', 'rating']

test('文本答案容量与后端保持一致', () => {
  assert.deepEqual(TEXT_ANSWER_LIMITS, { text: 200, textarea: 1000 })
})

test('newQuestion 为六种题型生成独立身份与默认结构', () => {
  const ids = []
  for (const type of questionTypes) {
    const first = newQuestion(type)
    const second = newQuestion(type)
    assert.equal(first.type, type)
    assert.equal(first.title, '')
    assert.equal(first.required, true)
    assert.match(first.id, UUID)
    ids.push(first.id, second.id)
    if ('options' in first) {
      assert.equal(first.options.length, 2)
      assert.deepEqual(
        first.options.map((option) => option.label),
        ['选项 1', '选项 2'],
      )
      for (const option of first.options) assert.match(option.id, UUID)
      assert.equal(new Set(first.options.map((option) => option.id)).size, 2)
    }
  }
  assert.equal(new Set(ids).size, ids.length, '新题身份不得重复')
})

test('newQuestion 各题型默认约束', () => {
  const multiple = newQuestion('multiple_choice')
  assert.equal(multiple.min_choices, 1)
  assert.equal(multiple.max_choices, 2)
  assert.equal(newQuestion('text').max_length, TEXT_ANSWER_LIMITS.text)
  assert.equal(newQuestion('textarea').max_length, TEXT_ANSWER_LIMITS.textarea)
  const number = newQuestion('number')
  assert.deepEqual(
    { integer: number.integer, min: number.min, max: number.max },
    { integer: false, min: null, max: null },
  )
  const rating = newQuestion('rating')
  assert.deepEqual(
    { min: rating.min, max: rating.max, min_label: rating.min_label, max_label: rating.max_label },
    { min: 1, max: 5, min_label: '很不满意', max_label: '非常满意' },
  )
})

test('newQuestion 拒绝未知题型', () => {
  assert.throws(() => newQuestion('unknown'), { message: '未知题型' })
})

test('duplicateQuestion 重写身份且不污染原题', () => {
  const source = singleChoiceQuestion()
  const document = surveyDocument([source])
  const before = structuredClone(source)
  const duplicated = duplicateQuestion(source, document)
  assert.deepEqual(source, before)
  assert.notEqual(duplicated.id, source.id)
  const originalIds = new Set([source.id, ...source.options.map((option) => option.id)])
  for (const id of [duplicated.id, ...duplicated.options.map((option) => option.id)])
    assert.ok(!originalIds.has(id), '复制出的题目与选项必须使用全新身份')
  duplicated.options[0].label = '改写'
  assert.equal(source.options[0].label, '甲', '复制结果必须与原题完全隔离')
})

test('duplicateSurveyDocument 同步重写整卷全部引用', () => {
  const source = readFlowDocument('diamond')
  const before = structuredClone(source)
  const duplicated = duplicateSurveyDocument(source)
  assert.deepEqual(source, before, '来源文档不得被修改')
  assert.equal(duplicated.questions.length, source.questions.length)
  const originalIds = [
    ...source.questions.flatMap((question) => [
      question.id,
      ...(question.options ?? []).map((option) => option.id),
    ]),
    ...source.logic.variables.map((variable) => variable.id),
    ...source.logic.flow.nodes.map((node) => node.id),
  ]
  const serialized = JSON.stringify(duplicated)
  for (const id of originalIds) assert.ok(!serialized.includes(id), `复制不得遗留 ${id}`)
  assert.equal(duplicated.logic.variables[0].expression.question_id, duplicated.questions[1].id)
  assert.equal(duplicated.logic.rules[0].question_id, duplicated.questions[1].id)
  assert.deepEqual(duplicated.logic.flow.nodes[2].question_ids, [duplicated.questions[1].id])
  assert.equal(duplicated.logic.flow.entry, duplicated.logic.flow.nodes[0].id)
  const compiled = compileDocumentLogic(duplicated.logic, duplicated.questions)
  assert.ok(Array.isArray(compiled.variableOrder))
  assert.equal(compiled.variableOrder.length, 2, '菱形文档声明两个计算变量')
})

test('removeQuestion 清理题目、规则与流程归属', () => {
  const keep = textQuestion()
  const drop = textareaQuestion()
  const page = '00000000-0000-4000-8000-0000000000b1'
  const end = '00000000-0000-4000-8000-0000000000b2'
  const rule = (question_id) => ({
    question_id,
    required_if: { op: 'boolean', value: true },
  })
  const document = surveyDocument([keep, drop], {
    logic: {
      dsl_version: 1,
      semantics_version: 1,
      variables: [],
      rules: [rule(keep.id), rule(drop.id)],
      flow: {
        entry: page,
        nodes: [
          { type: 'page', id: page, question_ids: [keep.id, drop.id], next: end },
          { type: 'end', id: end, outcome: 'complete' },
        ],
      },
    },
  })
  removeQuestion(document, drop.id)
  assert.deepEqual(document.questions, [keep])
  assert.deepEqual(document.logic.rules, [rule(keep.id)])
  assert.deepEqual(document.logic.flow.nodes[0].question_ids, [keep.id])
})

test('removeQuestion 对未知身份与无规则文档保持无副作用', () => {
  const question = textQuestion()
  const document = surveyDocument([question])
  const snapshot = structuredClone(document)
  removeQuestion(document, '00000000-0000-4000-8000-00000000ffff')
  assert.deepEqual(document, snapshot)
  removeQuestion(document, question.id)
  assert.deepEqual(document.questions, [], '没有规则时仍删除题目本身')
})

test('duplicateQuestionRule 重写自身引用并保留上游依赖', () => {
  const other = singleChoiceQuestion({ id: '00000000-0000-4000-8000-0000000000a1' })
  const source = multipleChoiceQuestion({
    id: '00000000-0000-4000-8000-0000000000a2',
    title: '多选来源',
  })
  const page = '00000000-0000-4000-8000-0000000000b1'
  const end = '00000000-0000-4000-8000-0000000000b2'
  const sourceRule = {
    question_id: source.id,
    visible_if: { op: 'answered', question_id: other.id },
    required_if: { op: 'selected', question_id: other.id, option_id: other.options[0].id },
    validate: {
      op: 'any',
      expressions: [
        {
          op: 'contains_all',
          question_id: source.id,
          option_ids: source.options.map((option) => option.id),
        },
        {
          op: 'coalesce',
          value: {
            op: 'all',
            expressions: [
              {
                op: 'compare',
                comparison: 'gt',
                left: {
                  op: 'length',
                  value: { op: 'answer', question_id: source.id },
                },
                right: { op: 'number', value: '0' },
              },
            ],
          },
          fallback: { op: 'boolean', value: false },
        },
      ],
    },
  }
  const document = surveyDocument([other, source], {
    logic: {
      dsl_version: 1,
      semantics_version: 1,
      variables: [],
      rules: [structuredClone(sourceRule)],
      flow: {
        entry: page,
        nodes: [
          { type: 'page', id: page, question_ids: [other.id, source.id], next: end },
          { type: 'end', id: end, outcome: 'complete' },
        ],
      },
    },
  })
  const sourceBefore = structuredClone(source)
  const duplicated = duplicateQuestion(source, document)
  document.questions.push(duplicated)

  assert.deepEqual(source, sourceBefore)
  assert.deepEqual(document.logic.rules[0], sourceRule, '原规则不得被改写')
  const copied = document.logic.rules.at(-1)
  assert.equal(document.logic.rules.length, 2)
  assert.notEqual(copied, sourceRule)
  assert.equal(copied.question_id, duplicated.id)
  assert.deepEqual(copied.visible_if, sourceRule.visible_if, '上游题目引用保持原样')
  assert.deepEqual(copied.required_if, sourceRule.required_if, '上游选项引用保持原样')

  const [contains, coalesce] = copied.validate.expressions
  assert.equal(copied.validate.op, 'any')
  assert.deepEqual(contains, {
    op: 'contains_all',
    question_id: duplicated.id,
    option_ids: duplicated.options.map((option) => option.id),
  })
  assert.equal(coalesce.value.op, 'all')
  assert.equal(coalesce.value.expressions[0].left.value.question_id, duplicated.id)
  assert.equal(coalesce.value.expressions[0].right.value, '0')
  assert.deepEqual(coalesce.fallback, { op: 'boolean', value: false })
  assert.deepEqual(document.logic.flow.nodes[0].question_ids, [other.id, source.id, duplicated.id])
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.ok(Array.isArray(compiled.variableOrder))
  assert.equal(compiled.variableOrder.length, 0, '该文档没有计算变量')
})
