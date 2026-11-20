<!-- [入口在哪] 填写页与预览页的题面会话；普通问卷单页呈现，规则问卷只渲染当前节点可见的题目。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 底栏左侧常驻「上一步」（规则问卷）与「清空重填/重新预览」，核对页每道题旁可就地「修改」。 -->
<!-- [主提交通道在哪] 底栏右侧唯一主操作：普通问卷「提交答卷/检查填写结果」，规则问卷「下一步」或「确认提交答卷/确认预览结果」。 -->
<!-- 预览和填写共用同一会话；只有显式确认才交付当前有效答案。 -->
<script setup lang="ts">
import { computed, nextTick, ref, shallowRef, watch } from 'vue'
import { FlowInputs, replayEvents } from '../flow-inputs'
import { LogicError } from '../logic'
import type { Answer, FlowEvent, FlowSnapshot, Question, SurveyDocument } from '../types'
import { isAnswered } from '../answers'
import type { Inputs, InputValue } from '../answers'
import { message } from '../api'
import SurveyFields from './SurveyFields.vue'
const props = withDefaults(
  defineProps<{
    document: SurveyDocument
    disabled?: boolean
    submitDisabled?: boolean
    preview?: boolean
    errors?: Record<string, string>
    hideFooter?: boolean
    /** 上次未提交的填写进度；本地存储读回后形状不可信，由回放助手逐项校验。 */
    restore?: readonly unknown[]
  }>(),
  { errors: () => ({}), hideFooter: false, restore: () => [] },
)
const emit = defineEmits<{
  submit: [answers: Answer[]]
  change: [count: number]
  reset: []
  /** 暂存进度回放结果；仅当存在待回放事件时通知父级。 */
  replayed: [applied: number, total: number]
}>()
let inputs: FlowInputs | undefined
/** 规则问卷已应用的事件序列；普通问卷按每题最新输入派生，不保留按键历史。 */
let recorded: FlowEvent[] = []
const snapshot = shallowRef<FlowSnapshot>()
const values = ref<Inputs>({})
const unsettled = shallowRef<ReadonlySet<string>>(new Set())
const touched = ref<Set<string>>(new Set())
const attempted = ref(false)
const error = ref('')
const root = ref<HTMLElement>()
/** 没有流程定义时按单页普通问卷处理：提交前临时前进校验，成功后退回编辑态。 */
const plain = computed(() => !props.document.logic)
const answeredCount = computed(() => Object.values(values.value).filter(isAnswered).length)
function refresh() {
  snapshot.value = inputs?.snapshot()
  values.value = inputs?.values() ?? {}
  unsettled.value = inputs?.unsettled() ?? new Set()
  for (const id of snapshot.value?.cleared ?? []) touched.value.delete(id)
}
function buildSession(restore: readonly unknown[]) {
  inputs = undefined
  recorded = []
  error.value = ''
  touched.value = new Set()
  attempted.value = false
  let applied = 0
  try {
    const session = new FlowInputs(props.document)
    inputs = session
    const result = replayEvents(session, restore, plain.value)
    recorded = result.events
    applied = result.applied
  } catch (reason) {
    error.value = message(reason, '流程无法加载')
  }
  refresh()
  // 只有真实存在过暂存事件才提示；重置或空草稿不能复活旧通知。
  if (restore.length) emit('replayed', applied, restore.length)
  emit('change', answeredCount.value)
}
function reset() {
  if (props.disabled) return
  // 清空重填不再回放暂存进度；父级收到 reset 后会同步删除本地草稿。
  buildSession([])
  emit('reset')
}
function confirmReset() {
  if (props.disabled) return
  const hasAnswers = Object.values(values.value).some(isAnswered)
  if (!props.preview && (hasAnswers || (snapshot.value?.history.length ?? 0) > 1)) {
    if (!window.confirm('确定清空当前已填写的回答并从头开始吗？')) return
  }
  reset()
}
/** 当前进度的事件序列副本，供填写页按版本暂存。 */
function events(): FlowEvent[] {
  // 普通问卷只保留每题最新输入（含数字过渡原文）；逐键历史会让草稿体积按输入长度平方增长。
  if (plain.value) {
    const latest: FlowEvent[] = []
    for (const question of props.document.questions) {
      const value = values.value[question.id]
      if (!isAnswered(value)) continue
      latest.push({
        type: 'change',
        question_id: question.id,
        value: Array.isArray(value) ? [...value] : value,
      })
    }
    return latest
  }
  return recorded.map((event) =>
    event.type === 'change'
      ? {
          type: 'change',
          question_id: event.question_id,
          value: Array.isArray(event.value) ? [...event.value] : event.value,
        }
      : { ...event },
  )
}
function commit(id: string) {
  if (!inputs || props.disabled) return
  inputs.commit(id)
  refresh()
}
watch(
  [
    () => props.document,
    () => props.document.logic,
    () => props.document.questions,
    () => props.restore,
  ],
  () => buildSession(props.restore),
  { immediate: true },
)
const pageDocument = computed(() => {
  const current = props.document.logic?.flow?.nodes.find(
    (node) => node.id === snapshot.value?.current_node,
  )
  const ids =
    current?.type === 'page' ? current.question_ids : props.document.questions.map((q) => q.id)
  return {
    ...props.document,
    questions: props.document.questions
      .filter((q) => ids.includes(q.id) && snapshot.value?.visible.includes(q.id))
      .map((q) => ({ ...q, required: snapshot.value?.required.includes(q.id) ?? q.required })),
  }
})
const numbers = computed<Record<string, number>>(() => {
  const result: Record<string, number> = {}
  props.document.questions.forEach((question, index) => {
    result[question.id] = index + 1
  })
  return result
})
const fieldErrors = computed<Record<string, string>>(() => {
  const result: Record<string, string> = {}
  const current = snapshot.value
  for (const q of pageDocument.value.questions) {
    // 引擎还没见过的数字前缀按“输入中”处理，不显示上一轮遗留的校验错误；结算后照常显示。
    if (unsettled.value.has(q.id)) continue
    const serverError = props.errors[q.id]
    if (serverError) {
      result[q.id] = serverError
      continue
    }
    if (!attempted.value && !touched.value.has(q.id)) continue
    const detail = current?.error_messages?.[q.id]
    const code = current?.errors[q.id]
    const text = detail || (code && new LogicError(code, '$.answers', q.id).message)
    if (text) result[q.id] = text
  }
  return result
})
async function focus(id?: string) {
  await nextTick()
  const first = id ?? pageDocument.value.questions.find((q) => fieldErrors.value[q.id])?.id
  if (first) document.getElementById(`answer-${first}`)?.focus()
  else root.value?.focus()
}
function change(id: string, value: InputValue) {
  if (!inputs || props.disabled) return
  try {
    inputs.change(id, value)
    if (!plain.value)
      recorded.push({
        type: 'change',
        question_id: id,
        value: Array.isArray(value) ? [...value] : value,
      })
    touched.value.add(id)
    error.value = ''
  } catch (reason) {
    error.value = message(reason, '修改失败')
  }
  refresh()
  emit('change', answeredCount.value)
}
function answerLabel(question: Question) {
  const value = snapshot.value?.answers[question.id]
  if ('options' in question)
    return (
      question.options
        .filter((o) => (Array.isArray(value) ? value.includes(o.id) : value === o.id))
        .map((o) => o.label)
        .join('、') || '未回答'
    )
  return value ?? '未回答'
}
function navigate(direction: 'next' | 'previous') {
  if (!inputs || props.disabled) return
  attempted.value = direction === 'next'
  try {
    inputs[direction]()
    recorded.push({ type: direction })
    attempted.value = false
    error.value = ''
  } catch (reason) {
    error.value = message(reason, '无法继续填写')
  }
  refresh()
  emit('change', answeredCount.value)
  void focus()
}
function submissionAnswers(): Answer[] | undefined {
  if (!inputs || props.disabled || props.submitDisabled) return
  if (!snapshot.value?.outcome) {
    if (!plain.value) {
      navigate('next')
      return
    }
    // 普通问卷没有核对页：next() 只做本页必填与格式校验，成功后立刻退回编辑态。
    attempted.value = true
    try {
      inputs.next()
    } catch (reason) {
      error.value = message(reason, '无法提交')
      refresh()
      void focus()
      return
    }
    attempted.value = false
    refresh()
    if (!snapshot.value?.outcome) return
  }
  if (plain.value) error.value = ''
  try {
    return inputs.submissionAnswers()
  } catch (reason) {
    error.value = message(reason, '无法提交')
    void focus()
  } finally {
    // 普通问卷的临时 next 不属于草稿导航：交卷或失败后都退回可编辑页。
    if (plain.value) {
      try {
        inputs.previous()
      } catch {}
      refresh()
    }
  }
}
function confirm() {
  const answers = submissionAnswers()
  if (answers) emit('submit', answers)
}
async function focusQuestion(id: string) {
  if (!inputs || props.disabled) return
  while (!pageDocument.value.questions.some((q) => q.id === id) || snapshot.value?.outcome) {
    if ((snapshot.value?.history.length ?? 0) <= 1) break
    inputs.previous()
    recorded.push({ type: 'previous' })
    refresh()
  }
  attempted.value = true
  await focus(id)
}
defineExpose({ submissionAnswers, focusQuestion, reset, events })
</script>
<template>
  <div ref="root" tabindex="-1" class="flow-fields">
    <p v-if="error" class="notice mt" role="alert">{{ error }}</p>
    <template v-if="snapshot">
      <template v-if="!snapshot.outcome || plain">
        <p v-if="!plain" class="hint mt" aria-live="polite">
          当前第 {{ snapshot.history.length }} 步 · 只需填写当前显示的题目
        </p>
        <SurveyFields
          v-model="values"
          :document="pageDocument"
          :numbers="numbers"
          :errors="fieldErrors"
          :disabled="disabled"
          @change="change"
          @commit="commit"
        />
        <p v-if="!pageDocument.questions.length" class="notice mt">
          本页没有需要回答的问题，请继续。
        </p>
      </template>
      <section v-else class="panel mt" aria-live="polite">
        <h2>{{ snapshot.outcome === 'complete' ? '填写完成，请确认提交' : '本次填写已结束' }}</h2>
        <p class="subtle mt">
          {{
            snapshot.outcome === 'complete'
              ? '可以返回核对或修改。确认提交后才会保存答卷。'
              : '你的回答进入了筛选结束分支。确认提交后会生成回执，但不计入各题统计。'
          }}
        </p>
        <dl v-if="snapshot.outcome === 'complete'" class="flow-review">
          <template
            v-for="question in document.questions.filter((q) => snapshot?.visible.includes(q.id))"
            :key="question.id"
          >
            <dt>{{ question.title }}</dt>
            <dd>
              <span>{{ answerLabel(question) }}</span>
              <button
                type="button"
                class="text-button accent"
                :disabled="disabled"
                @click="focusQuestion(question.id)"
              >
                修改
              </button>
            </dd>
          </template>
        </dl>
      </section>
      <div v-if="!hideFooter" class="fill-footer actions between">
        <div class="actions fill-footer-left">
          <button
            v-if="!plain && snapshot.history.length > 1"
            type="button"
            class="button"
            :disabled="disabled"
            @click="navigate('previous')"
          >
            上一步
          </button>
          <button
            type="button"
            class="text-button danger-text"
            :disabled="disabled"
            @click="confirmReset"
          >
            {{ preview ? '重新预览' : '清空重填' }}
          </button>
          <span v-if="plain" class="hint">
            已回答 {{ answeredCount }} / {{ document.questions.length }} 题 ·
            标注“必填”的题目必须回答
          </span>
        </div>
        <div class="actions fill-footer-right">
          <button
            v-if="plain && preview"
            type="button"
            class="button"
            :disabled="disabled || submitDisabled"
            @click="confirm"
          >
            检查填写结果
          </button>
          <button
            v-else-if="plain"
            type="submit"
            class="button button-primary"
            :disabled="disabled || submitDisabled"
          >
            提交答卷
          </button>
          <button
            v-else-if="!snapshot.outcome"
            type="button"
            class="button button-primary"
            :disabled="disabled"
            @click="navigate('next')"
          >
            下一步
          </button>
          <button
            v-else
            type="button"
            class="button button-primary"
            :disabled="disabled || submitDisabled"
            @click="confirm"
          >
            {{
              preview
                ? '确认预览结果'
                : snapshot.outcome === 'screen_out'
                  ? '确认提交（筛选结束）'
                  : '确认提交答卷'
            }}
          </button>
        </div>
      </div>
    </template>
  </div>
</template>
