//! `newId` 的公开契约：标准 RFC 9562 UUIDv7，时间戳可读且同一毫秒内不重复。
import assert from 'node:assert/strict'
import test from 'node:test'
import { newId } from '../../src/uuid.ts'

const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/

test('newId 生成带版本与变体位的 UUIDv7，并嵌入当前毫秒时间戳', () => {
  const before = Date.now()
  const ids = Array.from({ length: 512 }, () => newId())
  const after = Date.now()
  const seen = new Set()
  for (const id of ids) {
    assert.match(id, UUID_V7)
    const timestamp = Number.parseInt(id.replaceAll('-', '').slice(0, 12), 16)
    assert.ok(timestamp >= before && timestamp <= after, `${id} 的时间戳落在生成区间内`)
    seen.add(id)
  }
  assert.equal(seen.size, ids.length, '随机位保证同一毫秒内不重复')
})

test('newId 生成的文本可被标准 UUID 解析并按 v7 读取', () => {
  const id = newId()
  const bytes = Uint8Array.from(id.replaceAll('-', '').match(/../g), (pair) =>
    Number.parseInt(pair, 16),
  )
  assert.equal(bytes.length, 16)
  assert.equal(bytes[6] >> 4, 7)
  assert.equal(bytes[8] >> 6, 0b10)
})
