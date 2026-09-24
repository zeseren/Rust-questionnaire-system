//! 应用只挂载一个状态容器和路由实例。
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import './styles.css'
import './application.css'

createApp(App).use(createPinia()).use(router).mount('#app')
