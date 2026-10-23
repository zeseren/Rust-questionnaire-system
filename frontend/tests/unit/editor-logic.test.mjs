//! 编辑期诊断：结构前置检查先于编译器，编译错误仍映射到可聚焦的编辑器目标。
import assert from 'node:assert/strict'
import test from 'node:test'
import { logicDiagnostic } from '../../src/editor-logic.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'

function document() {
  return readFlowDocument('diamond')
}

/** 诊断只需定位到可聚焦目标；文案本身由领域错误测试覆盖，这里不逐字镜像。 */
const focusOf = (diagnostic) => ({ target: diagnostic.target, path: diagnostic.path })

test('无流程规则与合法流程都不产生诊断', () => {
  assert.equal(
    logicDiagnostic({ dsl_version: 1, semantics_version: 1, variables: [], rules: [] }, []),
    null,
  )
  const valid = document()
  assert.equal(logicDiagnostic(valid.logic, valid.questions), null)
})

test('结构错误返回题目归属、节点或整体流程的精确目标', () => {
  const cases = [
    {
      name: '入口不是第一个页面',
      mutate: (document) => {
        document.logic.flow.entry = document.logic.flow.nodes[2].id
        return {
          target: `logic-node-${document.logic.flow.nodes[0].id}`,
          path: '$.logic.flow.nodes[0]',
        }
      },
    },
    {
      name: '页面没有题目',
      mutate: (document) => {
        document.logic.flow.nodes[0].question_ids = []
        return {
          target: `logic-node-${document.logic.flow.nodes[0].id}`,
          path: '$.logic.flow.nodes[0]',
        }
      },
    },
    {
      name: '页面包含已删除题目',
      mutate: (document) => {
        const node = document.logic.flow.nodes[3]
        node.question_ids.push('00000000-0000-4000-8000-000000009999')
        return {
          target: `logic-node-${node.id}`,
          path: '$.logic.flow.nodes[3]',
        }
      },
    },
    {
      // 指向前方节点、指向自身与指向不存在的节点命中同一守卫，保留一个代表。
      name: '出口指向非后续节点',
      mutate: (document) => {
        const node = document.logic.flow.nodes[2]
        node.next = node.id
        return {
          target: `logic-node-${node.id}`,
          path: '$.logic.flow.nodes[2]',
        }
      },
    },
    {
      name: '存在不可达节点',
      mutate: (document) => {
        const flow = document.logic.flow
        flow.nodes[1].otherwise = flow.nodes[2].id
        return {
          target: `logic-node-${flow.nodes[3].id}`,
          path: '$.logic.flow.nodes[3]',
        }
      },
    },
    {
      name: '页面题目归属顺序与问卷题目列表不一致',
      mutate: (document) => {
        const flow = document.logic.flow
        const left = flow.nodes[3].question_ids
        flow.nodes[3].question_ids = flow.nodes[4].question_ids
        flow.nodes[4].question_ids = left
        return {
          target: 'logic-membership',
          path: '$.logic.flow',
        }
      },
    },
  ]
  for (const item of cases) {
    const current = document()
    const expected = item.mutate(current)
    const diagnostic = logicDiagnostic(current.logic, current.questions)
    assert.deepEqual(focusOf(diagnostic), expected, item.name)
    assert.ok(
      typeof diagnostic.message === 'string' && diagnostic.message.length > 0,
      `${item.name} 必须给出可读文案`,
    )
  }
})

test('编译错误按路径映射到变量、规则、节点、题目或编辑器兜底', () => {
  const variable = document()
  const variableId = variable.logic.variables[0].id
  variable.logic.variables[0].expression = {
    op: 'answer',
    question_id: '00000000-0000-4000-8000-000000009999',
  }
  assert.deepEqual(focusOf(logicDiagnostic(variable.logic, variable.questions)), {
    target: `logic-variable-${variableId}`,
    path: '$.logic.variables[0].expression',
  })

  const rule = document()
  rule.logic.rules[0].validate = {
    op: 'answer',
    question_id: rule.questions[1].id,
  }
  assert.deepEqual(focusOf(logicDiagnostic(rule.logic, rule.questions)), {
    target: 'logic-rule-0',
    path: '$.logic.rules[0].validate',
  })

  const node = document()
  node.logic.flow.nodes[1].branches[0].when.option_id = '00000000-0000-4000-8000-000000009999'
  assert.deepEqual(focusOf(logicDiagnostic(node.logic, node.questions)), {
    target: `logic-node-${node.logic.flow.nodes[1].id}`,
    path: '$.logic.flow.nodes[1].branches[0].when',
  })

  const question = document()
  question.questions[1].id = question.questions[0].id
  question.logic.flow.nodes[2].question_ids = [question.questions[0].id]
  assert.deepEqual(focusOf(logicDiagnostic(question.logic, question.questions)), {
    target: `question-${question.questions[0].id}`,
    path: '$.questions',
  })

  const fallback = document()
  fallback.logic.dsl_version = 2
  assert.deepEqual(focusOf(logicDiagnostic(fallback.logic, fallback.questions)), {
    target: 'logic-editor',
    path: '$',
  })
})

test('非领域异常原样重抛', () => {
  const logic = { dsl_version: 1, semantics_version: 1, variables: [], rules: [] }
  logic.self = logic
  assert.throws(() => logicDiagnostic(logic, []), TypeError)
})
