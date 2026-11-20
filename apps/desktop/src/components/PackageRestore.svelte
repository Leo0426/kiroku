<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import type { LanguageBatchResult, RestoreReview, RestoreRules } from '../lib/types'
  import { listen } from '../lib/events'
  import ProgressBar from './ProgressBar.svelte'
  import Icon from './Icon.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  const fields: { key: keyof RestoreRules; label: string }[] = [
    { key: 'title', label: '标题' }, { key: 'tags', label: '标签' }, { key: 'actors', label: '演员' },
    { key: 'favorite', label: '收藏' }, { key: 'metadata', label: '其他影片资料' },
    { key: 'poster', label: '海报' }, { key: 'fanart', label: '背景图' }, { key: 'avatars', label: '演员头像' },
  ]
  const labels: Record<string,string> = {
    title:'标题', original_title:'原始标题', plot:'简介', actors:'演员', tags:'标签', favorite:'收藏',
    director:'导演', studio:'片商', series:'系列', publisher:'发行商', release_date:'发行日期', runtime:'时长',
    locked_fields:'人工锁定字段', text_versions:'多语言文本', scores:'评分', artwork:'海报裁剪', hosted_actor_images:'托管头像',
  }
  const statuses = { ready:'可导回', unchanged:'已一致或按规则保留', unmatched:'当前库未匹配', error:'读取失败' }
  const initialRules = Object.fromEntries(fields.map(field => [field.key,'fill'])) as RestoreRules
  let opened = $state(false), directory = $state(''), rules = $state<RestoreRules>({...initialRules})
  let review = $state.raw<RestoreReview | null>(null), result = $state.raw<LanguageBatchResult | null>(null)
  let task = $state.raw<CancellableJob<unknown> | null>(null), busy = $state(''), error = $state('')
  let progress = $state({done:0,total:0})
  $effect(() => {
    let mounted = true, off: (() => void) | undefined
    listen('package:progress', event => { if (event.jobId === task?.jobId) progress = event })
      .then(remove => { if(mounted) off = remove; else remove() })
    return () => { mounted = false; off?.() }
  })
  let selected = $state<string[]>([]), limit = $state(40), generation = 0
  const ready = $derived(review?.items.filter(row => row.status === 'ready').map(row => row.code) ?? [])
  const unmatched = $derived(review?.items.filter(row => row.status === 'unmatched').length ?? 0)

  function reset() {
    generation++
    progress = {done:0,total:0}
    void task?.cancel()
    task = null; busy = ''; error = ''; review = null; result = null; selected = []; limit = 40
  }
  $effect(() => { app.settings; untrack(() => { reset(); directory = ''; opened = false }) })
  onDestroy(reset)
  function format(value: unknown): string {
    if (value == null || value === '') return '（空）'
    if (Array.isArray(value)) return value.map(format).join('、') || '（空）'
    if (typeof value === 'boolean') return value ? '是' : '否'
    if (typeof value === 'object') return 'name' in value ? String(value.name) : JSON.stringify(value)
    return String(value)
  }
  async function choose() {
    const current = generation
    try {
      const path = await api.pickDirectory('选择资料包文件夹', directory || undefined)
      if (path && current === generation) { reset(); directory = path }
    } catch (e) { if (current === generation) error = String(e) }
  }
  async function inspect() {
    if (busy || !directory) return
    reset()
    const current = generation
    busy = '正在校验资料并匹配当前媒体库…'
    const job = cancellableJob(id => api.packageRestorePreview(directory, $state.snapshot(rules), id), e => app.toast('error',String(e)))
    task = job
    try {
      const value = await job.promise
      if (current !== generation || job.cancelled) return
      review = value
      selected = value.items.filter(row => row.status === 'ready').map(row => row.code)
    } catch (e) { if (current === generation) error = String(e) }
    finally { if (current === generation) { busy = ''; task = null } }
  }
  async function apply() {
    if (busy || !review || !selected.length) return
    const token = review.token, codes = [...selected], current = generation
    progress = {done:0,total:codes.length}
    error = ''; busy = '正在导回选中影片…'
    const job = cancellableJob(id => api.packageRestoreApply(token, codes, id), e => app.toast('error', String(e)))
    task = job
    try {
      const value = await job.promise
      if (current === generation) { result = value; review = null; if (value.results.length) app.libraryVersion++ }
    } catch (e) { if (current === generation) { error = String(e); review = null } }
    finally { if (current === generation) { busy = ''; task = null } }
  }
</script>

