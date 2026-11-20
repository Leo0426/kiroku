<script lang="ts">
  import type { Snippet } from 'svelte'
  import type { ScanItem } from '../lib/types'
  import Icon from './Icon.svelte'

  let { items, row }: { items: ScanItem[]; row: Snippet<[ScanItem]> } = $props()
  const pageSize = 50
  let query = $state('')
  let requestedPage = $state(0)
  const needle = $derived(query.trim().toLowerCase())
  const matches = $derived(needle
    ? items.filter(item => item.path.toLowerCase().includes(needle) || item.code?.id.toLowerCase().includes(needle))
    : items)
  const pages = $derived(Math.ceil(matches.length / pageSize))
  const page = $derived(Math.max(0, Math.min(requestedPage, pages - 1)))
  const visible = $derived(matches.slice(page * pageSize, (page + 1) * pageSize))
  let list: HTMLDivElement

  function update(search: string, index: number) {
    query = search
    requestedPage = index
    list.scrollTop = 0
  }
</script>

{#if items.length > 10 || query}
  <div class="scan-search">
    <input class="input" type="search" aria-label="查找扫描文件" placeholder="查找文件名、路径或番号" value={query} oninput={event => update(event.currentTarget.value, 0)} />
    {#if needle}<span class="muted tnum" role="status">匹配 {matches.length} 个</span>{/if}
  </div>
{/if}
<div class="rows" bind:this={list}>
  {#each visible as item (item.id)}{@render row(item)}{/each}
  {#if !matches.length && needle}
    <div class="no-match"><span>没有匹配的文件</span><button class="btn sm" onclick={() => update('', 0)}>清除搜索</button></div>
  {/if}
</div>
{#if pages > 1}
  <div class="pager">
    <span class="muted tnum" aria-live="polite">{page * pageSize + 1}–{page * pageSize + visible.length} / {matches.length} 个文件</span>
    <button class="btn sm" aria-label="上一页文件" disabled={page === 0} onclick={() => update(query, page - 1)}><Icon name="arrow-left" size={14} /></button>
    <button class="btn sm" aria-label="下一页文件" disabled={page >= pages - 1} onclick={() => update(query, page + 1)}><Icon name="arrow-right" size={14} /></button>
  </div>
{/if}

<style>
  .scan-search, .pager, .no-match { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; padding: var(--s3) var(--s4); font-size: var(--fs-sm); }
  .scan-search { border-top: 1px solid var(--line); }
  .scan-search input { flex: 1; max-width: 340px; }
  .rows { padding: var(--s1); max-height: 52vh; overflow-y: auto; }
  .pager { border-top: 1px solid var(--line); }
  .pager span { margin-right: auto; }
  .no-match { justify-content: center; color: var(--text-2); }
</style>
