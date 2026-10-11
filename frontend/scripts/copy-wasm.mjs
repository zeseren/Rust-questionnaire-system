// 把 wasm-pack 的 bundler 产物同步到 src/wasm/pkg/，让不装 Rust 工具链的前端开发者
// 也能拿到与后端同源的引擎。只复制引擎五件套：胶水脚本、导入表、模块本体与两份类型
// 声明；wasm-pack 顺带生成的 package.json / .gitignore 不进源码树。
import { cp, mkdir, rm } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const source = resolve(here, '../../logic-wasm/pkg')
const target = resolve(here, '../src/wasm/pkg')
const files = [
  'logic_wasm.js',
  'logic_wasm_bg.js',
  'logic_wasm_bg.wasm',
  'logic_wasm.d.ts',
  'logic_wasm_bg.wasm.d.ts',
]

await mkdir(target, { recursive: true })
for (const file of files) {
  const destination = resolve(target, file)
  await rm(destination, { force: true })
  await cp(resolve(source, file), destination)
}
console.log(`已同步 ${files.length} 个 wasm 产物到 ${target}`)
