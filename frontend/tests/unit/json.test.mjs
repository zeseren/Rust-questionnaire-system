//! JSON 边界：复制必须与原件完全隔离。
import assert from 'node:assert/strict'
import test from 'node:test'
import { copy } from '../../src/json.ts'

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
