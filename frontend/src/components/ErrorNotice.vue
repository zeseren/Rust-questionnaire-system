<!-- 错误原因可被阅读器感知，是否重试由调用页面决定。 -->
<script setup lang="ts">
import Icon from './Icon.vue'
withDefaults(defineProps<{ message: string; retry?: boolean }>(), { retry: false })
defineEmits<{ retry: [] }>()
</script>
<template>
  <div v-if="message" class="notice notice-error" role="alert">
    <Icon name="info" />
    <div class="grow">{{ message }}</div>
    <button v-if="retry" type="button" class="text-button" @click="$emit('retry')">重试</button>
  </div>
</template>
