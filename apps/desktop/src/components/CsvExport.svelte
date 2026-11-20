<script lang="ts">
  import ExportScope from './ExportScope.svelte'
  import type { ExportScope as Scope } from '../lib/export-scope'
  import { csvLabels, standardColumns } from '../lib/csv-columns'
  import type { CsvColumn, CsvOptions, Query } from '../lib/types'

  let { query, busy, onexport, onstop }: {
    query: Query; busy:boolean; onexport:(options:CsvOptions, query:Query)=>Promise<void>; onstop:()=>void
  }=$props()
  let open=$state(false)
  let scope=$state<Scope>({kind:'filtered'})
  let columns=$state<CsvColumn[]>([...standardColumns])
  const allColumns=Object.keys(csvLabels) as CsvColumn[]

  async function save() {
    const chosen=allColumns.filter(key=>columns.includes(key))
    if (busy || !chosen.length) return
    // Selected films can span filters. Preserve sort order while looking them up in the full index.
    const q=scope.kind==='filtered' ? $state.snapshot(query) : {sort:query.sort,reverse:query.reverse}
    await onexport({columns:chosen,...(scope.kind==='selection'?{codes:[...scope.codes]}:{})},q)
  }
</script>

<section aria-label="自定义 CSV">
  <button class="btn" aria-expanded={open} onclick={()=>open=!open}>自定义 CSV</button>
  {#if open}
    <div class="body">
      <ExportScope {query} bind:scope disabled={busy} labelPrefix="CSV "/>
      <fieldset disabled={busy}><legend>导出字段</legend><div class="choices">
        {#each allColumns as key}<label><input class="checkbox" type="checkbox" value={key} bind:group={columns}/>{csvLabels[key]}</label>{/each}
      </div></fieldset>
      <div class="actions">
        <button class="btn primary" disabled={busy || !columns.length || (scope.kind==='selection' && !scope.codes.length)} onclick={save}>保存 CSV 清单</button>
        {#if busy}<button class="btn" onclick={onstop}>停止导出清单</button>{/if}
      </div>
    </div>
  {/if}
</section>

<style>
.body { display:grid; gap:var(--s3); margin-top:var(--s3); }
fieldset { border:0; margin:0; padding:0; }
legend { font-weight:600; margin-bottom:var(--s2); }
.choices,.actions { display:flex; flex-wrap:wrap; gap:var(--s2) var(--s3); }
label { display:flex; align-items:center; gap:var(--s1); }
</style>
