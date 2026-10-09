<!-- 代码编辑器只负责文本交互，诊断来自同一 DSL 编译入口。 -->
<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Compartment, EditorState } from '@codemirror/state'
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from '@codemirror/view'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import {
  bracketMatching,
  HighlightStyle,
  indentOnInput,
  StreamLanguage,
  syntaxHighlighting,
} from '@codemirror/language'
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  snippetCompletion,
} from '@codemirror/autocomplete'
import type { CompletionContext } from '@codemirror/autocomplete'
import { lintGutter, setDiagnostics } from '@codemirror/lint'
import { search, searchKeymap } from '@codemirror/search'
import { tags } from '@lezer/highlight'
import type { TextBinding } from '../logic-text-tools'
const props = defineProps<{
  modelValue: string
  disabled?: boolean
  bindings: TextBinding[]
  diagnostic: { message: string; offset: number } | null
}>()
const emit = defineEmits<{
  'update:modelValue': [value: string]
  apply: []
  cursor: [line: number, column: number]
}>()
const host = ref<HTMLElement>()
const readonly = new Compartment()
let view: EditorView | undefined
const snippets = [
  ['answered', 'answered(${题目})', '题目已回答'],
  ['answer', 'answer(${题目})', '读取题目答案'],
  ['selected', 'selected(${题目}, ${选项})', '选中指定选项'],
  ['contains_any', 'contains_any(${题目}, [${选项}])', '选中任一选项'],
  ['contains_all', 'contains_all(${题目}, [${选项}])', '选中全部选项'],
  ['variable', 'variable(${变量})', '读取计算变量'],
  ['coalesce', 'coalesce(${值}, ${默认值})', '缺失时使用默认值'],
  ['length', 'length(${值})', '文本长度或选项数量'],
  ['all', 'all(${条件一}, ${条件二})', '全部条件成立'],
  ['any', 'any(${条件一}, ${条件二})', '任一条件成立'],
  ['number', 'number("${0}")', '保留原文的数字常量'],
  ['visible', 'visible ${题目} = ${条件};', '题目显示条件'],
  ['required', 'required ${题目} = ${条件};', '动态必填条件'],
  ['validate', 'validate ${题目} = ${条件};', '答案校验条件'],
  ['let', 'let ${变量}: boolean = ${条件};', '声明已绑定的变量'],
  ['page', 'page ${页面} [${题目}] -> ${下一节点};', '声明已绑定的页面'],
  [
    'branch',
    'branch ${节点} {\n  when ${条件} -> ${目标};\n  else -> ${默认目标};\n}',
    '有序条件分支',
  ],
  ['end', 'end ${节点} = complete;', '完成填写'],
]
function complete(context: CompletionContext) {
  const word = context.matchBefore(/[A-Za-z_][A-Za-z_0-9]*$/)
  if (!word && !context.explicit) return null
  return {
    from: word?.from ?? context.pos,
    options: [
      ...props.bindings.map((binding) => ({
        label: binding.alias,
        type: 'variable',
        detail: binding.label,
        boost: 2,
      })),
      ...snippets.map(([label, template, detail]) =>
        snippetCompletion(template!, { label: label!, detail, type: 'keyword' }),
      ),
      ...[
        'true',
        'false',
        'boolean',
        'number',
        'text',
        'choice',
        'choices',
        'complete',
        'screen_out',
        'entry',
        'bind',
      ].map((label) => ({ label, type: 'keyword' })),
    ],
    validFor: /^[A-Za-z_][A-Za-z_0-9]*$/,
  }
}
const language = StreamLanguage.define({
  token(stream) {
    if (stream.eatSpace()) return null
    if (stream.match('//')) {
      stream.skipToEnd()
      return 'comment'
    }
    if (stream.match(/"(?:[^"\\]|\\.)*(?:"|$)/)) return 'string'
    if (stream.match(/-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/)) return 'number'
    if (
      stream.match(
        /\b(?:dsl|semantics|bind|question|option|variable|node|let|visible|required|validate|entry|page|branch|when|else|end|complete|screen_out)\b/,
      )
    )
      return 'keyword'
    if (stream.match(/\b(?:true|false)\b/)) return 'bool'
    if (stream.match(/\b(?:boolean|number|text|choice|choices)\b/)) return 'typeName'
    if (stream.match(/[A-Za-z_][A-Za-z_0-9]*/)) return 'variableName'
    if (stream.match(/[-+*/=!<>&|]+/)) return 'operator'
    stream.next()
    return null
  },
  languageData: {
    commentTokens: { line: '//' },
    closeBrackets: { brackets: ['(', '[', '{', '"'] },
  },
})
const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: 'var(--violet)' },
  { tag: tags.string, color: 'var(--green)' },
  { tag: tags.number, color: 'var(--teal)' },
  { tag: tags.bool, color: 'var(--violet)' },
  { tag: tags.typeName, color: 'var(--teal)' },
  { tag: tags.comment, color: 'var(--text-tertiary)', fontStyle: 'italic' },
  { tag: tags.operator, color: 'var(--accent)' },
])
function diagnostics() {
  if (!view) return
  const diagnostic = props.diagnostic
  const from = Math.min(diagnostic?.offset ?? 0, view.state.doc.length)
  view.dispatch(
    setDiagnostics(
      view.state,
      diagnostic
        ? [
            {
              from,
              to: Math.min(from + 1, view.state.doc.length),
              severity: 'error',
              message: diagnostic.message,
            },
          ]
        : [],
    ),
  )
}
onMounted(() => {
  view = new EditorView({
    parent: host.value,
    doc: props.modelValue,
    extensions: [
      lineNumbers(),
      highlightActiveLineGutter(),
      history(),
      drawSelection(),
      highlightActiveLine(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      language,
      syntaxHighlighting(highlight),
      autocompletion({ override: [complete] }),
      lintGutter(),
      search(),
      readonly.of([
        EditorState.readOnly.of(!!props.disabled),
        EditorView.editable.of(!props.disabled),
      ]),
      EditorView.contentAttributes.of({
        'aria-label': '文本 DSL 编辑器',
        spellcheck: 'false',
        'aria-describedby': 'dsl-editor-help',
      }),
      EditorState.phrases.of({
        Find: '查找',
        Replace: '替换',
        next: '下一项',
        previous: '上一项',
        all: '全部',
        'match case': '区分大小写',
        regexp: '正则表达式',
        'by word': '全词匹配',
        replace: '替换',
        'replace all': '全部替换',
        close: '关闭',
        'No diagnostics': '未发现问题',
      }),
      keymap.of([
        {
          key: 'Mod-Enter',
          run: () => {
            if (!props.disabled) emit('apply')
            return true
          },
        },
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        ...completionKeymap,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) emit('update:modelValue', update.state.doc.toString())
        if (update.docChanged || update.selectionSet) {
          const head = update.state.selection.main.head,
            line = update.state.doc.lineAt(head)
          emit('cursor', line.number, [...line.text.slice(0, head - line.from)].length + 1)
        }
      }),
    ],
  })
  diagnostics()
})
watch(
  () => props.modelValue,
  (value) => {
    if (view && value !== view.state.doc.toString())
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } })
  },
)
watch(
  () => props.disabled,
  (disabled) =>
    view?.dispatch({
      effects: readonly.reconfigure([
        EditorState.readOnly.of(!!disabled),
        EditorView.editable.of(!disabled),
      ]),
    }),
)
watch(() => props.diagnostic, diagnostics)
onBeforeUnmount(() => view?.destroy())
function focus(offset = 0) {
  if (!view) return
  const position = Math.max(0, Math.min(offset, view.state.doc.length))
  view.dispatch({
    selection: { anchor: position },
    effects: EditorView.scrollIntoView(position, { y: 'center' }),
  })
  view.focus()
}
function insert(text: string) {
  if (!view || props.disabled) return
  view.dispatch(view.state.replaceSelection(text))
  view.focus()
}
defineExpose({ focus, insert })
</script>
<template>
  <div ref="host" class="dsl-code-editor" :class="{ 'is-readonly': disabled }"></div>
</template>
