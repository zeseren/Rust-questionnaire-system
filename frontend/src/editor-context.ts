//! 自定义控件共享编辑锁，归档和请求期间不能绕过原生字段集的禁用状态。
import type { InjectionKey, Ref } from 'vue'
export const editorDisabledKey: InjectionKey<Readonly<Ref<boolean>>> = Symbol('editor-disabled')
