//! 路由守卫只控制入口展示，资源权限仍由后端验证。
import { createRouter, createWebHistory } from 'vue-router'
import { useAuth, useNotice } from './stores.ts'

export function safeReturn(value: unknown): string {
  return typeof value === 'string' &&
    /^\/(?!\/)[^\\]*$/.test(value) &&
    !/^\/(login|register)(\?|$)/.test(value)
    ? value
    : '/'
}
export const router = createRouter({
  history: createWebHistory(),
  scrollBehavior: () => ({ top: 0 }),
  routes: [
    {
      path: '/',
      name: 'home',
      component: () => import('./views/DashboardView.vue'),
      meta: { auth: true, title: '工作台' },
    },
    {
      path: '/surveys',
      name: 'surveys',
      component: () => import('./views/DashboardView.vue'),
      meta: { auth: true, title: '我的问卷' },
    },
    {
      path: '/archive',
      name: 'archive',
      component: () => import('./views/DashboardView.vue'),
      meta: { auth: true, title: '已归档' },
    },
    {
      path: '/surveys/:id/edit',
      name: 'editor',
      component: () => import('./views/EditorView.vue'),
      meta: { auth: true, title: '编辑问卷' },
    },
    {
      path: '/surveys/:id/reports',
      name: 'reports',
      component: () => import('./views/ReportsView.vue'),
      meta: { auth: true, title: '答卷与统计' },
    },
    {
      path: '/surveys/:id/preview',
      name: 'preview',
      component: () => import('./views/PreviewView.vue'),
      meta: { auth: true, standalone: true, title: '预览问卷' },
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('./views/SettingsView.vue'),
      meta: { auth: true, title: '账户设置' },
    },
    {
      path: '/admin',
      name: 'admin',
      component: () => import('./views/AdminView.vue'),
      meta: { auth: true, admin: true, title: '平台管理' },
    },
    {
      path: '/login',
      name: 'login',
      component: () => import('./views/AuthView.vue'),
      meta: { standalone: true, title: '登录' },
    },
    {
      path: '/register',
      name: 'register',
      component: () => import('./views/AuthView.vue'),
      meta: { standalone: true, title: '注册' },
    },
    {
      path: '/s/:id',
      name: 'fill',
      component: () => import('./views/FillView.vue'),
      meta: { standalone: true, title: '填写问卷' },
    },
    {
      path: '/:pathMatch(.*)*',
      name: 'not-found',
      component: () => import('./views/NotFoundView.vue'),
      meta: { standalone: true, title: '页面不存在' },
    },
  ],
})
router.beforeEach(async (to) => {
  const auth = useAuth()
  await auth.restore()
  if (to.meta.auth && !auth.user) return { name: 'login', query: { redirect: to.fullPath } }
  if (to.meta.admin && auth.user?.role !== 'admin') {
    useNotice().show('没有访问平台管理的权限。')
    return '/'
  }
})
router.afterEach((to) => {
  document.title = `${String(to.meta.title || '问序')} · 问序`
})
