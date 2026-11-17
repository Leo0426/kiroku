<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import type { Metadata, RevisionPreview } from '../lib/types'
  import Icon from './Icon.svelte'
  let { meta, onsaved }: { meta: Metadata; onsaved: () => void } = $props()
  let phase = $state<'idle' | 'translating' | 'preview' | 'saving' | 'done'>('idle')
  let preview = $state<RevisionPreview | null>(null)
  let error = $state(''), elapsed = $state(0), cancelled = $state(false)
  let task: CancellableJob<RevisionPreview> | null = null
  let timer: ReturnType<typeof setInterval> | undefined
  let alive = true
  onDestroy(() => { alive = false; clearInterval(timer); void task?.cancel() })
  async function translate() {
    phase = 'translating'; error = ''; preview = null; elapsed = 0; cancelled = false
    const started = Date.now()
    timer = setInterval(() => elapsed = Math.floor((Date.now() - started) / 1000), 1000)
    const request = cancellableJob(id => api.metadataUpdatePreview(meta.code, { kind: 'translate' }, id), e => { if (alive) error = String(e) })
    task = request
    try {
      const result = await request.promise
      if (!alive) return
      if (request.cancelled) { cancelled = true; phase = 'idle'; return }
      preview = result; phase = 'preview'
    } catch (e) {
      if (alive) { cancelled = request.cancelled; error = cancelled ? '' : String(e); phase = 'idle' }
    } finally { clearInterval(timer); task = null }
  }
  async function save() {
    if (!preview) return
    const p = preview; phase = 'saving'; error = ''
    try {
      const result = await api.revisionSave(p.token, p.root, p.code)
      if (!alive) return
      if (result.error || result.indexError) throw [result.error, result.indexError].filter(Boolean).join('；')
      preview = null; phase = 'done'; onsaved(); app.toast('ok', '译文已保存，可从历史撤销')
    } catch (e) { if (alive) { error = String(e); preview = null; phase = 'idle' } }
  }
</script>
<section aria-label="单片翻译" class:active={phase !== 'idle' || !!error || cancelled}>
  <button class="btn" disabled={phase === 'translating' || phase === 'saving'} onclick={translate}><Icon name="globe" size={15}/>{phase === 'translating' ? '正在翻译…' : '翻译此影片'}</button>
  {#if phase === 'translating'}
    <div role="status" aria-live="polite"><span>正在生成标题与简介译文 · 已用 {elapsed} 秒</span><progress aria-label="翻译进度"></progress></div>
    <button class="btn ghost" onclick={() => task?.cancel()}>取消翻译</button>
  {:else if phase === 'saving'}
    <div role="status">正在保存译文与 NFO…<progress aria-label="译文保存进度"></progress></div>
  {:else if preview}
    <div class="preview" aria-label="翻译预览">
      <p role="status">翻译完成，请确认译文</p>
      {#each ['title', 'plot'] as const as field}
        {#if preview.after.text_versions?.localized['zh-Hans']?.[field] != null}
          <h4>{field === 'title' ? '标题' : '简介'}</h4><p>{preview.after.text_versions.localized['zh-Hans'][field]}</p>
        {/if}
      {/each}
      <button class="btn primary" onclick={save}>保存译文</button>
      <button class="btn ghost" onclick={() => { preview = null; phase = 'idle' }}>放弃译文</button>
    </div>
  {:else if phase === 'done'}<span role="status">译文已保存</span>
  {:else if cancelled}<span role="status">翻译已取消</span>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
  section { display:flex; align-items:center; flex-wrap:wrap; gap:12px; margin:12px 0; }
  section.active { padding:16px; background:var(--surface-2); border-radius:var(--r-sm); }
  [role=status] { font-size:var(--fs-sm); color:var(--text-2); }
  progress { display:block; width: min(280px, 100%); height:6px; margin-top:8px; accent-color:var(--accent); }
  .preview, [role=alert] { flex-basis:100%; min-width:0; }
  p { white-space:pre-wrap; overflow-wrap:anywhere; }
</style>
