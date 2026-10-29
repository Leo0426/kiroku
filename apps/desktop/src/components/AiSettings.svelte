<script lang="ts">
  import { onDestroy } from 'svelte'
  import type { AiCfg, AiProtocol } from '../lib/types'
  import { newAiProvider, selectedAiProvider, validateAiProvider } from '../lib/ai'
  import { api } from '../lib/api'
  import Icon from './Icon.svelte'
  import InfoTip from './InfoTip.svelte'

  let { cfg = $bindable() }: { cfg: AiCfg } = $props()
  let kind = $state<AiProtocol | 'custom'>('custom')
  const provider = $derived(selectedAiProvider(cfg))
  let showKey = $state(false)
  let testing = $state(false)
  let result = $state('')
  let error = $state('')
  let revision = 0
  let lastDraft = ''

  function invalidate() {
    revision++
    testing = false
    result = ''
    error = ''
  }
  $effect(() => {
    const draft = JSON.stringify(cfg)
    if (draft !== lastDraft) { lastDraft = draft; invalidate() }
  })
  $effect(() => { cfg.selected_provider; showKey = false })
  onDestroy(invalidate)

  function add() {
    const p = newAiProvider(kind)
    cfg.providers.push(p)
    cfg.selected_provider = p.id
  }
  function remove() {
    cfg.providers = cfg.providers.filter(p => p.id !== cfg.selected_provider)
    cfg.selected_provider = null
  }
  async function trial() {
    invalidate()
    const token = revision
    try {
      validateAiProvider(provider)
      testing = true
      // Trial always translates the sample title, regardless of the batch field switches.
      const draft = $state.snapshot(cfg)
      draft.title = true
      draft.plot = false
      const translated = await api.aiTest(draft)
      if (token === revision) result = translated.title ?? '(没有返回译文)'
    } catch (e) {
      if (token === revision) error = String(e)
    } finally {
      if (token === revision) testing = false
    }
  }
</script>

<div class="ai-settings">
  <div class="tipped"><label class="switch"><input class="checkbox" type="checkbox" bind:checked={cfg.enabled} /><span>启用 AI 汉化</span></label><InfoTip label="AI 汉化说明"><p>通过 OpenAI 或 Anthropic 兼容 API 翻译标题与简介。每个 Provider 可单独设置地址、密钥和模型，费用由服务商收取。</p></InfoTip></div>
  <div class="row">
    <label class="label" for="ai-provider">当前 Provider</label>
    <select id="ai-provider" class="input grow" value={cfg.selected_provider ?? ''} onchange={e => { invalidate(); cfg.selected_provider = e.currentTarget.value || null }}>
      <option value="">请选择 Provider</option>
      {#each cfg.providers as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
    <button type="button" class="btn sm ghost" disabled={!provider} onclick={remove}>删除 Provider</button>
  </div>
  <div class="row add-row">
    <select class="input grow" aria-label="新 Provider 类型" bind:value={kind}>
      <option value="custom">自定义兼容服务</option>
      <option value="openai">OpenAI 预设</option>
      <option value="anthropic">Anthropic 预设</option>
    </select>
    <button type="button" class="btn sm" onclick={add}>添加 Provider</button>
  </div>
  {#if provider}
    <div class="fields" oninput={invalidate} onchange={invalidate}>
      <div class="row"><label class="label" for="ai-name">Provider 名称</label><input id="ai-name" class="input grow" bind:value={provider.name} /></div>
      <div class="row">
        <label class="label" for="ai-protocol">API 协议</label>
        <select id="ai-protocol" class="input grow" bind:value={provider.protocol}>
          <option value="openai">OpenAI · Chat Completions</option><option value="anthropic">Anthropic · Messages</option>
        </select>
      </div>
      <div class="row"><label class="label" for="ai-url">API 地址</label><input id="ai-url" class="input grow mono" type="url" placeholder="https://api.example.com/v1" spellcheck="false" bind:value={provider.base_url} /></div>
      <div class="row">
        <label class="label" for="ai-key">API Key</label>
        <input id="ai-key" class="input grow mono" type={showKey ? 'text' : 'password'} autocomplete="off" spellcheck="false" bind:value={provider.api_key} />
        <button class="btn sm ghost" type="button" onclick={() => showKey = !showKey}>{showKey ? '隐藏' : '显示'}</button>
      </div>
      <div class="row"><label class="label" for="ai-model">模型</label><input id="ai-model" class="input grow mono" spellcheck="false" bind:value={provider.model} /></div>
      <details>
        <summary>兼容性与思考选项<InfoTip label="兼容性与思考选项说明"><p>若服务商不支持结构化输出或思考参数，可使用兼容模式和模型默认。fallback 仅在服务商明确支持时开启。</p></InfoTip></summary>
        <div class="row"><label class="label" for="ai-format">输出格式</label>
          <select id="ai-format" class="input grow" bind:value={provider.output_format}>
            <option value="json_schema">严格 JSON Schema</option><option value="prompt">兼容模式（提示词 JSON）</option>
          </select>
        </div>
        <div class="row"><label class="label" for="ai-effort">思考强度</label>
          <select id="ai-effort" class="input grow" bind:value={provider.effort}>
            <option value="">模型默认（不发送参数）</option><option value="low">低（low）</option><option value="medium">中（medium）</option><option value="high">高（high）</option>
          </select>
        </div>
        {#if provider.protocol === 'anthropic'}
          <label class="switch"><input class="checkbox" type="checkbox" bind:checked={provider.anthropic_fallback} /><span>启用 Anthropic 服务端 fallback</span></label>
        {/if}
      </details>
    </div>
    <div class="row">
      <button class="btn sm" type="button" onclick={trial} disabled={testing}><Icon name={testing ? 'loader' : 'check'} size={14} />试译</button>
      <InfoTip label="试译说明"><p>使用当前未保存配置试译一句，可能产生 API 费用。</p></InfoTip>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if result}<p class="success" role="status" aria-label="AI 试译结果">试译：{result}</p>{/if}
  {:else}
    <p class="hint">请选择用于汉化的 AI Provider</p>
  {/if}
  <div class="row">
    <span class="label">翻译哪些</span>
    <label class="switch"><input class="checkbox" type="checkbox" bind:checked={cfg.title} /><span>标题</span></label>
    <label class="switch"><input class="checkbox" type="checkbox" bind:checked={cfg.plot} /><span>简介</span></label>
  </div>
</div>

<style>
  .ai-settings { min-width: 0; }
  .hint { color: var(--text-2); font-size: var(--fs-sm); margin: var(--s2) 0; }
  .row { display: flex; align-items: center; flex-wrap: wrap; gap: var(--s3); margin: var(--s3) 0; }
  .tipped { display: flex; align-items: center; gap: var(--s1); }
  summary :global(.tip) { margin-left: var(--s1); }
  .label { flex: 0 0 110px; color: var(--text-2); font-size: var(--fs-sm); }
  .grow { flex: 1; min-width: 140px; }
  .add-row { padding-bottom: var(--s3); border-bottom: 1px solid var(--line); }
  .switch { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); }
  details { margin-top: var(--s3); }
  summary { cursor: pointer; color: var(--text-2); font-size: var(--fs-sm); }
  .error { color: var(--danger); overflow-wrap: anywhere; }
  .success { color: var(--ok); overflow-wrap: anywhere; }
  @media (max-width: 760px) { .fields .label { flex-basis: 100%; } }
</style>
