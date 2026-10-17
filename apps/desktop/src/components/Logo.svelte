<script lang="ts">
  /**
   * 品牌标志,与应用图标同造型:深色圆角方块 + 带缺口的琥珀环 + 中心点。
   * 悬停时环转一圈;busy 时慢速持续旋转(有任务在跑)。
   */
  let { size = 30, busy = false }: { size?: number; busy?: boolean } = $props()
  const uid = Math.random().toString(36).slice(2, 8)
</script>

<span class="logo" class:busy style="--s:{size}px" aria-hidden="true">
  <svg viewBox="0 0 64 64" width={size} height={size}>
    <defs>
      <linearGradient id="bg-{uid}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a2630" /><stop offset="1" stop-color="#0e0d11" /></linearGradient>
      <linearGradient id="am-{uid}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#f6c983" /><stop offset="1" stop-color="#df9233" /></linearGradient>
    </defs>
    <rect x="4" y="4" width="56" height="56" rx="12.6" fill="url(#bg-{uid})" />
    <g class="ring">
      <circle cx="32" cy="32" r="16.2" fill="none" stroke="url(#am-{uid})" stroke-width="6.25" stroke-dasharray="92.2 9.6" transform="rotate(-28 32 32)" />
    </g>
    <circle class="dot" cx="32" cy="32" r="3.5" fill="url(#am-{uid})" />
  </svg>
</span>

<style>
  .logo { display: inline-grid; place-items: center; width: var(--s); height: var(--s); flex-shrink: 0; }
  svg { display: block; overflow: visible; transition: transform var(--d-base) var(--ease-spring); }
  .ring { transform-origin: 32px 32px; transition: transform 900ms var(--ease-spring); }
  .dot { transform-origin: 32px 32px; transition: transform var(--d-base) var(--ease-spring); }
  /* 悬停:环转一圈、中心点轻轻放大 */
  :global(:hover) > .logo .ring, .logo:hover .ring { transform: rotate(360deg); }
  :global(:hover) > .logo .dot, .logo:hover .dot { transform: scale(1.25); }
  .logo:active svg { transform: scale(0.92); }
  .busy .ring { animation: spin 2.4s linear infinite; }
  @media (prefers-reduced-motion: reduce) {
    .ring, .dot { transition: none; }
    .busy .ring { animation: none; }
  }
</style>
