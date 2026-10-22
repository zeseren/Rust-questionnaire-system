//! 简洁条件与表达式树之间的双向映射，只接受可无损表达的节点。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  changeConditionOperator,
  comparisonNames,
  comparisonOperators,
  conditionExpression,
  conditionOperators,
  groupExpression,
  readConditionGroup,
} from '../../src/condition-builder.ts'
import {
  numberQuestion,
  ratingQuestion,
  singleChoiceQuestion,
  textQuestion,
} from '../helpers/documents.mjs'

const describe = (operators) => operators.map((entry) => entry.value)

test('比较名称与运算符顺序是稳定契约', () => {
  assert.deepEqual(comparisonNames, {
    eq: '等于',
    ne: '不等于',
    gt: '大于',
    ge: '大于或等于',
    lt: '小于',
    le: '小于或等于',
  })
  assert.deepEqual(comparisonOperators, ['eq', 'ne', 'gt', 'ge', 'lt', 'le'])
})

test('conditionOperators 按题型提供可用运算符', () => {
  assert.deepEqual(describe(conditionOperators()), ['answered', 'not_answered'])
  assert.deepEqual(describe(conditionOperators(singleChoiceQuestion())), [
    'answered',
    'not_answered',
    'selected',
    'not_selected',
  ])
  assert.deepEqual(describe(conditionOperators(textQuestion())), [
    'answered',
    'not_answered',
    'eq',
    'ne',
  ])
  assert.deepEqual(describe(conditionOperators(numberQuestion())), [
    'answered',
    'not_answered',
    'eq',
    'ne',
    'gt',
    'ge',
    'lt',
    'le',
  ])
  assert.deepEqual(describe(conditionOperators(ratingQuestion())), [
    'answered',
    'not_answered',
    'eq',
    'ne',
    'gt',
    'ge',
    'lt',
    'le',
  ])
  assert.deepEqual(conditionOperators(textQuestion())[2], { value: 'eq', label: '等于' })
})

test('changeConditionOperator 清空不适用的操作数并保留兼容值', () => {
  const number = numberQuestion()
  const choice = singleChoiceQuestion()
  const text = textQuestion()
  assert.deepEqual(
    changeConditionOperator(
      { questionId: number.id, operator: 'ge', value: '18.00', valueType: 'number' },
      'answered',
      number,
    ),
    { questionId: number.id, operator: 'answered', value: '' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: number.id, operator: 'gt', value: '18.00', valueType: 'number' },
      'not_answered',
      number,
    ),
    { questionId: number.id, operator: 'not_answered', value: '' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: choice.id, operator: 'answered', value: '' },
      'selected',
      choice,
    ),
    { questionId: choice.id, operator: 'selected', value: choice.options[0].id },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: choice.id, operator: 'selected', value: choice.options[1].id },
      'not_selected',
      choice,
    ),
    { questionId: choice.id, operator: 'not_selected', value: choice.options[1].id },
  )
  assert.deepEqual(
    changeConditionOperator({ questionId: choice.id, operator: 'eq', value: 'x' }, 'selected'),
    { questionId: choice.id, operator: 'selected', value: '' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: number.id, operator: 'ge', value: '18.00', valueType: 'number' },
      'gt',
      number,
    ),
    { questionId: number.id, operator: 'gt', value: '18.00', valueType: 'number' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: text.id, operator: 'eq', value: '001', valueType: 'text' },
      'ne',
      text,
    ),
    { questionId: text.id, operator: 'ne', value: '001', valueType: 'text' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: choice.id, operator: 'selected', value: choice.options[0].id },
      'gt',
      number,
    ),
    { questionId: choice.id, operator: 'gt', valueType: 'number', value: '0' },
  )
  assert.deepEqual(
    changeConditionOperator(
      { questionId: text.id, operator: 'ge', value: '5', valueType: 'number' },
      'ne',
      text,
    ),
    { questionId: text.id, operator: 'ne', valueType: 'text', value: '' },
  )
  assert.deepEqual(
    changeConditionOperator({ questionId: 'q', operator: 'answered', value: '' }, 'le'),
    { questionId: 'q', operator: 'le', valueType: 'number', value: '0' },
  )
})

test('conditionExpression 生成四类原始节点', () => {
  assert.deepEqual(conditionExpression({ questionId: 'q', operator: 'answered', value: '' }), {
    op: 'answered',
    question_id: 'q',
  })
  assert.deepEqual(conditionExpression({ questionId: 'q', operator: 'not_answered', value: '' }), {
    op: 'not',
    expression: { op: 'answered', question_id: 'q' },
  })
  assert.deepEqual(conditionExpression({ questionId: 'q', operator: 'selected', value: 'o' }), {
    op: 'selected',
    question_id: 'q',
    option_id: 'o',
  })
  assert.deepEqual(conditionExpression({ questionId: 'q', operator: 'not_selected', value: 'o' }), {
    op: 'not',
    expression: { op: 'selected', question_id: 'q', option_id: 'o' },
  })
  assert.deepEqual(conditionExpression({ questionId: 'q', operator: 'eq', value: '1' }), {
    op: 'compare',
    comparison: 'eq',
    left: { op: 'answer', question_id: 'q' },
    right: { op: 'number', value: '1' },
  })
  assert.deepEqual(
    conditionExpression({ questionId: 'q', operator: 'le', value: '001', valueType: 'text' }),
    {
      op: 'compare',
      comparison: 'le',
      left: { op: 'answer', question_id: 'q' },
      right: { op: 'text', value: '001' },
    },
  )
})

