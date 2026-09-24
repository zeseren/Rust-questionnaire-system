//! 开发代理保留浏览器 Origin，来源校验仍由后端执行。
import { defineConfig, loadEnv } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')
  const proxy = {
    '/api': { target: env.API_PROXY_TARGET || 'http://127.0.0.1:3000', changeOrigin: false },
  }
  return {
    plugins: [vue()],
    server: { host: 'localhost', port: 5173, strictPort: true, proxy },
    preview: { host: 'localhost', port: 5173, strictPort: true, proxy },
    build: { target: 'es2022' },
  }
})
