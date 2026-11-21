//! 未决提交与草稿的冻结作用域：匿名固定隔离，登录限次按载入账号隔离读写键。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  ANONYMOUS_SCOPE,
  draftKeyFor,
  freezePending,
  pendingKeyFor,
  scopeFor,
} from '../../src/pending-submission.ts'

const SURVEY_ID = '00000000-0000-4000-8000-000000000001'
const VERSION_ID = '00000000-0000-4000-8000-000000000002'
const ACCOUNT_A = '00000000-0000-4000-8000-00000000000a'
const ACCOUNT_B = '00000000-0000-4000-8000-00000000000b'

const submission = {
  version_id: VERSION_ID,
  idempotency_key: '00000000-0000-4000-8000-000000000003',
  answers: [],
}

// 匿名作用域固定且不采集登录账号
test('anonymous scope is fixed and does not capture login account', () => {
  assert.equal(scopeFor('anonymous', null), ANONYMOUS_SCOPE)
  assert.equal(scopeFor('anonymous', ACCOUNT_A), ANONYMOUS_SCOPE)
  assert.equal(scopeFor('login_once', ACCOUNT_A), ACCOUNT_A)
})

// 登录限次缺账号时不降级为匿名作用域
test('login-once scope without account does not degrade to anonymous scope', () => {
  assert.notEqual(scopeFor('login_once', null), ANONYMOUS_SCOPE)
  assert.equal(scopeFor('login_once', null), null)
})

// 登录限次草稿按载入账号隔离，匿名键保持原格式
test('login-once drafts are isolated per loaded account while anonymous key keeps its format', () => {
  const anonymous = draftKeyFor(SURVEY_ID, VERSION_ID, ANONYMOUS_SCOPE)
  assert.equal(anonymous, `wenxu:fill-draft:${SURVEY_ID}:${VERSION_ID}`)

  const a = draftKeyFor(SURVEY_ID, VERSION_ID, ACCOUNT_A)
  const b = draftKeyFor(SURVEY_ID, VERSION_ID, ACCOUNT_B)
  assert.notEqual(a, b, 'B 不能读写 A 的登录限次草稿')
  assert.ok(a.endsWith(`:${ACCOUNT_A}`))
  assert.ok(b.endsWith(`:${ACCOUNT_B}`))
})

// 未决提交按冻结账号分键，匿名不随登录身份变化
test('pending submissions key by frozen account and anonymous keys ignore login identity', () => {
  assert.equal(
    pendingKeyFor(SURVEY_ID, ANONYMOUS_SCOPE),
    `wenxu:fill-draft:${SURVEY_ID}:pending:anonymous`,
  )
  assert.notEqual(pendingKeyFor(SURVEY_ID, ACCOUNT_A), pendingKeyFor(SURVEY_ID, ACCOUNT_B))
})

// 存储载荷只接受完整 Submission，作用域由读取键决定
test('storage payload accepts only a complete Submission and scope comes from the read key', () => {
  assert.deepEqual(freezePending(submission, ACCOUNT_A), {
    scope: ACCOUNT_A,
    payload: submission,
  })

  for (const broken of [
    null,
    '文本',
    {},
    { version_id: VERSION_ID },
    { version_id: VERSION_ID, idempotency_key: 'x', answers: [{ question_id: 1, value: null }] },
  ]) {
    assert.equal(freezePending(broken, ACCOUNT_A), null, JSON.stringify(broken))
  }
})
