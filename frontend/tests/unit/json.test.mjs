//! JSON 边界：编解码是唯一的反序列化入口，复制必须与原件完全隔离。
import assert from 'node:assert/strict'
import test from 'node:test'
import { copy, decode, encode } from '../../src/json.ts'

test('encode 与 decode 往返保留 JSON 值', () => {
  const value = {
    title: '中文与 emoji 😀',
    list: [1, 2.5, null, true, { nested: '文本' }],
  }
  assert.deepEqual(decode(encode(value)), value)
  assert.equal(encode({ a: 1 }), '{"a":1}', '字段顺序按对象插入顺序稳定输出')
})

test('decode 对非法文本抛出 SyntaxError', () => {
  assert.throws(() => decode('{'), SyntaxError)
  assert.throws(() => decode(''), SyntaxError)
})

test('copy 深拷贝嵌套对象与数组', () => {
  const original = { list: [1, { deep: 'x' }], nested: { a: 1 } }
  const cloned = copy(original)
  assert.deepEqual(cloned, original)
  assert.notEqual(cloned, original)
  assert.notEqual(cloned.list, original.list)
  assert.notEqual(cloned.list[1], original.list[1])
  cloned.list[1].deep = 'y'
  cloned.list.push(2)
  cloned.nested.a = 2
  assert.deepEqual(original, { list: [1, { deep: 'x' }], nested: { a: 1 } })
})

test('copy 按 JSON 语义归一非有限数字与负零', () => {
  assert.equal(copy(NaN), null)
  assert.equal(copy(Infinity), null)
  assert.ok(Object.is(copy(-0), 0), '负零经 JSON 序列化归一为正零')
})

test('copy 拒绝不可序列化的循环引用', () => {
  const cyclic = {}
  cyclic.self = cyclic
  assert.throws(() => copy(cyclic), TypeError)
})
