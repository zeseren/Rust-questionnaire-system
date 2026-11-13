<!-- [入口在哪] 列表页「继续编辑」或新建成功后进入，顶部固定栏常驻返回列表与草稿实体操作。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 顶部固定栏左侧常驻「← 返回问卷列表」；草稿未发布时同区常驻「删除草稿」幽灵按钮（悬停变红 + 二次确认），任何一步都能退或删。 -->
<!-- [主提交通道在哪] 顶部固定栏最右侧「发布问卷 / 发布新版本」是全视图唯一主按钮，左侧紧邻次级「保存修改」。 -->
<!-- 草稿修订号独立保存，回收设置的响应不能掩盖其他窗口的编辑冲突。 -->
<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  provide,
  ref,
  shallowRef,
  watch,
} from 'vue'
import { onBeforeRouteLeave, RouterLink, useRoute, useRouter } from 'vue-router'
import { api, ApiError, downloadBlob, message } from '../api'
import { encode } from '../json'
import { dateTime, localInput, questionNames, shareLink, statusNames, toTimestamp } from '../format'
import {
  duplicateQuestion,
  moveQuestion,
  newQuestion,
  removeQuestion,
  syncFlowOrder,
} from '../documents'
import {
  SURVEY_ACTION_DESCRIPTIONS,
  SURVEY_ACTION_NAMES,
  type SurveyLifecycleAction,
} from '../survey-actions'
import { useNotice } from '../stores'
import type {
  FillingMode,
  Preview,
  Question,
  SurveyDetail,
  SurveyDocument,
  Version,
} from '../types'
import Icon from '../components/Icon.vue'
import Modal from '../components/Modal.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
import QuestionEditor from '../components/QuestionEditor.vue'
import LogicEditor from '../components/LogicEditor.vue'
import DraftPreview from '../components/DraftPreview.vue'
import SurveyTemplatePicker from '../components/SurveyTemplatePicker.vue'
import { createSurveyTemplate } from '../survey-templates'
import type { SurveyTemplateId } from '../survey-templates'
import { editorDisabledKey } from '../editor-context'
import '../editor.css'
import { LogicError } from '../logic'
const route = useRoute()
const router = useRouter()
const notice = useNotice()
const id = String(route.params.id)
const survey = shallowRef<SurveyDetail | null>(null)
const draft = ref<SurveyDocument | null>(null)
const revision = ref(0)
const loading = ref(true)
/** 最近一次服务端确认的草稿序列化结果；只在防抖比对与保存/加载同步点使用。 */
const savedDraft = ref('')
/** 深层变更标记：每次编辑只置一次位，不按键做整份文档序列化。 */
const dirty = ref(false)
let compareTimer: ReturnType<typeof setTimeout> | undefined
/** 防抖比对同时负责“改了又改回去”的还原检测。 */
function scheduleCompare() {
  clearTimeout(compareTimer)
  compareTimer = setTimeout(() => {
    dirty.value = !!draft.value && encode(draft.value) !== savedDraft.value
  }, 250)
}
/** 服务端确认后同步基线，避免加载或保存响应把页面标记成脏。 */
function markSaved() {
  clearTimeout(compareTimer)
  compareTimer = undefined
  dirty.value = false
}
watch(
  draft,
  () => {
    if (loading.value || !draft.value) return
    dirty.value = true
    scheduleCompare()
  },
  { deep: true, flush: 'sync' },
)
const busy = ref(false)
const error = ref('')
const fieldError = ref<{ id: string; message: string } | null>(null)
const conflict = ref(false)
const mode = ref<FillingMode | null>(null)
const deadline = ref('')
const editorForm = ref<HTMLFormElement>()
const action = ref<SurveyLifecycleAction | null>(null)
const textPending = ref(false)
const workspace = ref<'questions' | 'logic' | 'preview' | 'settings'>('questions')
const logicEditor = ref<InstanceType<typeof LogicEditor>>()
const editorKey = ref(0)
const templateDialog = ref(false)
const selectedTemplate = ref<SurveyTemplateId | null>('feedback')
const settingsDirty = computed(
  () =>
    !!survey.value &&
    (mode.value !== survey.value.mode || deadline.value !== localInput(survey.value.deadline)),
)
const changed = computed(
  () =>
    textPending.value ||
    dirty.value ||
    (survey.value?.status !== 'archived' && settingsDirty.value),
)
const locked = computed(() => busy.value || survey.value?.status === 'archived')
provide(editorDisabledKey, locked)
async function handleError(reason: unknown) {
  error.value = message(reason)
  if (reason instanceof ApiError) {
    conflict.value = reason.code === 'revision_conflict'
    if (reason.question_id) {
      fieldError.value = { id: reason.question_id, message: reason.message }
      workspace.value = 'questions'
      await nextTick()
      document.getElementById(`question-${reason.question_id}`)?.focus()
    }
  }
  if (reason instanceof LogicError) {
    error.value = `${reason.message}（${reason.path}）`
    workspace.value = 'logic'
    await nextTick()
    await logicEditor.value?.locate()
  }
}
/** 集中回写服务端返回的问卷详情；回收设置响应不重置本地草稿。 */
function applySurveyDetail(result: SurveyDetail, resetDraft = true) {
  survey.value = result
  revision.value = result.revision
  mode.value = result.mode
  deadline.value = localInput(result.deadline)
  if (resetDraft) {
    draft.value = result.draft
    savedDraft.value = encode(result.draft)
    markSaved()
  }
}
async function load() {
  loading.value = true
  error.value = ''
  conflict.value = false
  fieldError.value = null
  try {
    applySurveyDetail(await api<SurveyDetail>(`/api/surveys/${id}`))
    editorKey.value++
    textPending.value = false
  } catch (reason) {
    await handleError(reason)
  } finally {
    loading.value = false
  }
}
async function reload() {
  if (!changed.value || window.confirm('重新加载将放弃当前未保存的修改，是否继续？')) await load()
}
/** 文本草稿未应用时由三步弹窗决定：应用并保存、忽略草稿保存、取消保存。 */
const textDraftDialog = ref(false)
async function save() {
  if (!draft.value || !survey.value || locked.value || !editorForm.value) return
  if (textPending.value) {
    textDraftDialog.value = true
    return
  }
  await commit()
}
/** 用户选择后再动草稿：取消不会丢弃任何已写内容。 */
function chooseTextDraft(choice: 'apply' | 'discard' | 'cancel') {
  textDraftDialog.value = false
  if (choice === 'cancel') return
  if (choice === 'apply' && !logicEditor.value?.applyText()) {
    workspace.value = 'logic'
    error.value = '文本规则存在错误，请修正后再保存。'
    return
  }
  if (choice === 'discard') logicEditor.value?.discardText()
  textPending.value = false
  void commit()
}
async function commit() {
  const form = editorForm.value
  if (!form) return
  if (!form.checkValidity()) {
    const invalid = Array.from(form.elements).find(
      (element): element is HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement =>
        (element instanceof HTMLInputElement ||
          element instanceof HTMLSelectElement ||
          element instanceof HTMLTextAreaElement) &&
        !element.validity.valid,
    )
    workspace.value = invalid?.closest('#question-design')
      ? 'questions'
      : invalid?.closest('#logic-editor')
        ? 'logic'
        : 'settings'
    await nextTick()
    form.reportValidity()
    return
  }
  busy.value = true
  error.value = ''
  conflict.value = false
  fieldError.value = null
  // 回收设置的入参在草稿保存前固定，避免步骤间互相覆盖。
  const collection = settingsDirty.value ? { mode: mode.value, deadline: deadline.value } : null
  let draftRevision = revision.value
  try {
    // 草稿 PUT 必须先用旧修订号检测其他窗口的编辑冲突；回收设置的响应不参与这一步。
    if (dirty.value) {
      // 请求体在发出前定格：它同时是服务端已接受的基线，避免把等待期间的新输入误标为已保存。
      const payload = encode(draft.value)
      const result = await api<SurveyDetail>(`/api/surveys/${id}/draft`, 'PUT', {
        revision: revision.value,
        document: draft.value,
      })
      // 只确认草稿与修订号；模式与截止时间仍以用户当前输入为准，等 PATCH 成功再同步。
      survey.value = result
      revision.value = result.revision
      draftRevision = result.revision
      savedDraft.value = payload
      markSaved()
      // 保存期间继续输入的内容仍未落库，恢复脏标记并安排一次比对。
      if (encode(draft.value) !== payload) {
        dirty.value = true
        scheduleCompare()
      }
    }
    if (collection) {
      try {
        applySurveyDetail(
          await api<SurveyDetail>(`/api/surveys/${id}/collection`, 'PATCH', {
            mode: collection.mode,
            deadline: toTimestamp(collection.deadline),
          }),
          false,
        )
      } catch (reason) {
        // 草稿已经落库：明确报告两步结果，保留回收设置输入供再次保存。
        conflict.value = reason instanceof ApiError && reason.code === 'revision_conflict'
        error.value = `草稿已保存（修订 ${draftRevision}），但回收设置保存失败：${message(reason)}`
        return
      }
    }
    notice.show(collection ? '草稿与回收设置已保存。' : '草稿已保存。')
  } catch (reason) {
    await handleError(reason)
  } finally {
    busy.value = false
  }
}
async function preparePublish() {
  busy.value = true
  error.value = ''
  fieldError.value = null
  try {
    const preview = await api<Preview>(`/api/surveys/${id}/preview`)
    if (preview.revision !== revision.value)
      throw new ApiError(
        409,
        'revision_conflict',
        '草稿已被其他窗口修改，请保留当前内容并重新加载。',
      )
    if (!preview.publishable)
      throw new ApiError(
        422,
        'validation_failed',
        preview.validation_error?.message ?? '问卷尚未满足发布条件。',
        preview.validation_error?.question_id,
      )
    action.value = 'publish'
  } catch (reason) {
    await handleError(reason)
  } finally {
    busy.value = false
  }
}
async function performAction() {
  if (!action.value || busy.value) return
  const requested = action.value
  busy.value = true
  error.value = ''
  try {
    if (requested === 'delete') {
      await api(`/api/surveys/${id}`, 'DELETE')
      draft.value = null
      survey.value = null
      action.value = null
      notice.show('草稿已删除。')
      await router.push('/surveys')
    } else if (requested === 'publish') {
      const result = await api<{ survey: SurveyDetail; version: Version }>(
        `/api/surveys/${id}/publish`,
        'POST',
        { revision: revision.value },
      )
      applySurveyDetail(result.survey)
      action.value = null
      notice.show(
        `版本 V${result.version.number} 已发布${result.survey.status === 'closed' ? '，问卷仍保持关闭。' : '，现在可以分享问卷。'}`,
      )
    } else {
      applySurveyDetail(await api<SurveyDetail>(`/api/surveys/${id}/${requested}`, 'POST'))
      action.value = null
      notice.show(`${SURVEY_ACTION_NAMES[requested]}成功。`)
    }
  } catch (reason) {
    await handleError(reason)
    action.value = null
  } finally {
    busy.value = false
  }
}
async function duplicate() {
  busy.value = true
  error.value = ''
  try {
    const result = await api<SurveyDetail>(`/api/surveys/${id}/copy`, 'POST')
    await router.push(`/surveys/${result.id}/edit`)
    notice.show('已复制为新的问卷草稿。')
  } catch (reason) {
    await handleError(reason)
  } finally {
    busy.value = false
  }
}
async function add(type: Question['type']) {
  if (!draft.value || draft.value.questions.length >= 100) return
  const question = newQuestion(type)
  workspace.value = 'questions'
  draft.value.questions.push(question)
  syncFlowOrder(draft.value)
  await nextTick()
  document.getElementById(`question-${question.id}`)?.focus()
}
function remove(questionId: string) {
  if (locked.value || !draft.value) return
  const question = draft.value.questions.find((item) => item.id === questionId)
  const referenced = !!draft.value.logic?.rules.some((rule) => rule.question_id === questionId)
  if (
    (question?.title.trim() || referenced) &&
    !window.confirm('确定删除这道题目吗？相关规则引用也会一并清理。')
  )
    return
  removeQuestion(draft.value, questionId)
}
async function locateQuestion(questionId: string) {
  workspace.value = 'questions'
  await nextTick()
  document.getElementById(`question-${questionId}`)?.focus()
}
async function editRule(questionId: string) {
  if (locked.value) return
  workspace.value = 'logic'
  await nextTick()
  await logicEditor.value?.addRule(questionId)
}
function applyTemplate() {
  if (locked.value || !selectedTemplate.value || textPending.value) return
  const replacement = createSurveyTemplate(selectedTemplate.value)
  if (draft.value?.title) replacement.title = draft.value.title
  draft.value = replacement
  editorKey.value++
  fieldError.value = null
  error.value = ''
  templateDialog.value = false
  workspace.value = 'questions'
  notice.show('模板已应用，可以修改题目并试填规则。保存后生效。')
}
function move(index: number, direction: number) {
  if (!draft.value) return
  moveQuestion(draft.value, index, direction)
}
function selectInputText(event: FocusEvent) {
  if (event.target instanceof HTMLInputElement) event.target.select()
}
function exportDraft() {
  downloadBlob(
    new Blob([encode(draft.value)], { type: 'application/json;charset=utf-8' }),
    `draft-${id}.json`,
  )
}
async function copyLink() {
  try {
    await navigator.clipboard.writeText(shareLink(id))
    notice.show('填写链接已复制。')
  } catch {
    notice.show('请选中链接后手动复制。')
  }
}
function leave(event: BeforeUnloadEvent) {
  if (changed.value) {
    event.preventDefault()
    event.returnValue = ''
  }
}
onBeforeRouteLeave(() => !changed.value || window.confirm('还有未保存的修改，确定离开此页面吗？'))
onMounted(() => {
  load()
  window.addEventListener('beforeunload', leave)
})
onBeforeUnmount(() => {
  clearTimeout(compareTimer)
  window.removeEventListener('beforeunload', leave)
})
</script>
<template>
  <LoadingState v-if="loading" />
  <div v-else class="survey-editor">
    <div class="page-heading editor-heading">
      <div>
        <RouterLink class="text-button" to="/surveys">← 返回问卷列表</RouterLink>
        <p class="eyebrow">问卷编辑</p>
        <h1>{{ draft?.title || '无标题问卷' }}</h1>
        <p v-if="survey" class="heading-note">
          {{ statusNames[survey.status] }} · 修订 {{ revision }} ·
          {{ changed ? '有未保存的修改' : '已保存' }}
        </p>
      </div>
      <div v-if="survey" class="actions editor-entity-actions">
        <button
          v-if="!survey.current_version_id"
          type="button"
          class="text-button danger-text"
          :disabled="busy"
          @click="action = 'delete'"
        >
          删除草稿
        </button>
        <button
          v-if="survey.current_version_id && survey.status !== 'archived'"
          type="button"
          class="text-button danger-text"
          :disabled="busy || changed"
          @click="action = 'archive'"
        >
          归档问卷
        </button>
        <button type="button" class="text-button" @click="exportDraft">导出草稿</button>
        <RouterLink
          class="text-button"
          :to="`/surveys/${id}/preview`"
          target="_blank"
          rel="noopener"
        >
          预览已保存草稿
        </RouterLink>
        <button
          v-if="survey.current_version_id && survey.status !== 'unpublished'"
          type="button"
          class="text-button"
          @click="copyLink"
        >
          <Icon name="copy" />
          复制链接
        </button>
        <RouterLink
          v-if="survey.current_version_id"
          class="text-button"
          :to="`/surveys/${id}/reports`"
        >
          <Icon name="chart" />
          答卷与统计
        </RouterLink>
        <button
          v-if="survey.status === 'collecting'"
          type="button"
          class="text-button"
          :disabled="busy || changed"
          @click="action = 'close'"
        >
          关闭回收
        </button>
        <button
          v-if="survey.status === 'closed' && !survey.taken_down"
          type="button"
          class="text-button"
          :disabled="busy || changed"
          @click="action = 'resume'"
        >
          恢复回收
        </button>
        <button type="button" class="text-button" :disabled="busy || changed" @click="duplicate">
          <Icon name="copy" />
          复制问卷
        </button>
        <button class="button" :disabled="locked || !changed" @click="save">
          {{ busy ? '正在处理…' : '保存修改' }}
        </button>
        <button class="button button-primary" :disabled="locked || changed" @click="preparePublish">
          {{ survey.current_version_id ? '发布新版本' : '发布问卷' }}
        </button>
      </div>
    </div>
    <ErrorNotice :message="error" :retry="!survey" @retry="load" />
    <div v-if="conflict" class="notice">
      <div class="grow">当前编辑内容已保留。可以先下载草稿，再读取服务器上的最新内容。</div>
      <div class="actions">
        <button class="button" @click="exportDraft">下载当前草稿</button>
        <button class="button" @click="reload">重新加载</button>
      </div>
    </div>
    <div v-if="survey?.taken_down" class="notice">问卷已被管理员下架，暂不接受新提交。</div>
    <div v-if="survey?.status === 'archived'" class="notice">
      这份问卷已归档，可以查看答卷或复制为新问卷。
    </div>
    <nav v-if="survey && draft" class="workspace-nav" aria-label="问卷编辑工作区">
      <div class="workspace-tabs" role="group" aria-label="编辑步骤">
        <button
          type="button"
          :aria-pressed="workspace === 'questions'"
          @click="workspace = 'questions'"
        >
          <Icon name="document" />
          <span>题目设计</span>
          <span class="workspace-count">{{ draft.questions.length }}</span>
        </button>
        <button type="button" :aria-pressed="workspace === 'logic'" @click="workspace = 'logic'">
          <Icon name="grid" />
          <span>规则与跳题</span>
          <span v-if="draft.logic" class="workspace-indicator"></span>
        </button>
        <button
          type="button"
          :aria-pressed="workspace === 'preview'"
          @click="workspace = 'preview'"
        >
          <Icon name="play" />
          <span>试填预览</span>
        </button>
        <button
          type="button"
          :aria-pressed="workspace === 'settings'"
          @click="workspace = 'settings'"
        >
          <Icon name="settings" />
          <span>回收设置</span>
        </button>
      </div>
      <button
        type="button"
        class="text-button"
        :disabled="locked || textPending"
        @click="templateDialog = true"
      >
        <Icon name="spark" />
        从模板开始
      </button>
    </nav>
    <div v-if="textPending" class="notice">
      <div class="grow">有尚未应用的文本规则；保存时会询问应用或忽略这段草稿。</div>
      <button class="button" @click="workspace = 'logic'">回到文本规则</button>
    </div>
    <div
      v-if="survey && draft"
      class="editor-layout"
      :class="{ 'is-wide': workspace !== 'questions', 'is-settings': workspace === 'settings' }"
    >
      <form
        v-show="workspace === 'questions' || workspace === 'logic'"
        id="editor-form"
        ref="editorForm"
        class="editor-main"
        novalidate
        @submit.prevent="save"
      >
        <fieldset :disabled="locked">
          <div v-show="workspace === 'questions'" id="question-design">
            <section class="panel form-stack">
              <div class="actions between">
                <h2>问卷信息</h2>
                <span class="hint">先把想了解的事说清楚</span>
              </div>
              <label class="field">
                问卷标题
                <input
                  v-model="draft.title"
                  class="document-title-input"
                  placeholder="为问卷起一个清晰的标题"
                />
              </label>
              <label class="field">
                调查说明
                <textarea v-model="draft.description" placeholder="介绍背景、填写方式或预计用时" />
              </label>
              <label class="field">
                数据收集目的
                <span class="hint">发布前必填，填写者将在作答前看到这段说明。</span>
                <textarea v-model="draft.purpose" placeholder="为什么收集这些信息？将如何使用？" />
              </label>
              <label class="field">
                提交结束语
                <input v-model="draft.thank_you" placeholder="感谢你的认真回答。" />
              </label>
            </section>
            <QuestionEditor
              v-for="(question, index) in draft.questions"
              :key="question.id"
              :model-value="question"
              :index="index"
              :total="draft.questions.length"
              :disabled="locked"
              :has-rule="!!draft.logic?.rules.some((rule) => rule.question_id === question.id)"
              :error="fieldError?.id === question.id ? fieldError.message : undefined"
              @update:model-value="draft.questions[index] = $event"
              @move="move(index, $event)"
              @duplicate="draft.questions.splice(index + 1, 0, duplicateQuestion(question, draft))"
              @remove="remove(question.id)"
              @rule="editRule(question.id)"
            />
            <div v-if="!draft.questions.length" class="empty-state mt">
              <Icon name="document" />
              <h3>添加你的第一个问题</h3>
              <p>单选、多选、文本、数字与评分，选择适合的提问方式。</p>
              <button
                type="button"
                class="button"
                :disabled="locked"
                @click="templateDialog = true"
              >
                使用含规则的问卷模板
              </button>
            </div>
            <button
              type="button"
              class="add-question"
              :disabled="locked || draft.questions.length >= 100"
              @click="add('single_choice')"
            >
              ＋ 添加题目
            </button>
          </div>
          <div v-show="workspace === 'logic'">
            <LogicEditor
              :key="editorKey"
              ref="logicEditor"
              v-model="draft.logic"
              :questions="draft.questions"
              :disabled="locked"
              @pending="textPending = $event"
            />
          </div>
        </fieldset>
      </form>
      <DraftPreview v-if="workspace === 'preview'" :document="draft" :text-pending="textPending" />
      <aside v-show="workspace === 'questions' || workspace === 'settings'" class="editor-aside">
        <section v-show="workspace === 'questions'" class="panel">
          <div class="panel-header">
            <h2>添加题目</h2>
            <span class="hint">{{ draft.questions.length }} / 100</span>
          </div>
          <div class="question-palette">
            <button
              v-for="(label, type) in questionNames"
              :key="type"
              class="palette-button"
              :disabled="locked || draft.questions.length >= 100"
              @click="add(type)"
            >
              <Icon
                :name="
                  type === 'rating'
                    ? 'spark'
                    : type === 'number'
                      ? 'chart'
                      : type === 'multiple_choice'
                        ? 'grid'
                        : 'document'
                "
              />
              {{ label }}
            </button>
          </div>
          <div v-if="draft.questions.length" class="question-outline">
            <h3>题目大纲</h3>
            <nav aria-label="定位题目">
              <button
                v-for="(question, index) in draft.questions"
                :key="question.id"
                type="button"
                @click="locateQuestion(question.id)"
              >
                <span>{{ String(index + 1).padStart(2, '0') }}</span>
                <span class="truncate">{{ question.title || '未命名题目' }}</span>
                <span
                  v-if="draft.logic?.rules.some((rule) => rule.question_id === question.id)"
                  class="outline-rule"
                  title="已配置规则"
                >
                  ·
                </span>
              </button>
            </nav>
          </div>
        </section>
        <section v-show="workspace === 'settings'" class="panel collection-panel">
          <div class="panel-header"><h2>回收设置</h2></div>
          <div class="form-stack">
            <label class="field">
              填写模式
              <select
                v-model="mode"
                form="editor-form"
                :disabled="locked || !!survey.current_version_id"
              >
                <option :value="null" disabled>未设置（请选择填写模式）</option>
                <option value="anonymous">公开匿名</option>
                <option value="login_once">每个账号限填一次</option>
              </select>
            </label>
            <p class="hint">
              {{
                mode === null
                  ? '请选择填写模式（发布前必填）。'
                  : mode === 'anonymous'
                    ? '不关联账号，不承诺按自然人限次。'
                    : '需登录，每个账号跨版本仅限一次；不属于完全匿名。'
              }}首次发布后不能更改。
            </p>
            <label class="field">
              截止时间（北京时间）
              <input
                v-model="deadline"
                form="editor-form"
                type="datetime-local"
                :disabled="locked"
              />
            </label>
            <p class="hint">留空表示不设截止时间。延长截止时间后，已关闭问卷需手动恢复回收。</p>
            <p v-if="changed" class="accent hint">保存修改后即可发布。</p>
          </div>
          <p class="hint">
            状态管理、导出与删除草稿常驻在本页顶部固定栏，无需回到本步骤；这里只调整回收参数。
          </p>
          <div
            v-if="survey.current_version_id && survey.status !== 'unpublished'"
            class="share-box"
          >
            <label class="field">
              填写链接
              <input :value="shareLink(id)" readonly @focus="selectInputText" />
            </label>
            <div class="actions">
              <button class="text-button accent" @click="copyLink">复制链接</button>
              <a class="text-button" :href="shareLink(id)" target="_blank" rel="noopener">
                打开填写页
                <Icon name="up-right" />
              </a>
            </div>
          </div>
          <p class="hint mt">最近保存：{{ dateTime(survey.updated_at) }}</p>
        </section>
      </aside>
    </div>
  </div>
  <Modal v-if="textDraftDialog" title="文本规则尚未应用" @close="chooseTextDraft('cancel')">
    <p class="subtle mb">
      文本 DSL 中还有未写回可视化规则的修改。请选择这次保存要采用哪一种草稿；取消不会丢弃任何内容。
    </p>
    <ul class="hint mb">
      <li>应用并保存：先把文本规则写回可视化规则，再保存草稿。</li>
      <li>忽略草稿保存：放弃这段文本修改，按当前可视化规则保存。</li>
      <li>取消保存：回到编辑器，可继续修改文本或先试填确认。</li>
    </ul>
    <div class="actions between">
      <button class="button" @click="chooseTextDraft('cancel')">取消保存</button>
      <span class="grow" aria-hidden="true"></span>
      <button class="text-button danger-text" @click="chooseTextDraft('discard')">
        忽略草稿保存
      </button>
      <button class="button button-primary" @click="chooseTextDraft('apply')">应用并保存</button>
    </div>
  </Modal>
  <Modal v-if="templateDialog" title="选择一个问卷起点" wide @close="templateDialog = false">
    <p class="subtle mb">模板包含可直接试填的题目和规则，应用后可以自由修改。</p>
    <SurveyTemplatePicker v-model="selectedTemplate" :allow-blank="false" :disabled="locked" />
    <p class="hint mt">
      {{
        draft?.questions.length
          ? '应用将替换当前题目、规则与问卷说明，保留问卷标题。保存后生效。'
          : '应用后可在“规则与跳题”查看配置，在“试填预览”体验效果。'
      }}
    </p>
    <div class="actions between mt">
      <button class="button" @click="templateDialog = false">取消</button>
      <button
        class="button"
        :class="draft?.questions.length ? 'button-danger' : 'button-primary'"
        :disabled="locked || !selectedTemplate || textPending"
        @click="applyTemplate"
      >
        {{ draft?.questions.length ? '替换并应用模板' : '应用模板' }}
      </button>
    </div>
  </Modal>
  <Modal
    v-if="action"
    :title="SURVEY_ACTION_NAMES[action]"
    :dismissible="!busy"
    @close="action = null"
  >
    <p class="subtle mb">{{ SURVEY_ACTION_DESCRIPTIONS[action] }}</p>
    <p v-if="action === 'publish'" class="notice">
      将发布修订 {{ revision }}，共 {{ draft?.questions.length }} 道题。
    </p>
    <div class="actions between">
      <button class="button" :disabled="busy" @click="action = null">取消</button>
      <button
        class="button"
        :class="
          action === 'delete' || action === 'archive' || action === 'close'
            ? 'button-danger'
            : 'button-primary'
        "
        :disabled="busy"
        @click="performAction"
      >
        {{ busy ? '正在处理…' : `确认${SURVEY_ACTION_NAMES[action]}` }}
      </button>
    </div>
  </Modal>
</template>
