<!-- 页面顺序和出口由作者显式配置，结构变化后保留失效引用供定位修复。 -->
<script setup lang="ts">
import { nextTick } from 'vue'
import { insertFlowNode } from '../flow-compiler'
import type { FlowDefinition, FlowNode } from '../flow-compiler'
import type { ComputedVariable } from '../document-logic'
import type { Question } from '../types'
import ConditionBuilder from './ConditionBuilder.vue'
import FlowTarget from './FlowTarget.vue'
import FlowOverview from './FlowOverview.vue'
import SearchSelect from './SearchSelect.vue'
import Icon from './Icon.vue'
const flow = defineModel<FlowDefinition | null | undefined>({ required: true })
const props = defineProps<{ questions: Question[]; variables: ComputedVariable[] }>()
function start(perQuestion = false) {
  if (!props.questions.length) return
  const end = crypto.randomUUID()
  const groups = perQuestion
    ? props.questions.map((question) => [question.id])
    : [props.questions.map((question) => question.id)]
  const pages = groups.map((question_ids) => ({
    type: 'page' as const,
    id: crypto.randomUUID(),
    question_ids,
    next: end,
  }))
  pages.forEach((page, index) => {
    page.next = pages[index + 1]?.id ?? end
  })
  flow.value = {
    entry: pages[0]!.id,
    nodes: [...pages, { type: 'end', id: end, outcome: 'complete' }],
  }
}
async function locate(id: string) {
  await nextTick()
  document.getElementById(`logic-node-${id}`)?.focus()
}
function addNode(index: number, type: FlowNode['type']) {
  if (!flow.value) return
  const id = insertFlowNode(flow.value, index, type)
  void locate(id)
}
function move(index: number, direction: number) {
  if (!flow.value) return
  const to = index + direction
  if (to < 0 || to >= flow.value.nodes.length) return
  flow.value.nodes.splice(to, 0, flow.value.nodes.splice(index, 1)[0]!)
}
function assign(questionId: string, pageId: string) {
  for (const node of flow.value?.nodes ?? [])
    if (node.type === 'page') {
      node.question_ids = node.question_ids.filter((id) => id !== questionId)
      if (node.id === pageId) node.question_ids.push(questionId)
      node.question_ids.sort(
        (a, b) =>
          props.questions.findIndex((question) => question.id === a) -
          props.questions.findIndex((question) => question.id === b),
      )
    }
}
function pageOf(id: string) {
  return (
    flow.value?.nodes.find((node) => node.type === 'page' && node.question_ids.includes(id))?.id ??
    ''
  )
}
function removeFlow() {
  if (
    window.confirm('移除分页与分支后，所有可见题目将在同一页填写。题目规则和变量会保留，是否继续？')
  )
    flow.value = null
}
</script>
<template>
  <div class="form-stack">
    <div class="actions between">
      <div>
        <h3>页面流程</h3>
        <p class="hint">组织填写步骤，让不同的回答走向合适的问题。</p>
      </div>
      <button v-if="flow" type="button" class="text-button danger-text" @click="removeFlow">
        移除页面流程
      </button>
    </div>
    <div v-if="!flow" class="logic-empty">
      <span class="template-icon teal"><Icon name="grid" /></span>
      <h3>把问卷分成轻松的几个步骤</h3>
      <p class="hint">目前全部可见题目在同一页。可以先分页，再添加条件分支。</p>
      <div class="actions">
        <button
          type="button"
          class="button button-primary"
          :disabled="!questions.length"
          @click="start(true)"
        >
          每题一页
        </button>
        <button type="button" class="button" :disabled="!questions.length" @click="start()">
          从一个页面开始
        </button>
      </div>
    </div>
    <template v-else>
      <FlowOverview :flow="flow" :questions="questions" @select="locate" />
      <p class="hint">点击概览定位节点。出口只能前往后续节点；多个出口可指向同一页面实现汇合。</p>
      <label class="field">
        入口页面
        <SearchSelect
          v-model="flow.entry"
          label="入口页面"
          :options="
            flow.nodes.flatMap((node, index) =>
              node.type === 'page' ? [{ value: node.id, label: `${index + 1}. 页面` }] : [],
            )
          "
        />
      </label>
      <section id="logic-membership" tabindex="-1" class="logic-block form-stack">
        <div>
          <h3>题目归属</h3>
          <p class="hint">每道题属于一个页面，页面中的题目次序与题目设计保持一致。</p>
        </div>
        <label v-for="(question, index) in questions" :key="question.id" class="membership-row">
          <span>
            <span class="accent">{{ String(index + 1).padStart(2, '0') }}</span>
            {{ question.title || '未命名题目' }}
          </span>
          <SearchSelect
            :model-value="pageOf(question.id)"
            :label="`第 ${index + 1} 题归属页面`"
            :options="[
              { value: '', label: '未分配' },
              ...flow.nodes.flatMap((node, i) =>
                node.type === 'page' ? [{ value: node.id, label: `${i + 1}. 页面` }] : [],
              ),
            ]"
            @update:model-value="assign(question.id, $event)"
          />
        </label>
      </section>
      <section
        v-for="(node, index) in flow.nodes"
        :id="`logic-node-${node.id}`"
        :key="node.id"
        tabindex="-1"
        class="logic-block form-stack flow-node-detail"
        :class="`node-${node.type}`"
      >
        <div class="actions between">
          <h3>
            <span class="node-index">{{ index + 1 }}</span>
            {{ node.type === 'page' ? '页面' : node.type === 'branch' ? '条件分支' : '结束' }}
          </h3>
          <div class="actions">
            <button
              type="button"
              class="icon-button"
              :disabled="index === 0"
              :aria-label="`上移节点 ${index + 1}`"
              @click="move(index, -1)"
            >
              <Icon name="up" />
            </button>
            <button
              type="button"
              class="icon-button"
              :disabled="index === flow.nodes.length - 1"
              :aria-label="`下移节点 ${index + 1}`"
              @click="move(index, 1)"
            >
              <Icon name="down" />
            </button>
            <button
              type="button"
              class="icon-button danger-text"
              :aria-label="`删除节点 ${index + 1}`"
              @click="flow.nodes.splice(index, 1)"
            >
              <Icon name="trash" />
            </button>
          </div>
        </div>
        <template v-if="node.type === 'page'">
          <div class="page-question-chips">
            <span
              v-for="questionId in node.question_ids"
              :key="questionId"
              class="page-question-chip"
            >
              {{ questions.find((question) => question.id === questionId)?.title || '题目已删除' }}
            </span>
            <span v-if="!node.question_ids.length" class="hint">
              空页面，请在题目归属中分配题目。
            </span>
          </div>
          <button
            v-for="id in node.question_ids.filter(
              (id) => !questions.some((question) => question.id === id),
            )"
            :key="id"
            type="button"
            class="text-button danger-text"
            @click="node.question_ids = node.question_ids.filter((questionId) => questionId !== id)"
          >
            移除失效题目引用
          </button>
          <label class="field">
            填写后前往
            <FlowTarget v-model="node.next" :nodes="flow.nodes" :index="index" />
          </label>
        </template>
        <template v-else-if="node.type === 'branch'">
          <p class="hint">从上到下检查条件，第一个成立的出口生效。</p>
          <section
            v-for="(branch, branchIndex) in node.branches"
            :key="branchIndex"
            class="branch-condition form-stack"
          >
            <div class="actions between">
              <strong>条件 {{ branchIndex + 1 }}</strong>
              <div class="actions">
                <button
                  type="button"
                  class="text-button"
                  :disabled="branchIndex === 0"
                  @click="
                    node.branches.splice(
                      branchIndex - 1,
                      0,
                      node.branches.splice(branchIndex, 1)[0]!,
                    )
                  "
                >
                  提高优先级
                </button>
                <button
                  type="button"
                  class="text-button danger-text"
                  :disabled="node.branches.length <= 1"
                  @click="node.branches.splice(branchIndex, 1)"
                >
                  删除条件
                </button>
              </div>
            </div>
            <ConditionBuilder
              v-model="branch.when"
              :questions="
                questions.filter((question) =>
                  flow!.nodes
                    .slice(0, index)
                    .some(
                      (previous) =>
                        previous.type === 'page' && previous.question_ids.includes(question.id),
                    ),
                )
              "
              :variables="variables"
            />
            <label class="field">
              成立时前往
              <FlowTarget v-model="branch.target" :nodes="flow.nodes" :index="index" />
            </label>
          </section>
          <button
            type="button"
            class="text-button accent"
            :disabled="node.branches.length >= 16"
            @click="
              node.branches.push({ when: { op: 'boolean', value: true }, target: node.otherwise })
            "
          >
            <Icon name="plus" />
            添加分支条件
          </button>
          <label class="field">
            以上条件均不成立时
            <FlowTarget v-model="node.otherwise" :nodes="flow.nodes" :index="index" />
          </label>
        </template>
        <template v-else>
          <label class="field">
            结束方式
            <select v-model="node.outcome">
              <option value="complete">完成填写，允许确认提交</option>
              <option value="screen_out">筛选结束（仅可预览）</option>
            </select>
          </label>
          <p v-if="node.outcome === 'screen_out'" class="hint">包含筛选结束的问卷暂不可发布。</p>
        </template>
        <div class="node-insert actions">
          <span class="hint">在后面插入</span>
          <button
            v-for="(label, type) in { page: '页面', branch: '分支', end: '结束' }"
            :key="type"
            type="button"
            class="text-button"
            :disabled="flow.nodes.length >= 256"
            @click="addNode(index, type)"
          >
            <Icon name="plus" />
            {{ label }}
          </button>
        </div>
      </section>
      <button v-if="!flow.nodes.length" type="button" class="button" @click="start()">
        重新建立流程
      </button>
    </template>
  </div>
</template>
