<script lang="ts">
  import Logo from './Logo.svelte'
  import ProgressBar from './ProgressBar.svelte'

  let { title, label, branded = false }: { title: string; label: string; branded?: boolean } = $props()
</script>

<div class="screen">
  <div class="panel">
    {#if branded}<Logo size={68} busy />{/if}
    <h1>{title}</h1>
    <p role="status">{label}</p>
    <div class="track"><ProgressBar {label} /></div>
  </div>
</div>

<style>
  .screen { display: grid; place-items: center; width: 100%; min-height: 240px; flex: 1; padding: var(--s6); }
  .panel { display: grid; justify-items: center; gap: var(--s3); width: min(100%, 360px); text-align: center; }
  h1 { font-size: 28px; letter-spacing: 0.02em; }
  p { color: var(--text-2); }
  .track { width: min(100%, 220px); margin-top: var(--s2); }
</style>
