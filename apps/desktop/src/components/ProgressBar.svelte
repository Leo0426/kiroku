<script lang="ts">
  /** value 为 0–1，省略时表示进度未知。 */
  let { value, active = false, label }: { value?: number; active?: boolean; label: string } = $props()
  const pct = $derived(value === undefined ? undefined : Math.max(0, Math.min(1, value)) * 100)
</script>

<div class="track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct === undefined ? undefined : Math.round(pct)} aria-label={label}>
  <div class="bar" class:active class:indeterminate={pct === undefined} style:width={pct === undefined ? undefined : `${pct}%`}></div>
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
  .bar.indeterminate { width: 36%; animation: progress-slide 1.5s ease-in-out infinite; }
  @keyframes progress-slide { from { transform: translateX(-100%); } to { transform: translateX(278%); } }
  @media (prefers-reduced-motion: reduce) {
    .bar { transition: none; }
    .bar.active, .bar.indeterminate { animation: none; }
    .bar.indeterminate { margin-inline: auto; }
  }
</style>
