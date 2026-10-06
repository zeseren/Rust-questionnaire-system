//! 文本只生成统一 AST；别名显式绑定 UUID，解析预算与语义编译共同守住边界。
import { compileDocumentLogic } from './document-logic.ts'
import type { DocumentLogic } from './document-logic.ts'
import type { FlowNode } from './flow-compiler.ts'
import { checkExpressionBudget, LogicError } from './logic.ts'
import type { LogicErrorCode, LogicValueType } from './logic.ts'
import type { Question } from './types'

type Category = 'question' | 'option' | 'variable' | 'node'
type Token = { raw: string; kind: 'word' | 'number' | 'string' | 'symbol' | 'end'; offset: number }
export class LogicTextError extends Error {
  readonly code: LogicErrorCode
  readonly line: number
  readonly column: number
  readonly path: string
  readonly question_id: string | null
  constructor(
    code: LogicErrorCode,
    line: number,
    column: number,
    path: string,
    question_id: string | null,
    message: string,
  ) {
    super(`${message}（第 ${line} 行，第 ${column} 列）`)
    this.name = 'LogicTextError'
    this.code = code
    this.line = line
    this.column = column
    this.path = path
    this.question_id = question_id
  }
}
function failure(
  source: string,
  offset: number,
  code: LogicErrorCode,
  path = '$',
  question: string | null = null,
  message?: string,
): LogicTextError {
  const lines = source.slice(0, offset).split('\n')
  return new LogicTextError(
    code,
    lines.length,
    [...lines.at(-1)!].length + 1,
    path,
    question,
    message ?? new LogicError(code, path, question).message,
  )
}
function tokenize(source: string): Token[] {
  if (new TextEncoder().encode(source).length > 1048576) throw failure(source, 0, 'limit_exceeded')
  const tokens: Token[] = []
  let offset = 0
  while (offset < source.length) {
    const rest = source.slice(offset)
    const ignored = /^(?:[ \t\r\n]+|\/\/[^\n]*)/.exec(rest)
    if (ignored) {
      offset += ignored[0].length
      continue
    }
    const match =
      /^(?:([A-Za-z_][A-Za-z_0-9]*)|((?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)|("(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*")|(->|&&|\|\||==|!=|<=|>=|[{}()[\],;:=!<>+*/-]))/.exec(
        rest,
      )
    if (!match) throw failure(source, offset, 'invalid_syntax')
    const kind = match[1] ? 'word' : match[2] ? 'number' : match[3] ? 'string' : 'symbol'
    if (kind === 'word' && match[0].length > 64) throw failure(source, offset, 'limit_exceeded')
    tokens.push({ raw: match[0], kind, offset })
    if (tokens.length > 65536) throw failure(source, offset, 'limit_exceeded')
    offset += match[0].length
  }
  tokens.push({ raw: '', kind: 'end', offset })
  return tokens
}
const binary: Record<string, [number, string, string]> = {
  '||': [1, 'any', ''],
  '&&': [2, 'all', ''],
  '==': [3, 'compare', 'eq'],
  '!=': [3, 'compare', 'ne'],
  '<': [3, 'compare', 'lt'],
  '<=': [3, 'compare', 'le'],
  '>': [3, 'compare', 'gt'],
  '>=': [3, 'compare', 'ge'],
  '+': [4, 'arithmetic', 'add'],
  '-': [4, 'arithmetic', 'subtract'],
  '*': [5, 'arithmetic', 'multiply'],
  '/': [5, 'arithmetic', 'divide'],
}
class Parser {
  #source: string
  #tokens: Token[]
  #position = 0
  #bindings = new Map<string, { category: Category; id: string }>()
  #locations: { path: string; offset: number }[] = []
  constructor(source: string) {
    this.#source = source
    this.#tokens = tokenize(source)
  }
  token() {
    return this.#tokens[this.#position]!
  }
  take() {
    const token = this.token()
    if (token.kind !== 'end') this.#position++
    return token
  }
  fail(code: LogicErrorCode = 'invalid_syntax', token = this.token()): never {
    throw failure(this.#source, token.offset, code)
  }
  accept(raw: string) {
    if (this.token().raw !== raw) return false
    this.take()
    return true
  }
  expect(raw: string) {
    if (!this.accept(raw)) this.fail()
  }
  word() {
    const token = this.take()
    if (token.kind !== 'word') this.fail('invalid_syntax', token)
    return token.raw
  }
  string() {
    const token = this.take()
    if (token.kind !== 'string') this.fail('invalid_syntax', token)
    const value = JSON.parse(token.raw) as string
    if (/[\0\uD800-\uDFFF]/u.test(value)) this.fail('invalid_syntax', token)
    return value
  }
  reference(category: Category) {
    const token = this.token(),
      alias = this.word(),
      binding = this.#bindings.get(alias)
    if (!binding || binding.category !== category) this.fail('invalid_reference', token)
    return binding.id
  }
  references(category: Category) {
    this.expect('[')
    const ids: string[] = []
    if (!this.accept(']')) {
      do {
        ids.push(this.reference(category))
      } while (this.accept(','))
      this.expect(']')
    }
    return ids
  }
  expression(min = 0, depth = 1): unknown {
    if (depth > 64) this.fail('limit_exceeded')
    let left = this.primary(depth)
    const check = () => {
      try {
        checkExpressionBudget(left, 1, { count: 0 })
      } catch (reason) {
        if (reason instanceof LogicError)
          throw failure(this.#source, this.token().offset, reason.code, reason.path)
        throw reason
      }
    }
    check()
    while (true) {
      const operator = binary[this.token().raw]
      if (!operator || operator[0] < min) break
      this.take()
      const right = this.expression(operator[0] + 1, depth + 1)
      const [, op, kind] = operator
      left =
        op === 'all' || op === 'any'
          ? { op, expressions: [left, right] }
          : { op, [op === 'compare' ? 'comparison' : 'operator']: kind, left, right }
      check()
    }
    return left
  }
  primary(depth: number): unknown {
    if (this.accept('(')) {
      const expression = this.expression(0, depth + 1)
      this.expect(')')
      return expression
    }
    if (this.accept('!')) return { op: 'not', expression: this.expression(6, depth + 1) }
    if (this.accept('-')) {
      const token = this.take()
      if (token.kind !== 'number') this.fail('invalid_syntax', token)
      return { op: 'number', value: `-${token.raw}` }
    }
    const token = this.take()
    if (token.kind === 'number') return { op: 'number', value: token.raw }
    if (token.kind === 'string') {
      this.#position--
      return { op: 'text', value: this.string() }
    }
    if (token.raw === 'true' || token.raw === 'false')
      return { op: 'boolean', value: token.raw === 'true' }
    if (token.kind !== 'word') this.fail('invalid_syntax', token)
    if (!this.accept('(')) {
      const binding = this.#bindings.get(token.raw)
      if (binding?.category !== 'variable') this.fail('invalid_reference', token)
      return { op: 'variable', variable_id: binding.id }
    }
    const op = token.raw
    let result: unknown
    if (op === 'answer' || op === 'answered')
      result = { op, question_id: this.reference('question') }
    else if (op === 'variable') result = { op, variable_id: this.reference('variable') }
    else if (op === 'number') result = { op, value: this.string() }
    else if (op === 'selected' || op === 'contains_any' || op === 'contains_all') {
      const question_id = this.reference('question')
      this.expect(',')
      result =
        op === 'selected'
          ? { op, question_id, option_id: this.reference('option') }
          : { op, question_id, option_ids: this.references('option') }
    } else if (op === 'all' || op === 'any') {
      const expressions: unknown[] = []
      if (this.token().raw !== ')')
        do {
          expressions.push(this.expression(0, depth + 1))
        } while (this.accept(','))
      result = { op, expressions }
    } else if (op === 'coalesce' || op === 'length') {
      const value = this.expression(0, depth + 1)
      if (op === 'coalesce') {
        this.expect(',')
        result = { op, value, fallback: this.expression(0, depth + 1) }
      } else result = { op, value }
    } else this.fail('invalid_syntax', token)
    this.expect(')')
    return result
  }
  parse(questions: readonly Question[]): DocumentLogic {
    this.expect('dsl')
    const dsl = this.take()
    this.expect('semantics')
    const semantics = this.take()
    this.expect(';')
    if (dsl.kind !== 'number' || semantics.kind !== 'number') this.fail('invalid_syntax', dsl)
    const logic: DocumentLogic = {
      dsl_version: Number(dsl.raw),
      semantics_version: Number(semantics.raw),
      variables: [],
      rules: [],
    }
    const boundIds = new Set<string>()
    while (this.accept('bind')) {
      const token = this.token(),
        category = this.word()
      if (!['question', 'option', 'variable', 'node'].includes(category))
        this.fail('invalid_syntax', token)
      const alias = this.word()
      this.expect('=')
      const id = this.string()
      this.expect(';')
      if (
        !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id) ||
        /^0{8}-0{4}-0{4}-0{4}-0{12}$/.test(id) ||
        this.#bindings.has(alias) ||
        boundIds.has(id)
      )
        this.fail('invalid_reference', token)
      this.#bindings.set(alias, { category: category as Category, id })
      boundIds.add(id)
    }
    const nodes: FlowNode[] = [],
      definitions = new Set<string>()
    let entry: string | undefined
    while (this.token().kind !== 'end') {
      const start = this.token().offset,
        token = this.take()
      if (token.raw === 'let') {
        const id = this.reference('variable')
        if (definitions.has(id)) this.fail('invalid_reference', token)
        definitions.add(id)
        this.expect(':')
        const value_type = this.word()
        this.expect('=')
        if (!['boolean', 'number', 'text', 'choice', 'choices'].includes(value_type))
          this.fail('invalid_syntax', token)
        this.#locations.push({
          path: `$.logic.variables[${logic.variables.length}]`,
          offset: start,
        })
        logic.variables.push({
          id,
          value_type: value_type as LogicValueType,
          expression: this.expression(),
        })
        this.expect(';')
      } else if (['visible', 'required', 'validate'].includes(token.raw)) {
        const question_id = this.reference('question')
        let index = logic.rules.findIndex((rule) => rule.question_id === question_id)
        if (index < 0) {
          index = logic.rules.length
          logic.rules.push({ question_id })
        }
        const key = (
          { visible: 'visible_if', required: 'required_if', validate: 'validate' } as const
        )[token.raw as 'visible' | 'required' | 'validate']
        const rule = logic.rules[index]!
        if (Object.hasOwn(rule, key)) this.fail('invalid_syntax', token)
        this.#locations.push({ path: `$.logic.rules[${index}].${key}`, offset: start })
        this.expect('=')
        rule[key] = this.expression()
        this.expect(';')
      } else if (token.raw === 'entry') {
        if (entry !== undefined) this.fail('invalid_syntax', token)
        entry = this.reference('node')
        this.expect(';')
        this.#locations.push({ path: '$.logic.flow', offset: start })
      } else if (['page', 'branch', 'end'].includes(token.raw)) {
        const id = this.reference('node')
        if (definitions.has(id)) this.fail('invalid_reference', token)
        definitions.add(id)
        this.#locations.push({ path: `$.logic.flow.nodes[${nodes.length}]`, offset: start })
        if (token.raw === 'page') {
          const question_ids = this.references('question')
          this.expect('->')
          const next = this.reference('node')
          this.expect(';')
          nodes.push({ type: 'page', id, question_ids, next })
        } else if (token.raw === 'end') {
          this.expect('=')
          const outcome = this.word()
          this.expect(';')
          if (outcome !== 'complete' && outcome !== 'screen_out') this.fail('invalid_syntax', token)
          nodes.push({ type: 'end', id, outcome })
        } else {
          this.expect('{')
          const branches = []
          while (this.accept('when')) {
            const when = this.expression()
            this.expect('->')
            const target = this.reference('node')
            this.expect(';')
            branches.push({ when, target })
          }
          this.expect('else')
          this.expect('->')
          const otherwise = this.reference('node')
          this.expect(';')
          this.expect('}')
          nodes.push({ type: 'branch', id, branches, otherwise })
        }
      } else this.fail('invalid_syntax', token)
    }
    if (nodes.length || entry !== undefined) {
      if (!entry) this.fail('invalid_flow')
      logic.flow = { entry, nodes }
    }
    // 即使未使用，也验证题目和选项绑定，避免拼错别名绑定而得到似乎成功的文档。
    for (const binding of this.#bindings.values()) {
      const valid =
        binding.category === 'question'
          ? questions.some((q) => q.id === binding.id)
          : binding.category === 'option'
            ? questions.some((q) => 'options' in q && q.options.some((o) => o.id === binding.id))
            : definitions.has(binding.id)
      if (!valid) this.fail('invalid_reference', this.#tokens[0])
    }
    try {
      compileDocumentLogic(logic, questions)
    } catch (reason) {
      if (!(reason instanceof LogicError)) throw reason
      const location = this.#locations
        .filter((location) => reason.path.startsWith(location.path))
        .sort((a, b) => b.path.length - a.path.length)[0]
      throw failure(
        this.#source,
        location?.offset ?? 0,
        reason.code,
        reason.path,
        reason.question_id,
      )
    }
    return logic
  }
}

/** 解析后立即编译，只有结构、类型、引用及流程均合法时才返回规则包。 */
export function parseLogicText(source: string, questions: readonly Question[]): DocumentLogic {
  return new Parser(source).parse(questions)
}

/** 生成确定性的可读别名，绑定仍由 UUID 定义；常量原文与表达式分组保持不变。 */
export function formatLogicText(logic: DocumentLogic, questions: readonly Question[]): string {
  compileDocumentLogic(logic, questions)
  const aliases = new Map<string, string>(),
    lines = [`dsl ${logic.dsl_version} semantics ${logic.semantics_version};`]
  function bind(category: Category, id: string, alias: string) {
    aliases.set(id, alias)
    lines.push(`bind ${category} ${alias} = ${JSON.stringify(id)};`)
  }
  questions.forEach((q, i) => {
    bind('question', q.id, `q${i + 1}`)
    if ('options' in q) q.options.forEach((o, j) => bind('option', o.id, `q${i + 1}_o${j + 1}`))
  })
  logic.variables.forEach((v, i) => bind('variable', v.id, `v${i + 1}`))
  logic.flow?.nodes.forEach((n, i) => bind('node', n.id, `n${i + 1}`))
  const ref = (id: unknown) => {
    const alias = aliases.get(String(id))
    if (!alias) throw new LogicError('invalid_reference', '$')
    return alias
  }
  function expression(raw: unknown): string {
    const node = raw as Record<string, unknown>
    switch (node.op) {
      case 'boolean':
        return String(node.value)
      case 'number':
        return `number(${JSON.stringify(node.value)})`
      case 'text':
        return JSON.stringify(node.value)
      case 'answer':
      case 'answered':
        return `${node.op}(${ref(node.question_id)})`
      case 'variable':
        return `variable(${ref(node.variable_id)})`
      case 'selected':
        return `selected(${ref(node.question_id)}, ${ref(node.option_id)})`
      case 'contains_any':
      case 'contains_all':
        return `${node.op}(${ref(node.question_id)}, [${(node.option_ids as string[]).map(ref).join(', ')}])`
      case 'not':
        return `!(${expression(node.expression)})`
      case 'all':
      case 'any':
        return `${node.op}(${(node.expressions as unknown[]).map(expression).join(', ')})`
      case 'coalesce':
        return `coalesce(${expression(node.value)}, ${expression(node.fallback)})`
      case 'length':
        return `length(${expression(node.value)})`
      default: {
        const operator = Object.entries(binary).find(
          ([, value]) => value[1] === node.op && value[2] === (node.comparison ?? node.operator),
        )?.[0]
        if (!operator) throw new LogicError('invalid_syntax', '$')
        return `(${expression(node.left)} ${operator} ${expression(node.right)})`
      }
    }
  }
  lines.push('')
  logic.variables.forEach((v) =>
    lines.push(`let ${ref(v.id)}: ${v.value_type} = ${expression(v.expression)};`),
  )
  for (const rule of logic.rules)
    for (const [field, keyword] of [
      ['visible_if', 'visible'],
      ['required_if', 'required'],
      ['validate', 'validate'],
    ] as const)
      if (rule[field] != null)
        lines.push(`${keyword} ${ref(rule.question_id)} = ${expression(rule[field])};`)
  if (logic.flow) {
    lines.push(`entry ${ref(logic.flow.entry)};`)
    for (const node of logic.flow.nodes) {
      if (node.type === 'page')
        lines.push(
          `page ${ref(node.id)} [${node.question_ids.map(ref).join(', ')}] -> ${ref(node.next)};`,
        )
      else if (node.type === 'end') lines.push(`end ${ref(node.id)} = ${node.outcome};`)
      else {
        lines.push(`branch ${ref(node.id)} {`)
        node.branches.forEach((b) =>
          lines.push(`  when ${expression(b.when)} -> ${ref(b.target)};`),
        )
        lines.push(`  else -> ${ref(node.otherwise)};`, '}')
      }
    }
  }
  return `${lines.join('\n')}\n`
}
