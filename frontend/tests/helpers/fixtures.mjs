//! 测试夹具读取：所有回归共用同一份 JSON，缓存只保存只读原文。
import { readFileSync } from 'node:fs'
import { LogicError } from '../../src/logic.ts'

/** 夹具在整个测试进程内只解析一次；调用方不得就地修改返回值。 */
const cache = new Map()

/** 读取 `fixtures/survey-logic/<name>.json`。 */
export function loadFixture(name) {
  if (!cache.has(name)) {
    const url = new URL(`../../../fixtures/survey-logic/${name}.json`, import.meta.url)
    cache.set(name, JSON.parse(readFileSync(url, 'utf8')))
  }
  return cache.get(name)
}

/** 取 `flows.json#documents[name]` 的深拷贝，供需要改写的测试使用。 */
export function readFlowDocument(name) {
  const documents = loadFixture('flows').documents
  const document = documents[name]
  if (!document) throw new Error(`未知流程夹具文档：${name}`)
  return structuredClone(document)
}

/** 按 `{ path, value }` 列表就地应用编辑，路径段支持对象键与数组下标。 */
export function applyEdits(document, edits) {
  for (const edit of edits) {
    const parent = edit.path.slice(0, -1).reduce((node, key) => node[key], document)
    parent[edit.path.at(-1)] = structuredClone(edit.value)
  }
  return document
}

/**
 * 执行只允许抛出领域错误的行为：返回 `LogicError`，成功返回 `null`。
 * 其他异常原样抛出，避免把运行时缺陷伪装成语义回归。
 */
export function captureDomainError(run) {
  try {
    run()
    return null
  } catch (error) {
    if (!(error instanceof LogicError)) throw error
    return error
  }
}

/** 抽取人工预期里稳定出现的错误字段；`message` 由文案测试单独覆盖。 */
export function domainErrorFields(error, expectedError) {
  const fields = { code: error.code, question_id: error.question_id }
  if (!expectedError || 'path' in expectedError) fields.path = error.path
  return fields
}
