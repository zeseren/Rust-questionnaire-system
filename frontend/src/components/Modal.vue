<!-- 原生对话框管理焦点，关闭时交还给触发控件。 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import Icon from './Icon.vue'
const props = withDefaults(
  defineProps<{ title: string; dismissible?: boolean; wide?: boolean }>(),
  { dismissible: true },
)
const emit = defineEmits<{ close: [] }>()
const dialog = ref<HTMLDialogElement>()
function close() {
  if (props.dismissible) emit('close')
}
function outside(event: MouseEvent) {
  const element = dialog.value
  if (!element || event.target !== element) return
  const rect = element.getBoundingClientRect()
  if (
    event.clientX < rect.left ||
    event.clientX > rect.right ||
    event.clientY < rect.top ||
    event.clientY > rect.bottom
  )
    close()
}
onMounted(() => dialog.value?.showModal())
onBeforeUnmount(() => dialog.value?.close())
</script>
<template>
  <dialog
    ref="dialog"
    class="design-dialog"
    :class="{ 'dialog-wide': wide }"
    :aria-label="title"
    @cancel.prevent="close"
    @click="outside"
  >
    <div class="dialog-header">
      <h2>{{ title }}</h2>
      <button
        v-if="dismissible"
        type="button"
        class="icon-button"
        aria-label="关闭对话框"
        @click="close"
      >
        <Icon name="close" />
      </button>
    </div>
    <div class="dialog-body"><slot /></div>
  </dialog>
</template>
