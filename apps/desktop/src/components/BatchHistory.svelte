<script lang="ts">
  import HistoryOperations from './HistoryOperations.svelte'
  import Icon from './Icon.svelte'
  import { app } from '../lib/state.svelte'

  let { journalId, root }: { journalId: string | null; root: string | null } = $props()
  let open = $state(false)
</script>

{#if !journalId || !root}
  <p class="muted">本批暂无可查看的操作记录。</p>
{:else if root !== app.settings?.library_root}
  <p class="muted">切回媒体库 <span class="mono">{root}</span> 可查看本批操作。</p>
{:else}
  <div class="batch-history">
    <button class="btn sm ghost" aria-expanded={open} onclick={() => open = !open}>
      <Icon name="history" size={15} />{open ? '收起本批操作' : '查看本批操作'}
    </button>
    {#if open}<HistoryOperations id={journalId} expectedRoot={root} />{/if}
  </div>
{/if}

<style>
  .batch-history { display: grid; gap: var(--s3); width: 100%; text-align: left; }
  .batch-history > button { justify-self: center; }
  p { font-size: var(--fs-sm); overflow-wrap: anywhere; }
</style>
