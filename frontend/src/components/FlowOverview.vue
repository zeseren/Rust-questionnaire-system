<!-- 流程缩略图展示真实出口，点击节点回到对应配置，不把排列顺序误当执行路径。 -->
<script setup lang="ts">
import type { FlowDefinition } from '../flow-compiler'
import type { Question } from '../types'
const props = defineProps<{ flow: FlowDefinition; questions: Question[] }>()
defineEmits<{ select: [id: string] }>()
function target(id: string) {
  const index = props.flow.nodes.findIndex((node) => node.id === id)
  if (index < 0) return '未设置出口'
  const node = props.flow.nodes[index]!
  return `${index + 1}. ${node.type === 'page' ? '页面' : node.type === 'branch' ? '分支' : '结束'}`
}
</script>
<template>
  <nav class="flow-overview" aria-label="流程概览">
    <button
      v-for="(node, index) in flow.nodes"
      :key="node.id"
      type="button"
      class="flow-mini-node"
      :class="node.type"
      @click="$emit('select', node.id)"
    >
      <span class="actions between">
        <strong>
          {{ index + 1 }} ·
          {{ node.type === 'page' ? '页面' : node.type === 'branch' ? '条件分支' : '结束' }}
        </strong>
        <span v-if="flow.entry === node.id" class="flow-entry">入口</span>
      </span>
      <template v-if="node.type === 'page'">
        <span class="flow-mini-description">
          {{
            node.question_ids
              .map((id) => questions.find((question) => question.id === id)?.title || '未命名题目')
              .join(' / ') || '尚未分配题目'
          }}
        </span>
        <span class="flow-edge">下一步 → {{ target(node.next) }}</span>
      </template>
      <template v-else-if="node.type === 'branch'">
        <span v-for="(branch, i) in node.branches" :key="i" class="flow-edge">
          条件 {{ i + 1 }} → {{ target(branch.target) }}
        </span>
        <span class="flow-edge">否则 → {{ target(node.otherwise) }}</span>
      </template>
      <span v-else class="flow-mini-description">
        {{ node.outcome === 'complete' ? '完成填写' : '筛选结束' }}
      </span>
    </button>
  </nav>
</template>
