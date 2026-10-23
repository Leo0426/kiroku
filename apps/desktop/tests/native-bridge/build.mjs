import { build } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { resolve } from 'node:path'
const outDir = process.argv[2]
if (!outDir) throw new Error('Provide an isolated output directory')
await build({
  configFile: false, root: import.meta.dirname, base: './', plugins: [svelte()],
  build: { outDir: resolve(outDir), emptyOutDir: false, target: 'safari17', minify: false },
})
