<script module lang="ts">
  import { SvelteMap } from 'svelte/reactivity'
  import type { Actor, VersionPhase } from '../lib/types'

  interface Draft {
    values: Record<string, string>
    actors?: Actor[]
    phase?: VersionPhase
    syncVersion: boolean
  }
  // Keep only edits in memory so reopening a film picks up its latest untouched fields.
  const drafts = new SvelteMap<string, Draft>()
</script>

<script lang="ts">
  import ActorAvatar from './ActorAvatar.svelte'
  import Icon from './Icon.svelte'
  import { onDestroy, tick, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { EDITABLE_FIELDS, type Metadata, type RevisionPreview, type RevisionResult } from '../lib/types'
  let { meta, onsaved, onactive }: { meta: Metadata; onsaved: () => void; onactive?: (active: boolean) => void } = $props()
  let collapsed = $state(false)
  let editing = $state(false), busy = $state(false), error = $state('')
  const textFields = EDITABLE_FIELDS.filter(field => field.key !== 'actors')
  let values = $state<Record<string, string>>({})
  let actors = $state<Actor[]>([])
  const genderLabel = { female: '女', male: '男', unknown: '未知' }
  const libraryRoot = $derived(app.settings?.library_root)
  const savedThumbs = $derived(new Set(meta.actors.map(actor => actor.thumb)))
  let preview = $state<RevisionPreview | null>(null)
  let result = $state<RevisionResult | null>(null)
  const phaseOptions = [{key:'subtitle',label:'中文字幕'}, {key:'uncensored',label:'无码'}, {key:'cracked',label:'破解（含 LTX / U / UMR）'}, {key:'leaked',label:'流出'}] as const
  const emptyPhase = (): VersionPhase => ({subtitle:false,uncensored:false,cracked:false,leaked:false,definition:null})
  const savedPhase = () => meta.artwork_phase ?? meta.artwork?.phase ?? emptyPhase()
  let phase = $state<VersionPhase>(emptyPhase())
  let syncVersion = $state(false)
  const draftKey = untrack(() => JSON.stringify([libraryRoot, meta.code]))
  let originalValues = $state<Record<string, string>>({})
  let originalActors = $state<Actor[]>([])
  let originalPhase = $state<VersionPhase>(emptyPhase())
  const changedFields = $derived(textFields.filter(field => values[field.key] !== originalValues[field.key]))
  const actorsChanged = $derived(JSON.stringify(actors) !== JSON.stringify(originalActors))
  const phaseChanged = $derived(JSON.stringify(phase) !== JSON.stringify(originalPhase))
  const changedCount = $derived(changedFields.length + Number(actorsChanged) + Number(syncVersion || phaseChanged))
  let trigger: HTMLButtonElement
  let form: HTMLFormElement | undefined = $state()
  let previewHeading: HTMLHeadingElement | undefined = $state()
  let lastField: string | null = null
  const shortcut = /Mac/.test(navigator.platform) ? '⌘ ↵' : 'Ctrl ↵'
  $effect(() => {
    if (!editing) return
    if (!changedCount) { drafts.delete(draftKey); return }
    drafts.set(draftKey, {
      values: Object.fromEntries(changedFields.map(field => [field.key, values[field.key]])),
      actors: actorsChanged ? $state.snapshot(actors) : undefined,
      phase: phaseChanged ? $state.snapshot(phase) : undefined,
      syncVersion,
    })
  })
  function phaseText(value: unknown): string {
    if (!value) return '未记录'
    const p = value as VersionPhase
    return [...phaseOptions.filter(o=>p[o.key]).map(o=>o.label), p.definition].filter(Boolean).join('、') || '无标记'
  }
  let generation = 0
  onDestroy(() => { generation++ })
  function text(value: unknown): string {
    if (value == null) return ''
    if (Array.isArray(value)) return value.map(item => typeof item === 'string' ? item : `${item.name}（${genderLabel[item.gender as Actor['gender']] ?? '未知'}）`).join('\n')
    return String(value)
  }
  function start() {
    const draft = drafts.get(draftKey)
    originalValues = Object.fromEntries(textFields.map(f => [f.key, text(meta[f.key as keyof Metadata])]))
    originalPhase = { ...savedPhase() }
    originalActors = meta.actors.map(actor => ({ ...actor }))
    values = { ...originalValues, ...draft?.values }
    phase = { ...(draft?.phase ?? originalPhase) }
    syncVersion = draft?.syncVersion ?? false
    actors = (draft?.actors ?? originalActors).map(actor => ({ ...actor }))
    lastField = null
    editing = true; error = ''; preview = null; result = null
  }
  function cancelEdit() {
    editing = false; preview = null; error = ''; result = null
    drafts.delete(draftKey)
    trigger.focus()
  }
  async function addActor() {
    actors = [...actors, { name: '', gender: 'unknown' }]
    await tick()
    form?.querySelectorAll<HTMLInputElement>('.actor-name')[actors.length - 1]?.focus()
  }
  function rememberField(e: FocusEvent) {
    const input = e.target as HTMLElement
    if (input.matches('textarea, input, select')) lastField = input.getAttribute('aria-label')
  }
  async function returnToEdit() {
    preview = null
    await tick()
    const inputs = Array.from(form?.querySelectorAll<HTMLElement>('textarea, input, select') ?? [])
    const previous = inputs.find(input => lastField && input.getAttribute('aria-label') === lastField) ?? inputs[0]
    previous?.focus()
  }
  function previewShortcut(e: KeyboardEvent) {
    if (e.key !== 'Enter' || !(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey || e.repeat || e.isComposing || e.defaultPrevented) return
    if (!form?.contains(e.target as Node)) return
    e.preventDefault()
    void inspect()
  }
  function value(field: string, input: string): unknown {
    if (field === 'tags' || field === 'directors') return input.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
    if (field === 'runtime_min') { if (!input.trim()) return null; const n = Number(input); if (!Number.isInteger(n) || n < 0 || n > 4294967295) throw '片长必须为非负整数'; return n }
    return input === '' ? null : input
  }
  async function inspect() {
    if (busy || !changedCount) return
    const current = generation
    busy = true; error = ''; result = null
    try {
      if (actors.some(actor => !actor.name.trim())) throw '请填写演员姓名，或删除空白演员行'
      const patch: Record<string, unknown> = Object.fromEntries(changedFields.map(f => [f.key, value(f.key, values[f.key])]))
      const nextActors = actors.map(actor => {
        const next = { ...actor }
        if (next.thumb != null && !savedThumbs.has(next.thumb)) {
          const url = next.thumb.trim()
          if (url) next.thumb = url
          else delete next.thumb
        }
        return next
      })
      if (actorsChanged && JSON.stringify(nextActors) !== JSON.stringify(meta.actors)) patch.actors = nextActors
      if (syncVersion || phaseChanged) patch.artwork_phase = {...phase}
      if (!Object.keys(patch).length) throw '没有字段变化'
      const next = await api.revisionPreview(meta.code, patch)
      if (current === generation) {
        const moveFocus = form?.contains(document.activeElement) || document.activeElement === document.body
        preview = next
        await tick()
        if (moveFocus) previewHeading?.focus()
      }
    } catch (e) { if (current === generation) error = String(e) }
    finally { if (current === generation) busy = false }
  }
  async function save() {
    if (!preview || busy) return
    const current = generation, candidate = preview, submittedDraft = drafts.get(draftKey)
    busy = true; error = ''
    try {
      const saved = await api.revisionSave(candidate.token, candidate.root, candidate.code)
      if (!saved.error && !saved.indexError && drafts.get(draftKey) === submittedDraft) drafts.delete(draftKey)
      if (current !== generation) return
      result = saved; preview = null
      if (saved.error || saved.indexError) error = [saved.error, saved.indexError && `索引更新失败：${saved.indexError}`].filter(Boolean).join('；')
      else { editing = false; app.toast('ok', '人工修订已保存，可从历史撤销'); onsaved() }
    } catch (e) { if (current === generation) { error = String(e); preview = null } }
    finally {
      if (current === generation) {
        busy = false
        if (!editing) { await tick(); if (document.activeElement === document.body) trigger?.focus() }
      }
    }
  }
  $effect(() => onactive?.(editing && !collapsed))
</script>

<svelte:window onkeydown={previewShortcut} />

{#snippet actorList(items: Actor[], revisionToken?: string)}
  {#if items.length}
    <ul class="actor-list">
      {#each items as actor}
        <li>
          <ActorAvatar code={meta.code} url={actor.thumb} {libraryRoot} {revisionToken} size={32} placeholder />
          <span>{actor.name}（{genderLabel[actor.gender]}）{#if actor.thumb}<small>{actor.thumb}</small>{/if}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">（空）</p>
  {/if}
{/snippet}

{#snippet cancelButton()}
  <button type="button" class="btn ghost" disabled={busy} title={changedCount ? '放弃本片未保存的修改' : undefined} onclick={cancelEdit}>取消编辑</button>
{/snippet}

<section class:expanded={editing && !collapsed} aria-label="人工编辑">
  <button class="btn" type="button" bind:this={trigger} aria-label="编辑元数据" aria-expanded={editing && !collapsed} disabled={busy} title={editing && !collapsed ? '收起（保留当前填写内容）' : drafts.has(draftKey) ? '继续编辑本片草稿' : '展开'} onclick={() => { if (editing) collapsed = !collapsed; else { collapsed = false; start() } } }>
    <Icon name="pencil" size={15}/>编辑元数据
    {#if (!editing || collapsed) && drafts.has(draftKey)}<small class="draft-label">草稿</small>{/if}
    <Icon name="chevron-down" size={16}/>
  </button>
 <div class="tool-content" hidden={!(!collapsed && (editing || error || result?.error))}>
 {#if editing && !collapsed}
    {#if preview}
      <div aria-label="修订差异预览">
        <h3 class="editor-heading" bind:this={previewHeading} tabindex="-1">核对本次修改</h3>
        <p class="muted">确认保存后，修改过的元数据字段将锁定。</p>
        {#if syncVersion}<p>重新写入海报和 NFO 的版本标记。</p>{/if}
        {#each preview.changes as change}
          <div class="change">
            <strong>{change.field === 'artwork_phase' ? '版本标记 · 人工' : `${EDITABLE_FIELDS.find(f => f.key === change.field)?.label ?? change.field} · 人工 · 保存后锁定`}</strong>
            {#if change.field === 'actors'}
              <div class="actors-diff">
                <div><h4>修改前</h4>{@render actorList(change.before as Actor[])}</div>
                <div><h4>修改后</h4>{@render actorList(change.after as Actor[], preview.token)}</div>
              </div>
            {:else}
              <del>{(change.field === 'artwork_phase' ? phaseText(change.before) : text(change.before)) || '（空）'}</del>
              <ins>{(change.field === 'artwork_phase' ? phaseText(change.after) : text(change.after)) || '（空）'}</ins>
            {/if}
          </div>
        {/each}
        {#each preview.warnings ?? [] as warning}<p role="note">{warning}</p>{/each}
        <div class="tool-actions">
          <button class="btn primary" onclick={save} disabled={busy}>确认保存</button>
          <button class="btn" onclick={returnToEdit} disabled={busy}>返回修改</button>
          {@render cancelButton()}
        </div>
      </div>
    {:else}
      <form bind:this={form} onfocusin={rememberField} onsubmit={(event) => { event.preventDefault(); inspect() }}>
        <h3 class="editor-heading">编辑元数据</h3>
        <p class="muted">草稿保留到退出软件。确认保存后，修改过的元数据字段将锁定。</p>
        <fieldset disabled={busy}>
          <legend>影片资料</legend>
          <div class="tool-fields">
            {#each textFields as field}
              {@const list = field.key === 'tags' || field.key === 'directors'}
              <label class="tool-field" class:wide={['title', 'plot', 'series'].includes(field.key)}>
                <span>{field.label}{#if field.key === 'runtime_min'}（分钟）{/if}</span>
                {#if list}<small>一行一个名称</small>{/if}
                <textarea class="input" aria-label={`编辑${field.label}`} bind:value={values[field.key]} rows={field.key === 'plot' ? 4 : list ? 3 : 1}></textarea>
              </label>
            {/each}
          </div>
        </fieldset>
        <fieldset disabled={busy}>
          <legend>演员</legend>
          {#if actors.length}<p class="muted">新头像会在预览差异时加载。</p>{/if}
          {#each actors as actor, index}
            <div class="actor-row">
              <ActorAvatar code={meta.code} url={savedThumbs.has(actor.thumb) ? actor.thumb : null} {libraryRoot} size={34} placeholder />
              <label class="tool-field">姓名<input class="input actor-name" aria-label={`演员姓名 ${index + 1}`} bind:value={actor.name} /></label>
              <label class="tool-field">性别<select class="input" aria-label={`演员性别 ${index + 1}`} bind:value={actor.gender}><option value="unknown">未知</option><option value="female">女</option><option value="male">男</option></select></label>
              <button type="button" class="btn sm" aria-label={`删除演员 ${index + 1}`} onclick={() => actors = actors.filter((_, i) => i !== index)}>删除</button>
              <label class="tool-field thumb-field">头像网址（可选）<input class="input" aria-label={`演员头像网址 ${index + 1}`} inputmode="url" spellcheck="false" placeholder="https://…" bind:value={actor.thumb} /></label>
            </div>
          {/each}
          <button type="button" class="btn sm" onclick={addActor}>添加演员</button>
        </fieldset>
        <fieldset disabled={busy} aria-label="版本标记">
          <legend>版本标记</legend>
          <p class="muted">版本标记会写入 NFO，并按「封面角标」设置更新海报文件。</p>
          <div class="phase-options">
            {#each phaseOptions as option}
              <label><input class="checkbox" type="checkbox" aria-label={option.label} bind:checked={phase[option.key]} />{option.label}</label>
            {/each}
          </div>
          <label class="tool-field definition">清晰度<select class="input" aria-label="版本清晰度" bind:value={phase.definition}><option value={null}>无标记</option>{#each ['8K','4K','1440p','1080p','720p','480p','HD','SD'] as definition}<option value={definition}>{definition}</option>{/each}</select></label>
          <label class="sync-version"><input class="checkbox" type="checkbox" aria-label="重新写入已有版本标记" bind:checked={syncVersion} />重新写入已有版本标记</label>
        </fieldset>
        <div class="edit-footer">
          <span role="status" aria-live="polite" class:changed={changedCount > 0}>{changedCount ? `已改 ${changedCount} 项` : '尚未修改'}</span>
          <div class="tool-actions">
            <button class="btn primary" type="submit" disabled={busy || !changedCount} aria-keyshortcuts="Control+Enter Meta+Enter" title={`预览差异（${shortcut}）`}>{#if busy}<Icon name="loader" size={14} />{/if}预览差异</button>
            {@render cancelButton()}
          </div>
        </div>
      </form>
    {/if}
  {/if}
  {#if error && !collapsed}<p role="alert">{error}</p>{/if}
  {#if result?.error && !collapsed}<p>修订未完整完成，请从历史核对或撤销。日志：{result.journalId}</p>{/if}
</div>
</section>
<style>
  .editor-heading { margin: 0 0 var(--s1); font-size: var(--fs-md); }
  .editor-heading + p { margin: 0 0 var(--s3); font-size: var(--fs-sm); }
  .draft-label { color: var(--accent); font-size: var(--fs-xs); }
  .edit-footer { position: sticky; z-index: 1; bottom: 0; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--s2); margin-top: var(--s3); padding-block: var(--s2); border-top: 1px solid var(--line); background: var(--surface-2); }
  .edit-footer > span { color: var(--text-3); font-size: var(--fs-sm); white-space: nowrap; }
  .edit-footer > .changed { color: var(--accent); }
  .edit-footer > .tool-actions { flex: 1; min-width: min(100%, 160px); margin: 0; }
  form :is(input, textarea, select) { scroll-margin-block: var(--s3) 70px; }
  fieldset { min-width: 0; border: 1px solid var(--line); border-radius: var(--r-sm); padding: var(--s3); margin: var(--s4) 0 0; }
  legend { padding-inline: var(--s2); font-weight: 600; }
  fieldset > p { font-size: var(--fs-sm); }
  .phase-options { display: flex; flex-wrap: wrap; gap: var(--s2) var(--s4); margin: var(--s3) 0; }
  .phase-options label { display: flex; align-items: center; gap: var(--s2); }
  .definition { max-width: 160px; }
  .sync-version { display: flex; align-items: center; gap: var(--s2); margin-top: var(--s3); }
  .actor-row { display: grid; grid-template-columns: auto minmax(0, 1fr) minmax(64px, auto) auto; align-items: end; gap: var(--s2); padding-block: var(--s3); border-bottom: 1px solid var(--line); }
  .actor-row + button { margin-top: var(--s3); }
  .actor-row button { min-height: 34px; padding-inline: var(--s2); }
  .thumb-field { grid-column: 2 / -1; }
  .actors-diff { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 180px), 1fr)); gap: var(--s3); }
  .actors-diff h4 { margin: 0 0 var(--s2); font-size: var(--fs-sm); }
  .actor-list { display: grid; gap: var(--s2); list-style: none; padding: 0; margin: 0; }
  .actor-list li { display: flex; align-items: center; gap: var(--s2); }
  .actor-list small { display: block; }
  del { color: var(--text-3); white-space: pre-wrap; }
  ins { text-decoration: none; color: var(--accent); white-space: pre-wrap; }
  small { color: var(--text-3); }
  .change { display: flex; flex-direction: column; gap: 6px; margin: 16px 0; overflow-wrap: anywhere; }
</style>
