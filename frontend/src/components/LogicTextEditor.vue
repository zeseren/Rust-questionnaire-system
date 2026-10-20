<!-- [入口在哪] 规则与跳题工作区顶部切换到「文本 DSL」即进入，左侧编辑、右侧引用速查。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 底部固定栏最左侧常驻低饱和「恢复当前规则文本」幽灵按钮（悬停变红并二次确认），随时丢弃未应用的文本。 -->
<!-- [主提交通道在哪] 底部固定栏右侧「应用文本规则」是面板内次级提交（class="button"），全局唯一主按钮仍是编辑器固定栏的「发布问卷」。 -->
<!-- 错误或尚未应用的文本不覆盖草稿；工作区切换保留缓冲区并阻止误保存。 -->
<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import type { DocumentLogic, Question } from '../types'
import { message } from '../api'
import { encode } from '../json'
import { LogicTextError } from '../logic'
import { formatLogicText, parseLogicText } from '../wasm/index'
import { logicTextSignature, textBindings, textErrorOffset } from '../logic-text-tools'
import Icon from './Icon.vue'
const DslCodeEditor = defineAsyncComponent(() => import('./DslCodeEditor.vue'))
const logic = defineModel<DocumentLogic>({ required: true })
const props = defineProps<{ questions: Question[]; disabled?: boolean }>()
const emit = defineEmits<{ pending: [value: boolean] }>()
const source = ref<{ focus: (offset: number) => void; insert: (text: string) => void }>()
const fileInput = ref<HTMLInputElement>()
const text = ref(''),
  baseline = ref(''),
  syncedLogic = ref(''),
  syncedQuestions = ref('')
