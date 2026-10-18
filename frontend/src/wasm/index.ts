//! wasm 引擎的唯一入口：浏览器/Vite 走 wasm-pack 的 bundler 胶水，node --test 走
//! `_bg.js` + 手动实例化（bundler 产物里的 `.wasm` 静态导入只有打包器能解析）。
//!
//! 公开面与删除前的 TS 实现同名同签名：入参用 `json.ts` 的原生编解码压成 JSON
//! 字符串传过边界，返回的信封 `{"ok":…} | {"error":…}` 再还原成领域对象或抛出既有
//! 的错误类。数字统一为原生 `number`（有限双精度浮点数）。
import { decode, encode } from '../json.ts'
import { LogicError, LogicTextError } from '../logic.ts'
import type { LogicErrorCode } from '../logic.ts'
import type {
  DocumentLogic,
  DocumentLogicValues,
  EvaluatedResponse,
  FlowEvent,
  FlowSnapshot,
  LogicValue,
  LogicValueType,
  Question,
  SurveyDocument,
} from '../types.ts'

/** 领域错误信封；字段与 Rust `LogicError` / `LogicTextError` 的序列化一致。 */
interface EnvelopeError {
  code: LogicErrorCode
  path: string
  question_id: string | null
  message?: string
  line?: number
  column?: number
}
type Envelope<T> = { ok: T } | { error: EnvelopeError }

/** wasm 侧导出的最小形状；bundler 胶水与 node 手装实例共用同一份类型。 */
interface WasmEngine {
  parse_logic_text(source: string, questions_json: string): string
  format_logic_text(logic_json: string, questions_json: string): string
  compile_document_logic(logic_json: string, questions_json: string): string
  compile_expression(raw_json: string, questions_json: string): string
  evaluate_expression(raw_json: string, questions_json: string, answers_json: string): string
  evaluate_document_logic(logic_json: string, questions_json: string, answers_json: string): string
  renew_document_ids(document_json: string): string
  FlowSession: new (document_json: string) => WasmFlowSession
}
interface WasmFlowSession {
  dispatch(event_json: string): string
  snapshot(): string
  evaluate_answers(answers_json: string): string
}
/** wasm 模块的导入模块名就是 `_bg.js`，其导出即胶水提供的导入表。 */
interface WasmGlue extends WasmEngine, Record<string, WebAssembly.ImportValue> {
  __wbg_set_wasm(exports: WebAssembly.Exports): void
}

// 顶层 await：模块求值结束即引擎就绪，导入方总能直接调用下面的导出。
const engine = await loadEngine()

async function loadEngine(): Promise<WasmEngine> {
  // node --test 没有打包器：bundler 胶水的 `.wasm` 静态导入会直接失败，改为读文件手动实例化。
  // 这里必须用动态 import：静态 import 会在 node 下求值 `./pkg/logic_wasm.js`，而它依赖
  // 打包器的 wasm ESM 支持（node 的 wasm 模块集成仍需实验开关），node 侧会直接加载失败。
  if (typeof process !== 'undefined' && process.versions?.node) return loadNode()
  // Vite/浏览器：胶水负责把 `.wasm` 接进模块图（Vite 的 wasm 插件会生成导入表）。
  // 打包产物与 `WasmEngine` 同名同签名，动态导入的模块命名空间即满足契约。
  const module: WasmEngine = await import('./pkg/logic_wasm.js')
  return module
}

/** bundler 产物没有 `initSync`（按打包器契约生成），因此自己提供导入表完成实例化。 */
async function loadNode(): Promise<WasmEngine> {
  // `node:` 内置模块在浏览器构建里不存在，只能按环境动态加载；静态 import 会让
  // Vite 客户端构建把它当作外部模块处理并在加载期即告警/失效。
  const { createRequire } = await import('node:module')
  const { readFileSync } = await import('node:fs')
  // `_bg.js` 没有随产物发布的类型声明，`createRequire` 顺带绕开 TS 的模块解析。
  const glue = createRequire(import.meta.url)('./pkg/logic_wasm_bg.js') as WasmGlue
  const bytes = readFileSync(new URL('./pkg/logic_wasm_bg.wasm', import.meta.url))
  const { instance } = await WebAssembly.instantiate(bytes, {
    './logic_wasm_bg.js': glue,
  })
  glue.__wbg_set_wasm(instance.exports)
  return glue
}

/** 解析信封：成功取 `ok`，失败按 `code` 还原成既有错误类。 */
function unwrap<T>(text: string): T {
  const envelope = decode<Envelope<T>>(text)
  if ('ok' in envelope) return envelope.ok
  throw revive(envelope.error)
}

function revive(error: EnvelopeError): LogicError | LogicTextError {
  if (typeof error.line === 'number' && typeof error.column === 'number')
    return new LogicTextError(
      error.code,
      error.line,
      error.column,
      error.path,
      error.question_id,
      error.message,
    )
  return new LogicError(error.code, error.path, error.question_id)
}

