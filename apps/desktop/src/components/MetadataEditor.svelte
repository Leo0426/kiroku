<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { EDITABLE_FIELDS, type Actor, type Metadata, type RevisionPreview, type RevisionResult } from '../lib/types'
  let { meta, onsaved }: { meta: Metadata; onsaved: () => void } = $props()
  let editing = $state(false), busy = $state(false), error = $state('')
  let values = $state<Record<string, string>>({})
  let actors = $state<Actor[]>([])
  const genderLabel = { female: '女', male: '男', unknown: '未知' }
  let preview = $state<RevisionPreview | null>(null)
  let result = $state<RevisionResult | null>(null)
  let generation = 0
  onDestroy(() => { generation++ })
  $effect(() => { meta.code; app.settings?.library_root; generation++; editing = false; preview = null; error = ''; result = null; busy = false })
  function text(value: unknown): string {
    if (value == null) return ''
    if (Array.isArray(value)) return value.map(item => typeof item === 'string' ? item : `${item.name}（${genderLabel[item.gender as Actor['gender']] ?? '未知'}）`).join('\n')
    return String(value)
  }
  function start() {
    values = Object.fromEntries(EDITABLE_FIELDS.map(f => [f.key, text(meta[f.key as keyof Metadata])]))
    actors = meta.actors.map(actor => ({ ...actor }))
    editing = true; error = ''; preview = null; result = null
  }
  function value(field: string, input: string): unknown {
    if (field === 'tags' || field === 'directors') return input.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
    if (field === 'runtime_min') { if (!input.trim()) return null; const n = Number(input); if (!Number.isInteger(n) || n < 0 || n > 4294967295) throw '片长必须为非负整数'; return n }
    return input === '' ? null : input
  }
  async function inspect() {
    const current = generation
    busy = true; error = ''; result = null
    try {
      if (actors.some(actor => !actor.name.trim())) throw '请填写演员姓名，或删除空白演员行'
      const patch: Record<string, unknown> = Object.fromEntries(EDITABLE_FIELDS.filter(f => f.key !== 'actors' && values[f.key] !== text(meta[f.key as keyof Metadata])).map(f => [f.key, value(f.key, values[f.key])]))
      if (JSON.stringify(actors) !== JSON.stringify(meta.actors)) patch.actors = actors.map(actor => ({ ...actor }))
      if (!Object.keys(patch).length) throw '没有字段变化'
      const next = await api.revisionPreview(meta.code, patch)
      if (current === generation) preview = next
    } catch (e) { if (current === generation) error = String(e) }
    finally { if (current === generation) busy = false }
  }
  async function save() {
    if (!preview) return
    const current = generation, candidate = preview
    busy = true; error = ''
    try {
      const saved = await api.revisionSave(candidate.token, candidate.root, candidate.code)
      if (current !== generation) return
      result = saved; preview = null
      if (saved.error || saved.indexError) error = [saved.error, saved.indexError && `索引更新失败：${saved.indexError}`].filter(Boolean).join('；')
      else { editing = false; app.toast('ok', '人工修订已保存，可从历史撤销'); onsaved() }
    } catch (e) { if (current === generation) { error = String(e); preview = null } }
    finally { if (current === generation) busy = false }
  }
</script>
<section class:expanded={editing} aria-label="人工编辑">
  {#if !editing}<button class="btn" onclick={start}>编辑元数据</button>{:else}
    <h3>人工编辑</h3><p class="muted">仅修改发生变化的字段；保存后自动锁定，包括清空的值。视频文件名和位置保持不变。</p>
    {#if preview}
      <div aria-label="修订差异预览">
        {#each preview.changes as change}<div class="change"><strong>{EDITABLE_FIELDS.find(f => f.key === change.field)?.label ?? change.field} · 人工 · 保存后锁定</strong><del>{text(change.before) || '（空）'}</del><ins>{text(change.after) || '（空）'}</ins></div>{/each}
        <button class="btn primary" onclick={save} disabled={busy}>确认保存</button>
        <button class="btn" onclick={() => preview = null} disabled={busy}>返回修改</button>
      </div>
    {:else}
      <form onsubmit={(event) => { event.preventDefault(); inspect() }}>
        {#each EDITABLE_FIELDS as field}
          {#if field.key === 'actors'}
            <fieldset disabled={busy}>
              <legend>演员</legend>
              {#each actors as actor, index}
                <div class="actor-row">
                  <label>姓名<input aria-label={`演员姓名 ${index + 1}`} bind:value={actor.name} /></label>
                  <label>性别<select aria-label={`演员性别 ${index + 1}`} bind:value={actor.gender}><option value="unknown">未知</option><option value="female">女</option><option value="male">男</option></select></label>
                  <button type="button" class="btn sm" aria-label={`删除演员 ${index + 1}`} onclick={() => actors = actors.filter((_, i) => i !== index)}>删除</button>
                </div>
              {/each}
              <button type="button" class="btn sm" onclick={() => actors = [...actors, { name: '', gender: 'unknown' }]}>添加演员</button>
            </fieldset>
          {:else}
            <label>{field.label}{#if field.key === 'tags' || field.key === 'directors'}<small>一行一个名称；清空可删除全部。</small>{/if}<textarea aria-label={`编辑${field.label}`} bind:value={values[field.key]} rows={field.key === 'plot' ? 4 : 2} disabled={busy}></textarea></label>
          {/if}
        {/each}
        <button class="btn primary" type="submit" disabled={busy}>预览差异</button>
      </form>
    {/if}
    <button class="btn ghost" disabled={busy} onclick={() => { editing = false; preview = null }}>取消编辑</button>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if result?.error}<p>修订未完整完成，请从历史核对或撤销。日志：{result.journalId}</p>{/if}
</section>
<style>fieldset { border:1px solid var(--border); border-radius:var(--r-sm); padding:12px; } .actor-row { display:flex; align-items:center; gap:8px; } .actor-row label:first-child { flex:1; min-width:0; } .actor-row input { width:100%; } del { white-space:pre-wrap; } label { display:flex; flex-direction:column; gap:6px; margin:12px 0; } textarea { width:100%; resize:vertical; } small { color:var(--text-3); } .change { display:flex; flex-direction:column; gap:6px; margin:16px 0; overflow-wrap:anywhere; } del { color:var(--text-3); } ins { text-decoration:none; color:var(--accent); white-space:pre-wrap; }</style>
