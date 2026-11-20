<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { sourceCheckMessage } from '../lib/source-check-message'
  import type { Settings, SourceCheck } from '../lib/types'
  let { sourceId, label, settings }: { sourceId: string; label: string; settings: Settings } = $props()
  let busy = $state(false)
  let result = $state<SourceCheck | null>(null)
  let error = $state('')
  const config = $derived(JSON.stringify(settings))
  let generation = 0
  let alive = true
  onDestroy(() => { alive = false; generation++ })
  $effect(() => { config; generation++; result = null; error = '' })
  async function verify() {
    if (busy) return
    const version = generation
    const snapshot = config
    busy = true; result = null; error = ''
    try {
      // Plain draft: this request never saves settings or persistent credentials.
      const checks = await api.sourcesCheck([sourceId], JSON.parse(snapshot))
      if (version !== generation || snapshot !== config) return
      result = checks.find(check => check.sourceId === sourceId) ?? null
      if (!result) error = '未收到检测结果，请重试'
    } catch {
      if (version === generation && snapshot === config) error = '验证未完成，请检查配置或是否有任务正在运行'
    } finally {
      if (alive) busy = false
    }
  }
</script>
<div class="authentication">
  <button class="btn sm" type="button" aria-label={`验证 ${label} 认证`} disabled={busy} onclick={verify}>{busy ? '验证中…' : '验证认证'}</button>
  {#if result}<p role="status" aria-label={`${label} 认证结果`} class:confirmed={result.outcome.reason === 'authenticated'}>{sourceCheckMessage(result)}</p>
  {:else if error}<p role="alert">{error}</p>
  {:else}<span class="faint">使用当前填写的配置，无需先保存</span>{/if}
</div>
<style>
  .authentication { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); margin-top: var(--s3); }
  p, span { margin: 0; font-size: var(--fs-sm); overflow-wrap: anywhere; }
  .confirmed { color: var(--ok); }
</style>
