//! 问卷编辑原语：新题默认值、题目身份重写与删除后的引用清理。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  TEXT_ANSWER_LIMITS,
  duplicateQuestion,
  moveQuestion,
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

const questionTypes = ['single_choice', 'multiple_choice', 'text', 'textarea', 'number', 'rating']

// newQuestion 生成各题型的独立身份与默认约束
test('newQuestion creates distinct identities and default constraints per question type', () => {
  const ids = []
  for (const type of questionTypes) {
    const first = newQuestion(type)
    const second = newQuestion(type)
    assert.equal(first.type, type)
    assert.equal(first.title, '')
    assert.equal(first.required, true)
    ids.push(first.id, second.id)
    if ('options' in first) {
      assert.equal(first.options.length, 2)
      assert.deepEqual(
        first.options.map((option) => option.label),
        ['选项 1', '选项 2'],
      )
      assert.equal(new Set(first.options.map((option) => option.id)).size, 2)
    }
  }
  assert.equal(new Set(ids).size, ids.length, '新题身份不得重复')

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

// newQuestion 拒绝未知题型
test('newQuestion rejects unknown question types', () => {
  assert.throws(() => newQuestion('unknown'), { message: '未知题型' })
})

// duplicateQuestion 重写身份且不污染原题
test('duplicateQuestion rewrites identities without mutating the original', () => {
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

// moveQuestion 同步流程页面与题目顺序
test('moveQuestion keeps flow pages and question order in sync', () => {
  const document = readFlowDocument('diamond')
  document.logic.flow.nodes[0].question_ids.push(document.questions[1].id)
  document.logic.flow.nodes[2].question_ids = [document.questions[2].id]
  document.logic.rules = []
  document.logic.variables = []
  document.logic.flow.nodes[1].branches[0].when = { op: 'boolean', value: true }
  const original = document.questions.map((question) => question.id)
  moveQuestion(document, 0, 1)
  assert.deepEqual(
    document.questions.map((question) => question.id),
    [original[1], original[0], ...original.slice(2)],
  )
  const pages = document.logic.flow.nodes.filter((node) => node.type === 'page')
  assert.deepEqual(
    pages.flatMap((page) => page.question_ids),
    document.questions.map((question) => question.id),
    '所有页面按节点顺序拼接后必须等于题目顺序',
  )
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.deepEqual(compiled.variableOrder, [], '移动题目后流程仍应通过编译')
})

// removeQuestion 清理题目、规则与流程归属
test('removeQuestion cleans up questions, rules, and flow ownership', () => {
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

// removeQuestion 对未知身份与无规则文档保持无副作用
test('removeQuestion is a no-op for unknown identities and rule-free documents', () => {
  const question = textQuestion()
  const document = surveyDocument([question])
  const snapshot = structuredClone(document)
  removeQuestion(document, '00000000-0000-4000-8000-00000000ffff')
  assert.deepEqual(document, snapshot)
  removeQuestion(document, question.id)
  assert.deepEqual(document.questions, [], '没有规则时仍删除题目本身')
})

// duplicateQuestionRule 重写自身引用并保留上游依赖
test('duplicateQuestionRule rewrites self references and keeps upstream dependencies', () => {
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
  assert.equal(compiled.variableOrder.length, 0, '该文档没有计算变量')
})
