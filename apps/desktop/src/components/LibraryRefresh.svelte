<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { inTauri } from '../lib/events'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import type { LibraryPruned } from '../lib/types'
  import Icon from './Icon.svelte'

  let task = $state<CancellableJob<LibraryPruned> | null>(null)
  let busy = $state(false)
  let error = $state('')
  let generation = 0
  let observedRoot: string | null | undefined

  function invalidate() {
    generation++
    void task?.cancel()
    task = null
    busy = false
    error = ''
  }
  $effect(() => {
    const root = app.settings?.library_root ?? null
    const recovering = app.recovery.loading
    untrack(() => {
      if (root !== observedRoot) { observedRoot = root; invalidate() }
      if (!recovering) void refresh(true)
    })
  })
  onDestroy(invalidate)
  onMount(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    const focus = () => { if (!disposed) void refresh(true) }
    if (inTauri) {
      void import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
        getCurrentWindow().onFocusChanged(({ payload }) => { if (payload) focus() })
      ).then(off => { if (disposed) off(); else unlisten = off })
        .catch(() => { if (!disposed) error = '自动刷新监听不可用，请点击刷新媒体库' })
    } else window.addEventListener('focus', focus)
    return () => { disposed = true; unlisten?.(); window.removeEventListener('focus', focus) }
  })

  async function refresh(automatic = false) {
    const root = app.settings?.library_root
    if (!root || busy || (automatic && app.recovery.loading)) return
    const g = generation
    busy = true
    error = ''
    const request = cancellableJob(id => {
      if (g !== generation || root !== app.settings?.library_root) throw new Error('媒体库已变化')
      return api.libraryPruneMissing(root, automatic, id)
    }, e => { if (g === generation) error = `停止刷新失败：${String(e)}` })
    task = request
    try {
      const result = await request.promise
      if (g !== generation || root !== app.settings?.library_root || result.root !== root) return
      error = ''
      if (result.skipped) return
      if (result.removed > 0) app.toast('ok', `已移除 ${result.removed} 部已移走的影片`)
      else if (!automatic) app.toast('info', '媒体库已刷新，没有已移走的影片')
      if (result.removed > 0 || !automatic) app.libraryVersion++
    } catch (e) {
      if (g === generation && root === app.settings?.library_root) {
        if (request.cancelled) app.toast('info', '刷新已停止，保留原索引')
        else error = `刷新未完成，保留原索引：${String(e)}`
      }
    } finally {
      if (g === generation) { task = null; busy = false }
    }
  }
</script>

<div class="refresh">
  <button class="btn" title="检查已移走的影片，只移除媒体库记录；不会删除文件" disabled={busy} onclick={() => refresh()}>
    <Icon name={busy ? 'loader' : 'refresh'} size={14} />{busy ? '正在刷新…' : '刷新媒体库'}
  </button>
  {#if busy}<button class="btn sm ghost" onclick={() => task?.cancel()}>停止刷新</button>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</div>

<style>
  .refresh { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); max-width: 100%; }
  p { flex-basis: 100%; max-width: 400px; color: var(--danger); font-size: var(--fs-sm); overflow-wrap: anywhere; }
</style>
