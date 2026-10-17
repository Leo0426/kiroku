<script lang="ts">
  import LoadingStatus from './LoadingStatus.svelte'
  let { label }: { label: string } = $props()
</script>

<div class="loading-rows">
  <LoadingStatus {label} />
  <div class="rows" aria-hidden="true">
    {#each [0, 1, 2, 3] as row (row)}
      <div class="row card"><div class="skeleton-fill long"></div><div class="skeleton-fill short"></div></div>
    {/each}
  </div>
</div>

<style>
  .rows { display:grid; gap:var(--s3); margin-top:var(--s3); }
  .row { display:flex; flex-direction:column; justify-content:center; gap:var(--s3); height:84px; padding:var(--s4) var(--s5); }
  .long, .short { height:12px; border-radius:var(--r-pill); width:58%; }
  .short { width:32%; }
</style>