const validation = shallowRef<{ message: string; offset: number; path?: string } | null>(null)
const checked = ref(false)
const importError = ref('')
const query = ref('')
const cursor = ref({ line: 1, column: 1 })
let timer: ReturnType<typeof setTimeout> | undefined
const questionSignature = computed(() =>
  encode(
    props.questions.map((question) => ({
      id: question.id,
      type: question.type,
      options: 'options' in question ? question.options.map((option) => option.id) : undefined,
    })),
  ),
)
const stale = computed(
  () =>
    syncedLogic.value !== logicTextSignature(logic.value, props.questions) ||
    syncedQuestions.value !== questionSignature.value,
)
const pending = computed(() => text.value !== baseline.value || stale.value)
const bindings = computed(() => textBindings(text.value, props.questions))
const visibleBindings = computed(() =>
  bindings.value
    .filter((binding) =>
      `${binding.alias} ${binding.label}`.toLowerCase().includes(query.value.toLowerCase()),
    )
    .slice(0, 60),
)
watch(pending, (value) => emit('pending', value), { immediate: true, flush: 'sync' })
function check(): DocumentLogic | null {
  if (timer) {
    clearTimeout(timer)
    timer = undefined
  }
  checked.value = true
  try {
    const result = parseLogicText(text.value, props.questions)
    validation.value = null
    return result
  } catch (reason) {
    validation.value = {
      message: message(reason, '文本无法应用'),
      offset:
        reason instanceof LogicTextError
          ? textErrorOffset(text.value, reason.line, reason.column)
          : 0,
      path: reason instanceof LogicTextError ? reason.path : undefined,
    }
    return null
  }
}
function scheduleCheck() {
  checked.value = false
  validation.value = null
  clearTimeout(timer)
  timer = setTimeout(check, 300)
}
watch([text, questionSignature], scheduleCheck, { flush: 'sync' })
function synchronize(current = logic.value) {
  baseline.value = text.value
  syncedLogic.value = logicTextSignature(current, props.questions)
  syncedQuestions.value = questionSignature.value
}
function generate() {
  try {
    text.value = formatLogicText(logic.value, props.questions)
    synchronize()
    importError.value = ''
    check()
  } catch (reason) {
    importError.value = message(reason, '请先修复可视化配置，再生成文本。')
  }
}
function restore() {
  if (pending.value && !window.confirm('恢复会放弃尚未应用的文本，重新生成当前规则。是否继续？'))
    return
  generate()
}
function apply() {
  if (props.disabled) return
  const result = check()
  if (!result) {
    source.value?.focus(validation.value?.offset ?? 0)
    return
  }
  logic.value = result
  synchronize(result)
  importError.value = ''
}
function format() {
  const parsed = check()
  if (!parsed) return
  if (!window.confirm('整理文本会生成统一别名并移除注释，规则含义保持不变。是否继续？')) return
  text.value = formatLogicText(parsed, props.questions)
  check()
}
function download() {
  const url = URL.createObjectURL(new Blob([text.value], { type: 'text/plain;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = 'survey-logic.dsl'
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
async function importText(event: Event) {
  const input = event.currentTarget
  if (!(input instanceof HTMLInputElement)) return
  const file = input.files?.[0]
  input.value = ''
  if (!file || props.disabled) return
  if (file.size > 1048576) {
    importError.value = '规则文本不能超过 1 MiB。'
    return
  }
  if (pending.value && !window.confirm('导入会替换尚未应用的文本，是否继续？')) return
  try {
    text.value = await file.text()
    importError.value = ''
    check()
  } catch {
    importError.value = '无法读取文件，请重新选择。'
  }
}
function updateCursor(line: number, column: number) {
  cursor.value = { line, column }
}
generate()
onBeforeUnmount(() => {
  clearTimeout(timer)
  emit('pending', false)
})
</script>
<template>
  <div class="dsl-workbench panel">
    <div class="dsl-toolbar">
      <div class="actions">
        <Icon name="document" />
        <strong>规则文本</strong>
        <span class="hint">DSL 1</span>
      </div>
      <div class="actions">
        <button type="button" class="text-button" :disabled="disabled" @click="fileInput?.click()">
          导入文本
        </button>
        <button type="button" class="text-button" @click="download">下载文本</button>
        <button
          type="button"
          class="text-button"
          :disabled="disabled || !checked || !!validation"
          @click="format"
        >
          整理文本
        </button>
        <input
          ref="fileInput"
          type="file"
          class="sr-only"
          tabindex="-1"
          accept=".dsl,.txt,text/plain"
          aria-label="导入当前问卷的规则文本"
          :disabled="disabled"
          @change="importText"
        />
      </div>
    </div>
    <p v-if="stale" class="notice">题目或规则已变化，请恢复当前规则文本，或核对引用后重新应用。</p>
    <p v-if="importError" class="notice" role="alert">{{ importError }}</p>
    <div class="dsl-layout">
      <div class="dsl-source-pane">
        <DslCodeEditor
          ref="source"
          v-model="text"
          :disabled="disabled"
          :bindings="bindings"
          :diagnostic="validation"
          @apply="apply"
          @cursor="updateCursor"
        />
        <div class="dsl-statusbar">
          <span>
            {{ checked ? (validation ? '发现 1 处问题' : '语法与规则检查通过') : '正在检查…' }}
          </span>
          <span>第 {{ cursor.line }} 行，第 {{ cursor.column }} 列</span>
        </div>
      </div>
      <aside class="dsl-reference">
        <h3>引用速查</h3>
        <p class="hint">点击插入到光标位置。</p>
        <input
          v-model="query"
          type="search"
          aria-label="搜索 DSL 引用"
          placeholder="搜索题目或别名"
        />
        <div class="dsl-reference-list">
          <button
            v-for="binding in visibleBindings"
            :key="`${binding.category}-${binding.alias}`"
            type="button"
            :disabled="disabled"
            :title="binding.label"
            @click="
              source?.insert(
                binding.category === 'question'
                  ? `answer(${binding.alias})`
                  : binding.category === 'variable'
                    ? `variable(${binding.alias})`
                    : binding.alias,
              )
            "
          >
            <code>{{ binding.alias }}</code>
            <span>{{ binding.label }}</span>
          </button>
          <p v-if="!visibleBindings.length" class="hint">暂无匹配引用。</p>
          <p v-if="visibleBindings.length === 60" class="hint">显示前 60 项，可搜索缩小范围。</p>
        </div>
        <details class="dsl-help">
          <summary>常用语法</summary>
          <p>
            <code>visible q2 = answered(q1);</code>
            回答第 1 题后显示第 2 题。
          </p>
          <p>
            <code>required q2 = selected(q1, q1_o1);</code>
            选择指定选项后必填。
          </p>
          <p>
            <code>validate q1 = answer(q1) &gt;= 0;</code>
            校验数字答案。
          </p>
          <p class="hint">
            示例别名需替换为当前绑定。新变量和节点可先在可视化配置中建立；手写时需要 bind 绑定身份。
          </p>
        </details>
      </aside>
    </div>
    <div v-if="validation" class="dsl-diagnostic" role="status">
      <div class="grow">
        <strong>{{ validation.message }}</strong>
        <p class="hint">{{ validation.path }}</p>
      </div>
      <button type="button" class="button" @click="source?.focus(validation.offset)">
        定位问题
      </button>
    </div>
    <div class="dsl-actions">
      <div>
        <div class="actions">
          <button
            type="button"
            class="text-button danger-text"
            :disabled="disabled"
            @click="restore"
          >
            恢复当前规则文本
          </button>
          <p :class="pending ? 'accent' : 'hint'" role="status">
            {{ pending ? '文本尚未应用，请应用后保存草稿。' : '文本已与当前规则同步。' }}
          </p>
        </div>
        <p id="dsl-editor-help" class="hint">
          Ctrl / ⌘ + Enter 应用 · Ctrl + Space 补全 · Ctrl / ⌘ + F 查找 · Tab 移至下一控件
        </p>
      </div>
      <div class="actions">
        <button type="button" class="button" :disabled="disabled || !pending" @click="apply">
          应用文本规则
        </button>
      </div>
    </div>
  </div>
</template>
