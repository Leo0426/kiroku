<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import type { LibraryPruned } from '../lib/types'
  import Icon from './Icon.svelte'
  import PopupNotice from './PopupNotice.svelte'

  let { onrefreshed }: { onrefreshed?: () => void } = $props()
  let review = $state.raw<{ root: string; codes: string[]; summary: string } | null>(null)
  let dialog: HTMLDialogElement
  let keepButton: HTMLButtonElement | undefined = $state()
  $effect(() => {
    if (review && !dialog.open) {
      dialog.showModal()
      keepButton?.focus()
    } else if (!review && dialog.open) dialog.close()
  })

  let task = $state<CancellableJob<LibraryPruned> | null>(null)
  let busy = $state(false)
  let error = $state('')
  let generation = 0
  let observedRoot: string | null | undefined

  function invalidate() {
    review = null
    generation++
    void task?.cancel()
    task = null
    busy = false
    error = ''
  }
  $effect(() => {
    const root = app.settings?.library_root ?? null
    untrack(() => {
      if (root !== observedRoot) { observedRoot = root; invalidate() }
    })
  })
  onDestroy(invalidate)

  export function refresh() { return reconcile([]) }

  async function reconcile(removeMissing: string[]) {
    const root = app.settings?.library_root
    if (!root || busy || review) return
    const g = generation
    busy = true
    error = ''
    const request = cancellableJob(id => {
      if (g !== generation || root !== app.settings?.library_root) throw new Error('媒体库已变化')
      return api.libraryPruneMissing(root, false, id, removeMissing)
    }, e => { if (g === generation) error = `停止刷新失败：${String(e)}` })
    task = request
    try {
      const result = await request.promise
      if (g !== generation || root !== app.settings?.library_root || result.root !== root) return
      error = ''
      if (result.skipped) return
      const changes = []
      if (result.relocated) changes.push(`已重新找到 ${result.relocated} 部影片`)
      if (result.refreshed) changes.push(`已更新 ${result.refreshed} 部影片的海报`)
      if (result.ambiguous) changes.push(`${result.ambiguous} 部影片有多个匹配目录，已保留原记录`)
      if (result.removed) changes.push(`已移除 ${result.removed} 部已移走的影片的索引记录`)
      const summary = changes.join('。')
      if (result.missingCodes?.length && !removeMissing.length) {
        review = { root, codes: result.missingCodes, summary }
      } else {
        if (result.missingCodes?.length) changes.push(`另有 ${result.missingCodes.length} 条未确认的记录已保留`)
        app.toast(changes.length ? 'ok' : 'info', changes.join('。') || '媒体库已刷新，没有已移走的影片')
      }
      if (!removeMissing.length) onrefreshed?.()
      app.libraryVersion++
    } catch (e) {
      if (g === generation && root === app.settings?.library_root) {
        if (request.cancelled) app.toast('info', '刷新已停止，保留原索引')
        else error = `刷新未完成，保留原索引：${String(e)}`
      }
    } finally {
      if (g === generation) { task = null; busy = false }
    }
  }
  function removeConfirmed() {
    const current = review
    review = null
    if (current && current.root === app.settings?.library_root) void reconcile(current.codes)
  }
</script>

<div class="refresh">
  <button class="btn" title="重新查找影片与海报。库内找不到的影片可在确认后移除索引记录，磁盘文件会保留。" disabled={busy} onclick={() => refresh()}>
    <Icon name={busy ? 'loader' : 'refresh'} size={14} />{busy ? '正在刷新…' : '刷新媒体库'}
  </button>
  {#if busy}<button class="btn sm ghost" onclick={() => task?.cancel()}>停止刷新</button>{/if}
  {#if error}<PopupNotice text={error} />{/if}
</div>

<dialog class="app-dialog review" bind:this={dialog} role="alertdialog" aria-labelledby="missing-title" aria-describedby="missing-description"
  oncancel={e => { e.preventDefault(); review = null }}>
  {#if review}
    <h2 id="missing-title">有 {review.codes.length} 部影片未找到</h2>
    <p id="missing-description">请确认媒体盘已连接，影片目录可访问。可以保留记录，稍后重新刷新，也可以将这些条目从媒体库移除。两种操作都会保留磁盘上的文件。</p>
    {#if review.summary}<p>{review.summary}</p>{/if}
    <details><summary>查看影片编号</summary><p class="codes">{review.codes.join('、')}</p></details>
    <div class="review-actions">
      <button class="btn" onclick={removeConfirmed}>移除这 {review.codes.length} 条索引</button>
      <button class="btn primary" bind:this={keepButton} onclick={() => review = null}>保留记录</button>
    </div>
  {/if}
</dialog>

<style>
  .review { padding: var(--s5); }
  .review h2 { font-size: var(--fs-lg); }
  .review p, .review details { margin-top: var(--s3); font-size: var(--fs-sm); line-height: 1.6; }
  .review summary { cursor: pointer; }
  .codes { overflow-y: auto; max-height: 22vh; overflow-wrap: anywhere; color: var(--text-2); }
  .review-actions { display: flex; flex-wrap: wrap; gap: var(--s2); justify-content: flex-end; margin-top: var(--s5); }
  .refresh { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); max-width: 100%; }
</style>
