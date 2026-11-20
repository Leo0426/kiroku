<script lang="ts">
  import Icon from './Icon.svelte'
  import LoadingStatus from './LoadingStatus.svelte'
  import ProgressBar from './ProgressBar.svelte'
  import { app } from '../lib/state.svelte'
  import { api } from '../lib/api'
  import { flow } from '../lib/importflow.svelte'
  import type { RecoveryEntry } from '../lib/types'

  const recovery = app.recovery
  const currentRoot = $derived(app.settings?.library_root ?? null)
  const hasNotice = $derived(!!(recovery.visibleEntries.length || recovery.error))
  const report = $derived(JSON.stringify([recovery.visibleEntries, recovery.error]))
  const busy = $derived(!!recovery.busyId)
  const progress = $derived(recovery.preserving ? recovery.progress : null)
  let dialog: HTMLDialogElement
  let open = $state(false)
  let actionError = $state('')
  let shownRoot: string | null = null
  let shownReport = ''

  $effect(() => {
    if (shownRoot !== currentRoot) {
      shownRoot = currentRoot
      shownReport = ''
      open = false
      actionError = ''
    }
    if (!currentRoot || recovery.root !== currentRoot || recovery.loading || busy || app.dialog || app.commandsOpen) return
    if (report !== shownReport) {
      shownReport = report
      open = hasNotice
    }
  })

  $effect(() => {
    if (open && !dialog.open) dialog.showModal()
    else if (!open && dialog.open) dialog.close()
  })

  function close() {
    if (busy) return
    shownReport = report
    actionError = ''
    open = false
  }

  function showHistory() {
    close()
    app.navigate('history')
  }

  async function undo(entry: RecoveryEntry) {
    const root = recovery.root
    if (!await app.confirm({
      title: '检查并撤销中断操作',
      body: '将重新检查日志与磁盘，只撤销本次拥有且未被外部修改的产物。损坏或无法确认的文件会保留并报告。',
      confirmLabel: '确认撤销',
    }) || root !== currentRoot) return
    actionError = ''
    try { await recovery.undo(entry) } catch (e) { actionError = String(e) }
  }

  async function preserve(entry: RecoveryEntry) {
    const root = recovery.root
    if (flow.busy || !await app.confirm({
      title: '保留成果后继续',
      body: '核对中断操作并更新索引，保留已完成的文件和原日志。随后重新选择来源、扫描剩余文件并确认新计划。已有未执行导入预览将重置。已移走视频若缺少配套资料，仍需在库健康检查中核对。',
      confirmLabel: '核对并继续',
    }) || root !== currentRoot) return
    actionError = ''
    try {
      if (!await recovery.preserve(entry) || root !== currentRoot) return
      close()
      flow.reset()
      flow.cachedSourcesOnly = true
      app.navigate('import')
      app.toast('info', '已保留整理成果，请核对来源后扫描剩余文件')
    } catch (e) { actionError = String(e) }
  }

  function acknowledge() {
    try {
      recovery.acknowledge()
      close()
    } catch {
      actionError = '无法保存提醒设置，请重试'
    }
  }

  async function reveal(id: string) {
    actionError = ''
    try { await api.revealJournal(id) } catch (e) { actionError = String(e) }
  }
</script>

