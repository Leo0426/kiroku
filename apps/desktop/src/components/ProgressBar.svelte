<script lang="ts">
  /** value 为 0–1;active 时叠一层流光,表示任务仍在进行 */
  let { value = 0, active = false, label }: { value?: number; active?: boolean; label?: string } = $props()
  const pct = $derived(Math.max(0, Math.min(1, value)) * 100)
</script>

<div class="track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(pct)} aria-label={label}>
  <div class="bar" class:active style="width: {pct}%"></div>
</div>

<style>
  .track { height: 8px; border-radius: var(--r-pill); background: var(--surface-3); overflow: hidden; }
  .bar {
    height: 100%; border-radius: inherit; background: var(--accent);
    transition: width var(--d-slow) var(--ease-out);
  }
  .bar.active {
    background-image: linear-gradient(100deg, transparent 20%, rgb(255 255 255 / 0.35) 50%, transparent 80%);
    background-size: 200% 100%; background-color: var(--accent);
    animation: shimmer 1.6s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) { .bar.active { animation: none; } }
</style>
