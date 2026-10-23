//! 展示格式化：时间统一按 Asia/Shanghai，答案按题型映射原始值。
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  answerText,
  dateTime,
  exact,
  localInput,
  shareLink,
  toTimestamp,
} from '../../src/format.ts'
import {
  multipleChoiceQuestion,
  numberQuestion,
  singleChoiceQuestion,
  textQuestion,
} from '../helpers/documents.mjs'
import { installBrowserEnv } from '../helpers/browser.mjs'

test('dateTime 对空值与非法日期返回未设置，有效时间按上海时区展示', () => {
  for (const value of [null, undefined, '', 'not-a-date'])
    assert.equal(dateTime(value), '未设置', String(value))
  const formatted = dateTime('2024-01-02T03:04:00Z')
  assert.match(formatted, /2024/)
  assert.match(formatted, /11:04/)
})

test('exact 保留零值并只对空值降级', () => {
  assert.equal(exact(0), '0')
  assert.equal(exact(null), '暂无数据')
  assert.equal(exact(undefined), '暂无数据')
})

test('answerText 映射选项标签并回退未知身份', () => {
  const single = singleChoiceQuestion()
  const multiple = multipleChoiceQuestion()
  assert.equal(answerText(single, null), '未作答')
  assert.equal(answerText(single, undefined), '未作答')
  assert.equal(answerText(single, single.options[1].id), '乙')
  assert.equal(answerText(single, 'unknown-id'), 'unknown-id')
  assert.equal(
    answerText(
      multiple,
      multiple.options.map((option) => option.id),
    ),
    '丙、丁',
  )
  assert.equal(answerText(multiple, [multiple.options[0].id, 'unknown-id']), '丙、unknown-id')
  assert.equal(answerText(textQuestion(), '原文'), '原文')
  assert.equal(answerText(numberQuestion(), 0), '0')
  assert.equal(answerText(numberQuestion(), 12.5), '12.5')
})

test('localInput 与 toTimestamp 在 +08:00 上往返', () => {
  assert.equal(localInput(null), '')
  assert.equal(localInput(''), '')
  assert.equal(localInput('2024-01-02T03:04:00Z'), '2024-01-02T11:04')
  assert.equal(localInput('2024-01-01T20:00:00Z'), '2024-01-02T04:00')
  assert.equal(toTimestamp(''), null)
  assert.equal(toTimestamp('2024-01-02T11:04'), '2024-01-02T11:04:00+08:00')
  const timestamp = toTimestamp(localInput('2024-01-02T03:04:00Z'))
  assert.ok(timestamp)
  assert.equal(new Date(timestamp).toISOString(), '2024-01-02T03:04:00.000Z')
})

test('shareLink 以当前站点来源拼接并转义身份', (t) => {
  const env = installBrowserEnv(t)
  // 环境桩统一管理全局 `location`，这里固定一个带端口的来源验证拼接。
  env.location.origin = 'https://survey.example:8443'
  assert.equal(shareLink('abc'), 'https://survey.example:8443/s/abc')
  assert.equal(shareLink('a/b c'), 'https://survey.example:8443/s/a%2Fb%20c')
})