{#if !open && recovery.root === currentRoot && currentRoot}
  {#if recovery.loading}
    <div class="recovery-popup"><LoadingStatus label="正在检查整理日志…" /></div>
  {:else if hasNotice}
    <button class="btn recovery-popup" onclick={() => open = true}><Icon name="alert" size={16} />查看恢复事项</button>
  {/if}
{/if}

<dialog class="app-dialog recovery" bind:this={dialog} aria-labelledby="recovery-title"
  oncancel={(e) => { e.preventDefault(); close() }}>
  {#if open}
    <div class="content">
      <header>
        <span class="mark"><Icon name="history" size={20} /></span>
        <div class="heading">
          <h2 id="recovery-title">启动恢复</h2>
          <p class="muted">{recovery.visibleEntries.length ? `${recovery.visibleEntries.length} 条整理记录待处理` : '检查媒体库的整理日志'}</p>
        </div>
        <button class="btn icon ghost" aria-label="关闭恢复事项" disabled={busy} onclick={close}><Icon name="x" size={16} /></button>
      </header>

      {#if recovery.loading || busy}
        <div class="progress">
          {#if recovery.preserving}
            <div role="status">
              {#if recovery.stopping}正在停止，完成当前安全步骤后返回。
              {:else if progress?.stage === 'index'}正在更新媒体库索引：{progress.done} / {progress.total} 个目录
              {:else if progress?.stage === 'recover'}正在核对中断文件…
              {:else if progress?.stage === 'finished'}正在刷新记录…
              {:else}正在读取并核对整理日志…{/if}
              {#if progress}<span class="muted"> · 已用 {Math.floor(progress.elapsedMs / 1000)} 秒</span>{/if}
            </div>
            <ProgressBar label="恢复进度" active value={progress?.stage === 'index' && progress.total ? progress.done / progress.total : undefined} />
            <button class="btn sm" disabled={recovery.stopping} onclick={() => recovery.cancelPreserve()}>停止继续整理</button>
          {:else}
            <LoadingStatus label={busy ? '正在处理恢复事项…' : '正在检查整理日志…'} />
            <ProgressBar label="恢复检查进度" />
          {/if}
        </div>
      {/if}

      {#if recovery.error || actionError}<p class="error" role="alert">{actionError || recovery.error}</p>{/if}
      {#if recovery.stale && recovery.visibleEntries.length}<p class="muted">上次记录，尚未核对。检查完成后可处理这些记录。</p>{/if}
      <p class="muted root">{recovery.root}</p>

      {#each recovery.visibleEntries as entry (entry.id)}
        <section class="entry">
          <h3>{entry.codes.join('、') || `日志 ${entry.id}`}</h3>
          <p>{entry.message}</p>
          <div class="actions">
            <button class="btn sm" onclick={() => reveal(entry.id)} disabled={recovery.stale || !/^\d+$/.test(entry.id)}>查看日志位置</button>
            {#if entry.canPreserve}
              <button class="btn sm" disabled={recovery.stale || busy || flow.busy} onclick={() => preserve(entry)}>保留成果后继续</button>
            {/if}
            {#if entry.canUndo}
              <button class="btn sm" disabled={recovery.stale || busy} onclick={() => undo(entry)}>检查并撤销</button>
            {/if}
          </div>
        </section>
      {/each}

      <footer class="actions">
        <button class="btn sm ghost" onclick={showHistory} disabled={busy}>查看完整历史</button>
        <button class="btn sm" onclick={() => { actionError = ''; void recovery.refresh() }} disabled={recovery.loading || busy}>重新检查恢复事项</button>
        {#if recovery.visibleEntries.length}
          <button class="btn sm primary" disabled={recovery.loading || busy} onclick={acknowledge}>已知晓，不再提醒</button>
        {/if}
      </footer>
    </div>
  {/if}
</dialog>

<style>
  .recovery { --dialog-width: 620px; }
  .content { padding: var(--s5); display: grid; gap: var(--s3); font-size: var(--fs-sm); }
  header, .actions { display: flex; gap: var(--s2); align-items: center; }
  header { gap: var(--s3); }
  .heading { flex: 1; }
  .mark { display: grid; place-items: center; width: 40px; height: 40px; flex-shrink: 0; background: var(--warn-soft); color: var(--warn); border-radius: var(--r-md); }
  h2 { font-size: var(--fs-xl); }
  h3 { font-size: var(--fs-md); }
  .entry, footer { border-top: 1px solid var(--line); padding-top: var(--s3); }
  .entry { display: grid; gap: var(--s2); }
  .entry p { color: var(--text-2); }
  .actions { flex-wrap: wrap; }
  footer { justify-content: flex-end; }
  .progress { display: grid; gap: var(--s2); padding: var(--s3); background: var(--surface-2); border-radius: var(--r-sm); }
  .progress button { justify-self: start; }
  .error { color: var(--danger); }
  .root { font-size: var(--fs-xs); }
  .recovery-popup { position: fixed; z-index: 55; left: calc(var(--sidebar-w) + var(--s4)); bottom: var(--s4); background: var(--surface-1); border: 1px solid var(--line-strong); border-radius: var(--r-md); box-shadow: var(--shadow-2); }
  div.recovery-popup { padding: var(--s1) var(--s3); pointer-events: none; }
  @media (max-width: 820px) { .recovery-popup { left: calc(64px + var(--s3)); } }
  @media (max-width: 720px) { .recovery-popup { left: var(--s3); bottom: calc(64px + var(--s3) + env(safe-area-inset-bottom)); } }
</style>
