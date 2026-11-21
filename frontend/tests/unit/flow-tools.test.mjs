//! 流程拓扑工具：插入节点接管前驱的实际出口；题目归属保持页面拼接顺序等于题目顺序。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  assignablePageIds,
  assignQuestion,
  insertFlowNode,
  insertionExit,
} from '../../src/flow-tools.ts'

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

// 在页面后插入页面会回接前驱并返回新身份
test('inserting a page after a page reconnects the predecessor and returns a new identity', () => {
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

// 在分支后插入分支只接管兜底出口
test('inserting a branch after a branch takes over only the fallback exit', () => {
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

// 插入页面时只接管分支的兜底出口
test('inserting a page takes over only the branch fallback exit', () => {
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

// 插入结束节点仍回接前驱出口
test('inserting an end node still reconnects the predecessor exit', () => {
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

// 在结束节点后插入被拒绝
test('inserting after an end node is rejected', () => {
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

// 页面出口为空时拒绝插入
test('insertion is rejected when the page exit is empty', () => {
  const flow = {
    entry: id('e1'),
    nodes: [{ type: 'page', id: id('e1'), question_ids: [], next: '' }],
  }
  assert.equal(insertFlowNode(flow, 0, 'page'), null)
  assert.equal(flow.nodes.length, 1)
})

// 插入分支时页面出口指向新分支
test('inserting a branch points the page exit at the new branch', () => {
  const flow = linearFlow()
  const [, p2] = flow.nodes.map((node) => node.id)
  const inserted = insertFlowNode(flow, 0, 'branch')
  assert.equal(flow.nodes[0].next, inserted)
  assert.deepEqual(flow.nodes[1].branches, [{ when: { op: 'boolean', value: true }, target: p2 }])
  assert.equal(flow.nodes[1].otherwise, p2)
})

// 在汇合页后插入页面时接管实际出边而不是数组后继
test('inserting a page after a merge page takes over actual outgoing edges instead of array successors', () => {
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

// insertionExit 只对已接出口的页面与分支给出可接管的目标
test('insertionExit only offers takeover targets for pages and branches with a connected exit', () => {
  const [p1, branch, end] = [id('a1'), id('a2'), id('a3')]
  assert.equal(insertionExit({ type: 'page', id: p1, question_ids: [], next: end }), end)
  assert.equal(insertionExit({ type: 'page', id: p1, question_ids: [], next: '' }), null)
  assert.equal(insertionExit({ type: 'branch', id: branch, branches: [], otherwise: end }), end)
  assert.equal(insertionExit({ type: 'branch', id: branch, branches: [], otherwise: '' }), null)
  assert.equal(insertionExit({ type: 'end', id: end, outcome: 'complete' }), null)
})

function pagedFlow(groups) {
  const end = id('9f')
  const pages = groups.map((question_ids, index) => ({
    type: 'page',
    id: id(`9${index}`),
    question_ids: [...question_ids],
    next: end,
  }))
  pages.forEach((page, index) => {
    page.next = pages[index + 1]?.id ?? end
  })
  return { entry: pages[0].id, nodes: [...pages, { type: 'end', id: end, outcome: 'complete' }] }
}
const questions = ['q1', 'q2', 'q3'].map((name) => ({ id: name }))

// 每题一页时题目可以并入相邻页面，但不能越过其他题目
test('with one page per question, questions can merge into adjacent pages but not skip other questions', () => {
  const flow = pagedFlow([['q1'], ['q2'], ['q3']])
  const [p1, p2, p3] = flow.nodes.map((node) => node.id)
  assert.deepEqual([...assignablePageIds(flow, questions, 'q1')], [p1, p2])
  assert.deepEqual([...assignablePageIds(flow, questions, 'q2')], [p1, p2, p3])
  assert.deepEqual([...assignablePageIds(flow, questions, 'q3')], [p2, p3])

  assert.equal(assignQuestion(flow, questions, 'q3', p1), false, 'q3 不能越过 q2')
  assert.deepEqual(
    flow.nodes.slice(0, 3).map((node) => node.question_ids),
    [['q1'], ['q2'], ['q3']],
    '被拒绝时不改动流程',
  )

  assert.equal(assignQuestion(flow, questions, 'q2', p1), true)
  assert.deepEqual(
    flow.nodes.slice(0, 3).map((node) => node.question_ids),
    [['q1', 'q2'], [], ['q3']],
  )
  // 腾空的页面不再约束顺序：q3 现在可以继续并入第一页。
  assert.deepEqual([...assignablePageIds(flow, questions, 'q3')], [p1, p2, p3])
})

// 未归属的题目按全局顺序落位，页内保持题目顺序
test('unassigned questions land in global order while page-local order is preserved', () => {
  const flow = pagedFlow([['q1'], ['q3']])
  const [p1, p2] = flow.nodes.map((node) => node.id)
  assert.deepEqual([...assignablePageIds(flow, questions, 'q2')], [p1, p2])
  assert.equal(assignQuestion(flow, questions, 'q2', p2), true)
  assert.deepEqual(flow.nodes[1].question_ids, ['q2', 'q3'])

  assert.equal(assignablePageIds(flow, questions, '不存在的题目').size, 0)
  assert.equal(assignQuestion(flow, questions, 'q2', flow.nodes[2].id), false, '结束节点不是页面')
})