/** `FlowSession` 构造失败只能抛出携带 JSON 错误串的 JS 值（wasm-bindgen 契约）。 */
function reviveThrown(reason: unknown): never {
  // wasm-bindgen 把 `JsValue::from_str` 的错误按原样抛出，因此这里常见的是字符串而非 Error。
  const text = typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : ''
  let error: LogicError | LogicTextError | undefined
  try {
    error = revive(decode<{ error: EnvelopeError }>(text).error)
  } catch {
    error = undefined
  }
  if (error) throw error
  // 非 JSON 信封说明这不是领域错误信封，原样抛出运行时报错，不能伪装成语法错误。
  throw reason instanceof Error ? reason : new Error(String(reason))
}

export interface CompiledExpression {
  readonly valueType: LogicValueType
  readonly dependencies: readonly string[]
  /** 变量上下文无法跨 JSON 边界回调，因此只接受答案（变量引用在编译期即被拒绝）。 */
  evaluate(answers: Readonly<Record<string, unknown>>): LogicValue
}

export interface CompiledDocumentLogic {
  /** 计算变量的拓扑顺序；声明顺序不决定执行顺序。 */
  readonly variableOrder: readonly string[]
  /**
   * 输入须已规范化并排除不适用答案；结果不代表填写路径或提交资格。
   * 每次调用都在 wasm 内重新编译规则包，只服务编辑期诊断与回归断言。
   */
  evaluate(answers: Readonly<Record<string, unknown>>): DocumentLogicValues
}

/** 解析文本 DSL；失败信封额外携带行列号，因此语法/语义错误抛 `LogicTextError`。 */
export function parseLogicText(source: string, questions: readonly Question[]): DocumentLogic {
  return unwrap<DocumentLogic>(engine.parse_logic_text(source, encode(questions)))
}

/** 把规则 JSON 格式化回文本 DSL；输出与后端逐字节一致。 */
export function formatLogicText(logic: DocumentLogic, questions: readonly Question[]): string {
  return unwrap<string>(engine.format_logic_text(encode(logic), encode(questions)))
}

/** 保存前的诊断预检：编译整份规则，失败抛 `LogicError`。 */
export function compileDocumentLogic(
  logic: DocumentLogic,
  questions: readonly Question[],
): CompiledDocumentLogic {
  const logicJson = encode(logic),
    questionsJson = encode(questions)
  const compiled = unwrap<{ variable_order: string[] }>(
    engine.compile_document_logic(logicJson, questionsJson),
  )
  return {
    variableOrder: compiled.variable_order,
    evaluate: (answers) =>
      unwrap<DocumentLogicValues>(
        engine.evaluate_document_logic(logicJson, questionsJson, encode(answers)),
      ),
  }
}

/** 编译单个表达式包；类型与依赖供表达式编辑器提示。 */
export function compileExpression(
  raw: unknown,
  questions: readonly Question[],
): CompiledExpression {
  const rawJson = encode(raw),
    questionsJson = encode(questions)
  const compiled = unwrap<{ value_type: LogicValueType; dependencies: string[] }>(
    engine.compile_expression(rawJson, questionsJson),
  )
  return {
    valueType: compiled.value_type,
    dependencies: compiled.dependencies,
    evaluate: (answers) =>
      unwrap<LogicValue>(engine.evaluate_expression(rawJson, questionsJson, encode(answers))),
  }
}

/** 重写整份问卷身份：复制问卷/规则时生成独立 ID 并同步全部引用。 */
export function renewDocumentIds(document: SurveyDocument): SurveyDocument {
  return unwrap<SurveyDocument>(engine.renew_document_ids(encode(document)))
}

/** 有状态填写会话：状态留在 wasm 线性内存，接口与删除前的 TS 类一致。 */
export class FlowSession {
  #session: WasmFlowSession

  constructor(document: SurveyDocument) {
    let session: WasmFlowSession
    try {
      session = new engine.FlowSession(encode(document))
    } catch (reason) {
      throw reviveThrown(reason)
    }
    this.#session = session
  }

  /** 返回独立快照，改动快照不会影响会话。 */
  snapshot(): FlowSnapshot {
    return unwrap<FlowSnapshot>(this.#session.snapshot())
  }

  /** 事件失败不改状态。 */
  dispatch(event: FlowEvent): void {
    unwrap<null>(this.#session.dispatch(encode(event)))
  }

  /** 从最终输入重建路径，不读取或修改当前会话的历史与输入。 */
  evaluateAnswers(answers: readonly { question_id: string; value: unknown }[]): EvaluatedResponse {
    return unwrap<EvaluatedResponse>(this.#session.evaluate_answers(encode(answers)))
  }
}
