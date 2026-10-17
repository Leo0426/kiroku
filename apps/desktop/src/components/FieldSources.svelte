<script lang="ts">
  import { FIELDS, SOURCE_NAMES, type Metadata } from '../lib/types'
  import { api } from '../lib/api'
  let { meta }: { meta: Metadata } = $props()
  let error = $state('')
  function valid(url: string) { try { return ['http:', 'https:'].includes(new URL(url).protocol) } catch { return false } }
  function source(value?: string) { return value ? value.split(',').map(s => s.trim() === 'manual' ? '人工' : SOURCE_NAMES[s.trim()] ?? s.trim()).join('、') : '未知' }
  async function open(url: string) { try { await api.openSourceUrl(url) } catch (e) { error = String(e) } }
</script>
<details>
  <summary>字段来源与原站链接</summary>
  <dl>{#each FIELDS.filter(field => meta.provenance?.[field.key] || meta.locked_fields?.includes(field.key)) as field}<dt>{field.label}</dt><dd>{source(meta.provenance?.[field.key])}{#if meta.locked_fields?.includes(field.key)} · 已锁定{/if}</dd>{/each}</dl>
  <div class="source-links">
  {#each Object.entries(meta.source_urls) as [site, url]}
    {#if valid(url)}<button class="btn sm" title={url} onclick={() => open(url)}>{SOURCE_NAMES[site] ?? site} 原站</button>{:else}<p class="muted">{site}：无效来源链接</p>{/if}
  {/each}
  </div>
  {#if error}<p role="alert">{error}</p>{/if}
</details>
<style>
 details { padding:var(--s3);border:1px solid var(--line);border-radius:var(--r-md); }
 summary { cursor:pointer;color:var(--text-2);font-size:var(--fs-sm); }
 dl { display:grid;grid-template-columns:minmax(80px, 120px) minmax(0,1fr);gap:var(--s2);font-size:var(--fs-sm);margin:var(--s3) 0; }
 dd { margin:0;overflow-wrap:anywhere; } dt { color:var(--text-3); }
 .source-links { display:flex;flex-wrap:wrap;gap:var(--s2); }
</style>
