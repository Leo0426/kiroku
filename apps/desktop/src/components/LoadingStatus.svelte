<script lang="ts">
  import Icon from './Icon.svelte'
  let { label }: { label: string } = $props()
</script>

<p class="loading-status" role="status"><Icon name="loader" size={14} />{label}</p>

<style>
  .loading-status { display:flex; align-items:center; gap:var(--s2); color:var(--text-2); font-size:var(--fs-sm); min-height:28px; }
</style>
