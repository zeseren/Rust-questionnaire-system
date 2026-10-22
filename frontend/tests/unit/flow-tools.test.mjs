//! 流程拓扑插入：新节点接管前驱指向原后继的出口，其余跳转保持不变。
import assert from 'node:assert/strict'
import test from 'node:test'
import { insertFlowNode } from '../../src/flow-tools.ts'

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
const id = (suffix) => `00000000-0000-4000-8000-0000000000${suffix}`

function linearFlow() {
  const [p1, p2, end] = [id('b1'), id('b2'), id('b3')]
  return {
    entry: p1,
    nodes: [
      { type: 'page', id: p1, question_ids: [], next: p2 },
      { type: 'page', id: p2, question_ids: [], next: end },
      { type: 'end', id: end, outcome: 'complete' },
    ],
  }
}

test('在页面后插入页面会回接前驱并返回新身份', () => {
  const flow = linearFlow()
  const [p1, p2, end] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 0, 'page')
  assert.match(inserted, UUID)
  assert.notEqual(inserted, p1)
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [p1, inserted, p2, end],
  )
  assert.equal(flow.nodes[0].next, inserted)
  assert.deepEqual(flow.nodes[1], { type: 'page', id: inserted, question_ids: [], next: p2 })
})

test('在分支后插入分支只回接指向原后继的出口', () => {
  const [p1, branch, p2, p3, end] = [id('c1'), id('c2'), id('c3'), id('c4'), id('c5')]
  const flow = {
    entry: p1,
    nodes: [
      { type: 'page', id: p1, question_ids: [], next: branch },
      {
        type: 'branch',
        id: branch,
        branches: [
          { when: { op: 'boolean', value: true }, target: p3 },
          { when: { op: 'boolean', value: false }, target: p2 },
        ],
        otherwise: p2,
      },
      { type: 'page', id: p2, question_ids: [], next: p3 },
      { type: 'page', id: p3, question_ids: [], next: end },
      { type: 'end', id: end, outcome: 'complete' },
    ],
  }
  const inserted = insertFlowNode(flow, 1, 'branch')
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [p1, branch, inserted, p2, p3, end],
  )
  assert.equal(flow.nodes[1].branches[0].target, p3, '指向其他节点时不误改')
  assert.equal(flow.nodes[1].branches[1].target, inserted)
  assert.equal(flow.nodes[1].otherwise, inserted)
  assert.deepEqual(flow.nodes[2], {
    type: 'branch',
    id: inserted,
    branches: [{ when: { op: 'boolean', value: true }, target: p2 }],
    otherwise: p2,
  })
})

test('插入页面时前驱分支的全部出口都回接', () => {
  const [p1, branch, p2, end] = [id('d1'), id('d2'), id('d3'), id('d4')]
  const flow = {
    entry: p1,
    nodes: [
      { type: 'page', id: p1, question_ids: [], next: branch },
      {
        type: 'branch',
        id: branch,
        branches: [
          { when: { op: 'boolean', value: true }, target: p2 },
          { when: { op: 'boolean', value: false }, target: p2 },
        ],
        otherwise: p2,
      },
      { type: 'page', id: p2, question_ids: [], next: end },
      { type: 'end', id: end, outcome: 'complete' },
    ],
  }
  const inserted = insertFlowNode(flow, 1, 'page')
  assert.deepEqual(
    flow.nodes[1].branches.map((entry) => entry.target),
    [inserted, inserted],
  )
  assert.equal(flow.nodes[1].otherwise, inserted)
  assert.equal(flow.nodes[2].next, p2)
})

test('插入结束节点仍回接前驱出口', () => {
  const flow = linearFlow()
  const [p1, p2, end] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 1, 'end')
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [p1, p2, inserted, end],
  )
  assert.deepEqual(flow.nodes[2], { type: 'end', id: inserted, outcome: 'complete' })
  assert.equal(flow.nodes[0].next, p2, '入口页面的出口不指向目标')
  assert.equal(flow.nodes[1].next, inserted, '原后继的前驱出口回接到新节点')
})

test('在末尾插入时后继为空并保持原有出口', () => {
  const flow = linearFlow()
  const [, p2, end] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 2, 'page')
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [id('b1'), p2, end, inserted],
  )
  assert.deepEqual(flow.nodes[3], { type: 'page', id: inserted, question_ids: [], next: '' })
  assert.equal(flow.nodes[1].next, end, '前驱并不指向空目标，不能被改写')
})

test('插入分支时页面出口指向新分支', () => {
  const flow = linearFlow()
  const [, p2] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 0, 'branch')
  assert.equal(flow.nodes[0].next, inserted)
  assert.deepEqual(flow.nodes[1].branches, [{ when: { op: 'boolean', value: true }, target: p2 }])
  assert.equal(flow.nodes[1].otherwise, p2)
})
