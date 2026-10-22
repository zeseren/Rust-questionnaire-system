//! 编辑期诊断：结构前置检查先于编译器，编译错误仍映射到可聚焦的编辑器目标。
import assert from 'node:assert/strict'
import test from 'node:test'
import { logicDiagnostic } from '../../src/editor-logic.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'

function document() {
  return readFlowDocument('diamond')
}

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
          message: '入口必须是第一个页面。',
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
          message: '页面至少需要一道题，请分配题目或删除空页。',
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
          message: '页面包含已删除题目，请移除失效归属。',
        }
      },
    },
    {
      name: '出口指向前方节点',
      mutate: (document) => {
        const node = document.logic.flow.nodes[2]
        node.next = document.logic.flow.nodes[1].id
        return {
          target: `logic-node-${node.id}`,
          path: '$.logic.flow.nodes[2]',
          message: '出口必须指向后续存在的节点，请修正跳转。',
        }
      },
    },
    {
      name: '出口指向自身',
      mutate: (document) => {
        const node = document.logic.flow.nodes[2]
        node.next = node.id
        return {
          target: `logic-node-${node.id}`,
          path: '$.logic.flow.nodes[2]',
          message: '出口必须指向后续存在的节点，请修正跳转。',
        }
      },
    },
    {
      name: '出口指向不存在的节点',
      mutate: (document) => {
        const node = document.logic.flow.nodes[2]
        node.next = '00000000-0000-4000-8000-000000009999'
        return {
          target: `logic-node-${node.id}`,
          path: '$.logic.flow.nodes[2]',
          message: '出口必须指向后续存在的节点，请修正跳转。',
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
          message: '没有前置节点指向这里，请配置进入路径或删除节点。',
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
          message: '每题须归属一个页面，页面顺序必须与题目顺序一致。请调整题目归属或题目顺序。',
        }
      },
    },
  ]
  for (const item of cases) {
    const current = document()
    const expected = item.mutate(current)
    assert.deepEqual(logicDiagnostic(current.logic, current.questions), expected, item.name)
  }
})

test('编译错误按路径映射到变量、规则、节点、题目或编辑器兜底', () => {
  const variable = document()
  const variableId = variable.logic.variables[0].id
  variable.logic.variables[0].expression = {
    op: 'answer',
    question_id: '00000000-0000-4000-8000-000000009999',
  }
  assert.deepEqual(logicDiagnostic(variable.logic, variable.questions), {
    message: '规则引用的题目或选项无效',
    target: `logic-variable-${variableId}`,
    path: '$.logic.variables[0].expression',
  })

  const rule = document()
  rule.logic.rules[0].validate = {
    op: 'answer',
    question_id: rule.questions[1].id,
  }
  assert.deepEqual(logicDiagnostic(rule.logic, rule.questions), {
    message: '规则的操作数类型不匹配',
    target: 'logic-rule-0',
    path: '$.logic.rules[0].validate',
  })

  const node = document()
  node.logic.flow.nodes[1].branches[0].when.option_id = '00000000-0000-4000-8000-000000009999'
  assert.deepEqual(logicDiagnostic(node.logic, node.questions), {
    message: '规则引用的题目或选项无效',
    target: `logic-node-${node.logic.flow.nodes[1].id}`,
    path: '$.logic.flow.nodes[1].branches[0].when',
  })

  const question = document()
  question.questions[1].id = question.questions[0].id
  question.logic.flow.nodes[2].question_ids = [question.questions[0].id]
  assert.deepEqual(logicDiagnostic(question.logic, question.questions), {
    message: '规则引用的题目或选项无效',
    target: `question-${question.questions[0].id}`,
    path: '$.questions',
  })

  const fallback = document()
  fallback.logic.dsl_version = 2
  assert.deepEqual(logicDiagnostic(fallback.logic, fallback.questions), {
    message: '不支持此规则版本',
    target: 'logic-editor',
    path: '$',
  })
})

test('非领域异常原样重抛', () => {
  const logic = { dsl_version: 1, semantics_version: 1, variables: [], rules: [] }
  logic.self = logic
  assert.throws(() => logicDiagnostic(logic, []), TypeError)
})
