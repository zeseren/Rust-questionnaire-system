//! 流程拓扑插入：新节点接管前驱的实际出口（页面 next 或分支 otherwise），其余跳转保持不变。
import assert from 'node:assert/strict'
import test from 'node:test'
import { insertFlowNode } from '../../src/flow-tools.ts'

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
  assert.notEqual(inserted, p1)
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [p1, inserted, p2, end],
  )
  assert.equal(flow.nodes[0].next, inserted)
  assert.deepEqual(flow.nodes[1], { type: 'page', id: inserted, question_ids: [], next: p2 })
})

test('在分支后插入分支只接管兜底出口', () => {
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
  assert.equal(flow.nodes[1].branches[0].target, p3, '条件出口不受影响')
  assert.equal(flow.nodes[1].branches[1].target, p2, '条件出口不受影响')
  assert.equal(flow.nodes[1].otherwise, inserted)
  assert.deepEqual(flow.nodes[2], {
    type: 'branch',
    id: inserted,
    branches: [{ when: { op: 'boolean', value: true }, target: p2 }],
    otherwise: p2,
  })
})

test('插入页面时只接管分支的兜底出口', () => {
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
    [p2, p2],
    '条件出口不受影响',
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
  assert.equal(flow.nodes[1].next, inserted, '前驱出口回接到新节点')
})

test('在结束节点后插入被拒绝', () => {
  const flow = linearFlow()
  const before = flow.nodes.map((node) => node.id)
  for (const type of ['page', 'branch', 'end']) {
    assert.equal(insertFlowNode(flow, 2, type), null)
  }
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    before,
    '被拒绝时不改动流程',
  )
})

test('页面出口为空时拒绝插入', () => {
  const flow = {
    entry: id('e1'),
    nodes: [{ type: 'page', id: id('e1'), question_ids: [], next: '' }],
  }
  assert.equal(insertFlowNode(flow, 0, 'page'), null)
  assert.equal(flow.nodes.length, 1)
})

test('插入分支时页面出口指向新分支', () => {
  const flow = linearFlow()
  const [, p2] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 0, 'branch')
  assert.equal(flow.nodes[0].next, inserted)
  assert.deepEqual(flow.nodes[1].branches, [{ when: { op: 'boolean', value: true }, target: p2 }])
  assert.equal(flow.nodes[1].otherwise, p2)
})

test('在汇合页后插入页面时接管实际出边而不是数组后继', () => {
  // 与 routing 模板同构：分支两侧先汇合到 join，再统一结束。
  const [entry, branch, returning, newcomer, join, end] = [
    id('f1'),
    id('f2'),
    id('f3'),
    id('f4'),
    id('f5'),
    id('f6'),
  ]
  const flow = {
    entry,
    nodes: [
      { type: 'page', id: entry, question_ids: [], next: branch },
      {
        type: 'branch',
        id: branch,
        branches: [{ when: { op: 'boolean', value: true }, target: returning }],
        otherwise: newcomer,
      },
      { type: 'page', id: returning, question_ids: [], next: join },
      { type: 'page', id: newcomer, question_ids: [], next: join },
      { type: 'page', id: join, question_ids: [], next: end },
      { type: 'end', id: end, outcome: 'complete' },
    ],
  }
  const inserted = insertFlowNode(flow, 2, 'page')
  assert.ok(inserted)
  assert.deepEqual(
    flow.nodes.map((node) => node.id),
    [entry, branch, returning, inserted, newcomer, join, end],
  )
  assert.equal(flow.nodes[2].next, inserted, '前驱出口指向新页面')
  assert.equal(flow.nodes[3].next, join, '新页面接管原出口 join')
  assert.equal(flow.nodes[4].id, newcomer, '数组后继 newcomer 不受影响')
  assert.equal(flow.nodes[4].next, join)
  assert.equal(flow.nodes[5].next, end)
})
