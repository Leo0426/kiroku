<script lang="ts">
  import { applyNotice } from '../lib/notify'
  import { baseName } from '../lib/format'
  import { libraryLocationSetting } from '../lib/settings-search'
  import { app } from '../lib/state.svelte'
  import type { ApplyDone, ApplyEvent, PlanItem, RollbackResult } from '../lib/types'
  import Icon from './Icon.svelte'

  let { result, events, plans, rollback, undone, currentLibrary }: {
    result: ApplyDone
    events: ApplyEvent[]
    plans: PlanItem[]
    rollback: RollbackResult | null
    undone: boolean
    currentLibrary: boolean
  } = $props()
  const notes = $derived(events.filter(e => e.warnings.length))
  const notice = $derived(applyNotice(result, notes.length))
  const indexError = $derived(rollback ? rollback.indexError : result.indexError)
  const errors = $derived(rollback ? rollback.failures.map(message => ['', message] as const) : result.failed)
  const warning = $derived(rollback ? !undone || !!indexError : !!(result.cancelled || errors.length || result.imageFailures || indexError))
  let expanded = $state(false)
  let notesOpen = $state(false)
  const sourcePaths = $derived(new Map(plans.map(p => [p.itemId, p.from])))
</script>

<section class="outcome" aria-label="本批整理结果">
  <span class="mark" class:warning><Icon name={undone ? 'undo' : warning ? 'alert' : 'check'} size={30} /></span>
  <h2>{undone ? '本批已撤销' : rollback ? '本批撤销未完成' : notice.title}</h2>
  {#if !rollback}
    <p class="muted">{notice.body}</p>
  {:else if !undone}
    <p class="muted">核对本批操作后，可以重试撤销。</p>
  {/if}
  {#if errors.length}
    <div class="errors">
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (长错误列表需要支持键盘滚动) -->
      <ul tabindex="0" aria-label={rollback ? '撤销错误' : '执行错误'}>
        {#each (expanded ? errors : errors.slice(0, 1)) as [code, message]}
          <li>{#if code}<strong>{code}</strong>{/if}<span>{message}</span></li>
        {/each}
      </ul>
      {#if errors.length > 1}
        <button type="button" class="btn sm ghost" aria-expanded={expanded} onclick={() => expanded = !expanded}>
          {expanded ? '收起错误' : `查看全部 ${errors.length} 条错误`}
        </button>
      {/if}
    </div>
  {/if}
  {#if indexError}
    <div class="index-issue">
      <p role="alert">{rollback ? '撤销后索引更新失败：' : ''}{indexError}</p>
      {#if currentLibrary}
        <button class="btn sm" onclick={() => app.navigate('settings', libraryLocationSetting)}>前往索引设置</button>
      {/if}
    </div>
  {/if}
  {#if !rollback}
    {#if result.imageFailures}
      <p class="hint">打开缺图影片的详情，使用「补缺图」可以重试。</p>
    {/if}
    {#if notes.length}
      <details class="notes" bind:open={notesOpen}>
        <summary>查看文件提示</summary>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex (文件提示可独立滚动) -->
        <div class="file-notes" role="region" tabindex="0" aria-label="文件提示明细">
          {#if notesOpen}
            {#each notes as event (event.itemId)}
              {@const path = sourcePaths.get(event.itemId)}
              <div>
                <strong title={path}>{path ? baseName(path) : event.code}</strong>
                <ul>{#each event.warnings as note}<li>{note}</li>{/each}</ul>
              </div>
            {/each}
          {/if}
        </div>
      </details>
    {/if}
  {/if}
  {#if result.cleaned.length}
    <p class="hint">整理时已将 {result.cleaned.length} 个原文件夹移到废纸篓。{undone ? '如需其中的其他文件，可从废纸篓找回。' : ''}</p>
  {/if}
</section>

<style>
  .outcome { display: grid; justify-items: center; gap: var(--s3); width: 100%; min-width: 0; }
  .mark { width: 68px; height: 68px; display: grid; place-items: center; border-radius: 50%; color: var(--ok); background: var(--ok-soft); }
  .mark.warning { color: var(--warn); background: var(--warn-soft); }
  h2 { font-size: var(--fs-lg); font-weight: 600; }
  .hint { color: var(--text-2); font-size: var(--fs-sm); }
  .errors, .index-issue, .notes { width: min(100%, 560px); text-align: start; overflow-wrap: anywhere; }
  .errors { padding: var(--s3); background: var(--danger-soft); border-radius: var(--r-sm); }
  .errors strong { margin-inline-end: var(--s2); }
  ul { list-style: none; margin: 0; padding: 0; }
  .errors ul, .file-notes { max-height: 32vh; overflow-y: auto; }
  .notes { background: var(--surface-2); border-radius: var(--r-sm); font-size: var(--fs-sm); }
  summary { cursor: pointer; padding: var(--s3); }
  .file-notes { padding: 0 var(--s3) var(--s3); }
  .file-notes > div + div { margin-top: var(--s3); }
  li { padding: var(--s1); font-size: var(--fs-sm); }
  .errors button, .index-issue button { margin-top: var(--s2); }
</style>
