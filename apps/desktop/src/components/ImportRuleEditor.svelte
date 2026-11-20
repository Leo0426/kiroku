<script lang="ts">
  import { onDestroy } from 'svelte'
  import { flow } from '../lib/importflow.svelte'
  import type { ImportRules } from '../lib/types'
  import Icon from './Icon.svelte'

  let { disabled=false }: {disabled?:boolean}=$props()
  let draft = $state<ImportRules | null>(null)
  function edit() {
    if (!flow.effectiveRules) return
    draft = { ...flow.effectiveRules }
    flow.editingRules = true
  }
  function close() {
    draft = null
    flow.editingRules = false
  }
  async function apply(saveDefault = false) {
    if (!draft || flow.busy) return
    if (await flow.updateRules({ ...draft }, saveDefault)) close()
  }
  onDestroy(() => { flow.editingRules = false })
</script>

<div class="rule-editor">
  {#if draft}
    <form aria-label="本批整理规则" onsubmit={e => { e.preventDefault(); void apply() }}>
      <fieldset disabled={flow.busy || disabled}>
        <label>文件处理方式<select class="input" bind:value={draft.mode}>
          <option value="move">移动</option><option value="copy">复制，保留源文件</option>
          <option value="hardlink">创建硬链接</option><option value="symlink">创建软链接</option>
        </select></label>
        <label>库内子目录<input class="input" bind:value={draft.subdirectory} placeholder="留空使用媒体库根目录" /></label>
        <label>本批目录模板<input class="input mono" bind:value={draft.directoryTemplate} required /></label>
        <label>本批文件名模板<input class="input mono" bind:value={draft.fileTemplate} disabled={draft.plexNaming} /></label>
        <label class="inline"><input type="checkbox" class="checkbox" bind:checked={draft.plexNaming} />使用 Plex 命名</label>
      </fieldset>
      <div class="actions">
        <button class="btn primary" disabled={flow.busy || disabled}>{flow.step === 'plan' ? '应用并重新预览' : '应用到本批'}</button>
        <button type="button" class="btn" disabled={flow.busy || disabled} onclick={() => apply(true)}>设为导入默认</button>
        <button type="button" class="btn ghost" disabled={flow.busy || disabled} onclick={close}>取消</button>
      </div>
    </form>
  {:else}
    <button class="btn sm" aria-expanded="false" disabled={flow.busy || disabled} onclick={edit}><Icon name="sliders" size={14}/>调整本批规则</button>
  {/if}
</div>
<style>
  .rule-editor { margin-top:var(--s3); }
  form { display:grid; gap:var(--s3); }
  fieldset { border:0; padding:0; margin:0; display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,225px),1fr)); gap:var(--s3); }
  label { min-width:0; display:grid; gap:4px; color:var(--text-2); font-size:var(--fs-sm); }
  label.inline,.actions { display:flex; align-items:center; gap:var(--s2); flex-wrap:wrap; }
  input, select { width:100%; min-width:0; }
  .inline input { width:auto; }
</style>
