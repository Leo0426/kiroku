import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // 浏览器 mock 直接读取核心的内置标签译名
  server: { port: 1420, strictPort: true, fs: { allow: ['.', '../../crates/kiroku-core/assets'] } },
})