test('groupExpression 保留作者选择的组合方式', () => {
  assert.deepEqual(groupExpression({ mode: 'always', conditions: [] }), {
    op: 'boolean',
    value: true,
  })
  assert.deepEqual(groupExpression({ mode: 'never', conditions: [] }), {
    op: 'boolean',
    value: false,
  })
  const condition = { questionId: 'q', operator: 'answered', value: '' }
  assert.deepEqual(groupExpression({ mode: 'any', conditions: [condition] }), {
    op: 'any',
    expressions: [{ op: 'answered', question_id: 'q' }],
  })
  assert.deepEqual(groupExpression({ mode: 'all', conditions: [condition] }), {
    op: 'all',
    expressions: [{ op: 'answered', question_id: 'q' }],
  })
})

test('readConditionGroup 识别可无损表达的节点', () => {
  assert.deepEqual(readConditionGroup({ op: 'boolean', value: true }), {
    mode: 'always',
    conditions: [],
  })
  assert.deepEqual(readConditionGroup({ op: 'boolean', value: false }), {
    mode: 'never',
    conditions: [],
  })
  assert.deepEqual(readConditionGroup({ op: 'answered', question_id: 'q' }), {
    mode: 'all',
    conditions: [{ questionId: 'q', operator: 'answered', value: '' }],
  })
  assert.deepEqual(
    readConditionGroup({ op: 'not', expression: { op: 'answered', question_id: 'q' } }),
    {
      mode: 'all',
      conditions: [{ questionId: 'q', operator: 'not_answered', value: '' }],
    },
  )
  assert.deepEqual(
    readConditionGroup({
      op: 'not',
      expression: { op: 'selected', question_id: 'q', option_id: 'o' },
    }),
    { mode: 'all', conditions: [{ questionId: 'q', operator: 'not_selected', value: 'o' }] },
  )
  assert.deepEqual(readConditionGroup({ op: 'selected', question_id: 'q', option_id: 'o' }), {
    mode: 'all',
    conditions: [{ questionId: 'q', operator: 'selected', value: 'o' }],
  })
  const compare = {
    op: 'compare',
    comparison: 'ge',
    left: { op: 'answer', question_id: 'q' },
    right: { op: 'text', value: '001' },
  }
  assert.deepEqual(readConditionGroup(compare), {
    mode: 'all',
    conditions: [{ questionId: 'q', operator: 'ge', value: '001', valueType: 'text' }],
  })
  assert.deepEqual(
    readConditionGroup({
      op: 'any',
      expressions: [
        { op: 'answered', question_id: 'q' },
        { op: 'selected', question_id: 'q', option_id: 'o' },
      ],
    }),
    {
      mode: 'any',
      conditions: [
        { questionId: 'q', operator: 'answered', value: '' },
        { questionId: 'q', operator: 'selected', value: 'o' },
      ],
    },
  )
  assert.deepEqual(readConditionGroup({ op: 'all', expressions: [] }), {
    mode: 'all',
    conditions: [],
  })
})

test('readConditionGroup 拒绝无法无损表达的节点', () => {
  const rejected = [
    { op: 'all', expressions: [{ op: 'all', expressions: [] }] },
    { op: 'all', expressions: [{ op: 'boolean', value: true }] },
    { op: 'all' },
    { op: 'answered', question_id: 'q', extra: true },
    { op: 'answered' },
    { op: 'selected', question_id: 'q' },
    {
      op: 'not',
      expression: {
        op: 'compare',
        comparison: 'eq',
        left: { op: 'answer', question_id: 'q' },
        right: { op: 'number', value: '1' },
      },
    },
    {
      op: 'compare',
      comparison: 'unknown',
      left: { op: 'answer', question_id: 'q' },
      right: { op: 'number', value: '1' },
    },
    {
      op: 'compare',
      comparison: 'eq',
      left: { op: 'value', question_id: 'q' },
      right: { op: 'number', value: '1' },
    },
    {
      op: 'compare',
      comparison: 'eq',
      left: { op: 'answer', question_id: 'q' },
      right: { op: 'number', value: 1 },
    },
  ]
  for (const raw of rejected) assert.equal(readConditionGroup(raw), null, JSON.stringify(raw))
})

test('条件与条件组在原始节点上往返一致', () => {
  for (const condition of [
    { questionId: 'q', operator: 'answered', value: '' },
    { questionId: 'q', operator: 'not_answered', value: '' },
    { questionId: 'q', operator: 'selected', value: 'o' },
    { questionId: 'q', operator: 'not_selected', value: 'o' },
    { questionId: 'q', operator: 'eq', value: '18.00', valueType: 'number' },
    { questionId: 'q', operator: 'ne', value: '001', valueType: 'text' },
  ]) {
    const group = readConditionGroup(conditionExpression(condition))
    assert.deepEqual(group, { mode: 'all', conditions: [condition] })
  }
  for (const group of [
    { mode: 'all', conditions: [{ questionId: 'q', operator: 'answered', value: '' }] },
    {
      mode: 'any',
      conditions: [
        { questionId: 'q', operator: 'not_selected', value: 'o' },
        { questionId: 'q', operator: 'ge', value: '2', valueType: 'number' },
      ],
    },
    { mode: 'always', conditions: [] },
    { mode: 'never', conditions: [] },
  ])
    assert.deepEqual(readConditionGroup(groupExpression(group)), group)
})