<section aria-label="资料包导回">
  <button class="btn" aria-expanded={opened} onclick={() => { if (opened) reset(); opened = !opened }}>
    <Icon name="folder" size={15}/>从资料包导回<Icon name="chevron-down" size={15}/>
  </button>
  {#if opened}
    <div class="restore-body">
      {#if result}
        <p role="status">{result.cancelled ? '已停止。' : ''}已导回 {result.results.filter(r => !r.error).length} 部影片。可在操作历史中逐部撤销。</p>
        {#each result.failed as message}<p role="alert">{message}</p>{/each}
        {#each result.results as item}
          {#if item.error || item.indexError}<p role="alert">{item.code} · {item.error || item.indexError}</p>{/if}
        {/each}
        <button class="btn" onclick={reset}>继续导回</button>
      {:else if review}
        <p><strong>{ready.length} 部可导回</strong> · 共检查 {review.items.length} 部</p>
        {#if unmatched}<p>{unmatched} 部在当前媒体库中没有同编号影片。含视频的资料包可从「导入影片」添加。</p>{/if}
        <div class="actions"><button class="btn sm" disabled={!!busy || !ready.length} onclick={() => selected = selected.length === ready.length ? [] : [...ready]}>{selected.length === ready.length ? '取消全选' : '选中全部可导回影片'}</button></div>
        {#each review.items.slice(0,limit) as row (row.code)}
          <article>
            <label><input type="checkbox" class="checkbox" value={row.code} bind:group={selected} disabled={!!busy || row.status !== 'ready'}/> <strong>{row.code}</strong> · {statuses[row.status]}</label>
            {#if row.destination}<div class="path">{row.destination}</div>{/if}
            {#if row.changes.length || row.files.length}
              <details><summary>查看字段差异和写入文件</summary>
                {#each row.changes as change}<div><b>{labels[change.field] ?? change.field}</b>　{format(change.before)} → {format(change.after)}</div>{/each}
                {#each row.files as path}<div class="path">{path}</div>{/each}
              </details>
            {/if}
            {#each row.warnings as message}<p>{message}</p>{/each}
          </article>
        {/each}
        {#if limit < review.items.length}<button class="btn sm" onclick={() => limit += 40}>显示更多影片</button>{/if}
        <div class="actions"><button class="btn" disabled={!!busy} onclick={reset}>修改导回规则</button>
          <button class="btn primary" disabled={!!busy || !selected.length} onclick={apply}>确认导回（{selected.length}）</button></div>
      {:else}
        <p>按编号匹配当前媒体库，只写入预览中选定的影片资料。</p>
        <div class="actions"><button class="btn" onclick={choose} disabled={!!busy}>选择资料包文件夹</button>{#if directory}<span class="path">{directory}</span>{/if}</div>
        <p class="path">写入媒体库　{app.settings?.library_root}</p>
        {#if app.settings?.actor_hosting.enabled}<p class="path">头像托管目录　{app.settings.actor_hosting.directory}</p>{/if}
        <fieldset disabled={!!busy}><legend>同类资料的处理方式</legend>
          <div class="rule-grid">{#each fields as field}
            <label>{field.label}<select aria-label={field.label + '导回规则'} bind:value={rules[field.key]}>
              <option value="fill">只补空缺</option><option value="keep">保留本库</option><option value="replace">采用包内资料</option>
            </select></label>
          {/each}</div>
        </fieldset>
        <button class="btn primary" onclick={inspect} disabled={!!busy || !directory}>预览导回</button>
      {/if}
      {#if busy}<LoadingStatus label={busy}/>{#if progress.total}<ProgressBar label="资料包导回进度" value={progress.done/progress.total} active/><span>{progress.done} / {progress.total} 部</span>{/if}<button class="btn sm" onclick={() => void task?.cancel()}>停止导回任务</button>{/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </div>
  {/if}
</section>

<style>
  .restore-body { display:grid; gap:var(--s3); margin-top:var(--s2); padding:var(--s3); border-radius:var(--r-sm); background:var(--surface-2); overflow-wrap:anywhere; }
  .restore-body > button { justify-self:start; }
  .actions { display:flex; align-items:center; gap:var(--s2); flex-wrap:wrap; }
  fieldset { border:0; padding:0; min-width:0; }
  legend { font-weight:600; margin-bottom:var(--s2); }
  .rule-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,134px),1fr)); gap:var(--s2); }
  .rule-grid label { display:grid; gap:4px; }
  select { width:100%; min-height:32px; }
  .path { color:var(--text-2); font-size:var(--fs-sm); overflow-wrap:anywhere; }
  article { border-top:1px solid var(--line); padding-top:var(--s2); }
  article > label { display:flex; gap:var(--s2); align-items:center; }
  details { margin-top:var(--s1); }
  summary { cursor:pointer; }
  [role=alert] { color:var(--danger); }
</style>
