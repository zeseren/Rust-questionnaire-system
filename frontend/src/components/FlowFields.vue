<!-- 预览和填写共用同一会话；只有显式确认才交付当前有效答案。 -->
<script setup lang="ts">
import { computed, nextTick, ref, shallowRef, watch } from 'vue'
import { FlowInputs } from '../flow-inputs'
import { LogicError } from '../logic'
import type { Answer, FlowSnapshot, Question, SurveyDocument } from '../types'
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
  }>(),
  { errors: () => ({}) },
)
const emit = defineEmits<{ submit: [answers: Answer[]]; change: [count: number] }>()
let inputs: FlowInputs | undefined
const snapshot = shallowRef<FlowSnapshot>()
const values = ref<Inputs>({})
const touched = ref<Set<string>>(new Set())
const attempted = ref(false)
const error = ref('')
const root = ref<HTMLElement>()
function refresh() {
  snapshot.value = inputs?.snapshot()
  values.value = inputs?.values() ?? {}
  for (const id of snapshot.value?.cleared ?? []) touched.value.delete(id)
}
function initSession() {
  inputs = undefined
  error.value = ''
  touched.value = new Set()
  attempted.value = false
  try {
    inputs = new FlowInputs(props.document)
  } catch (reason) {
    error.value = message(reason, '流程无法加载')
  }
  refresh()
  emit('change', 0)
}
function reset() {
  if (props.disabled) return
  initSession()
}
watch(() => props.document, initSession, { immediate: true })
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
const fieldErrors = computed<Record<string, string>>(() => {
  const result: Record<string, string> = {}
  for (const q of pageDocument.value.questions) {
    const code = snapshot.value?.errors[q.id]
    const text =
      props.errors[q.id] ||
      (code && (attempted.value || touched.value.has(q.id))
        ? new LogicError(code, '$.answers', q.id).message
        : '')
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
    touched.value.add(id)
    error.value = ''
  } catch (reason) {
    error.value = message(reason, '修改失败')
  }
  refresh()
  emit('change', Object.values(values.value).filter(isAnswered).length)
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
    attempted.value = false
    error.value = ''
  } catch (reason) {
    error.value = message(reason, '无法继续填写')
  }
  refresh()
  void focus()
}
function submissionAnswers(): Answer[] | undefined {
  if (!inputs || props.disabled || props.submitDisabled) return
  if (snapshot.value?.outcome !== 'complete') {
    navigate('next')
    return
  }
  try {
    return inputs.submissionAnswers()
  } catch (reason) {
    error.value = message(reason, '无法提交')
    void focus()
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
    refresh()
  }
  attempted.value = true
  await focus(id)
}
defineExpose({ submissionAnswers, focusQuestion })
</script>
<template>
  <div ref="root" tabindex="-1" class="flow-fields">
    <p v-if="error" class="notice mt" role="alert">{{ error }}</p>
    <template v-if="snapshot">
      <template v-if="!snapshot.outcome">
        <p class="hint mt" aria-live="polite">
          当前第 {{ snapshot.history.length }} 步 · 只需填写当前显示的题目
        </p>
        <SurveyFields
          v-model="values"
          :document="pageDocument"
          :errors="fieldErrors"
          :disabled="disabled"
          @change="change"
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
              : '此筛选结果仅供预览，不会保存答卷。'
          }}
        </p>
        <dl v-if="snapshot.outcome === 'complete'" class="flow-review">
          <template
            v-for="question in document.questions.filter((q) => snapshot?.visible.includes(q.id))"
            :key="question.id"
          >
            <dt>{{ question.title }}</dt>
            <dd>{{ answerLabel(question) }}</dd>
          </template>
        </dl>
      </section>
      <div class="fill-footer">
        <button
          type="button"
          class="button"
          :disabled="disabled || snapshot.history.length <= 1"
          @click="navigate('previous')"
        >
          上一步
        </button>
        <button
          v-if="!snapshot.outcome"
          type="button"
          class="button button-primary"
          :disabled="disabled"
          @click="navigate('next')"
        >
          下一步
        </button>
        <button
          v-else-if="snapshot.outcome === 'complete'"
          type="button"
          class="button button-primary"
          :disabled="disabled || submitDisabled"
          @click="confirm"
        >
          {{ preview ? '确认预览结果' : '确认提交答卷' }}
        </button>
        <button v-if="preview" type="button" class="button" :disabled="disabled" @click="reset">
          重新预览
        </button>
      </div>
    </template>
  </div>
</template>
