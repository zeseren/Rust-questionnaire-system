<!-- 翻页范围取自服务器总数，空列表仍保留第一页语义。 -->
<script setup lang="ts">
import { computed } from 'vue'
const props = withDefaults(
  defineProps<{ page: number; total: number; pageSize?: number; disabled?: boolean }>(),
  { pageSize: 20 },
)
defineEmits<{ change: [page: number] }>()
const pages = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)))
</script>
<template>
  <nav class="pagination" aria-label="分页">
    <span>共 {{ total }} 项</span>
    <div class="actions">
      <button class="button" :disabled="disabled || page <= 1" @click="$emit('change', page - 1)">
        上一页
      </button>
      <span>{{ page }} / {{ pages }}</span>
      <button
        class="button"
        :disabled="disabled || page >= pages"
        @click="$emit('change', page + 1)"
      >
        下一页
      </button>
    </div>
  </nav>
</template>
