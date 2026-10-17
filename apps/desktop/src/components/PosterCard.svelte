<script lang="ts">
  import { api } from '../lib/api'
  import type { MovieCard } from '../lib/types'
  import Icon from './Icon.svelte'

  /** delay < 0 表示不播放入场动画(滚动中新出现的卡片不应再动) */
  let { card, delay = -1, onopen, onmenu }: { card: MovieCard | null; delay?: number; onopen?: (code: string) => void; onmenu?: (card: MovieCard, x: number, y: number) => void } = $props()

  // 键盘(Shift+F10 / 菜单键)触发时没有鼠标坐标,改用卡片位置
  function contextmenu(e: MouseEvent) {
    if (!card || !onmenu) return
    e.preventDefault()
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const keyboard = e.clientX === 0 && e.clientY === 0
    onmenu(card, keyboard ? r.left + r.width / 2 : e.clientX, keyboard ? r.top + r.height / 2 : e.clientY)
  }

  let loaded = $state(false)
  let failed = $state(false)
  const src = $derived(card ? api.posterUrl(card.poster, card.posterVersion) : null)

  // 虚拟列表复用组件时,海报变了要重置加载状态
  $effect(() => {
    src
    loaded = false
    failed = false
  })
</script>

{#if card}
  <button
    class="pcard"
    class:enter={delay >= 0}
    style="--delay:{Math.max(delay, 0)}ms"
    aria-label="{card.code} {card.title ?? ''}"
    title="查看 {card.code} 的影片详情"
    onclick={() => onopen?.(card.code)}
    oncontextmenu={contextmenu}
  >
    <div class="poster">
      {#if src && !failed}
        {#key src}
        <img class:ready={loaded} {src} alt="" loading="lazy" decoding="async" draggable="false" onload={() => (loaded = true)} onerror={() => (failed = true)} />
        {/key}
      {/if}
      <div class="ph" class:ready={loaded} class:skeleton-fill={!!src && !failed && !loaded} aria-hidden="true"><Icon name="image" size={26} /><span>{failed ? '海报未能加载' : !src ? '暂无海报' : card.code}</span></div>
      <div class="shade"></div>
      {#if card.score !== null && card.score >= 85}
        <span class="badge score tnum"><Icon name="star" size={10} />{(card.score / 10).toFixed(1)}</span>
      {/if}
      {#if card.parts > 1}<span class="badge parts tnum">{card.parts} 个分片</span>{/if}
      {#if card.favorite}<span class="badge fav" role="img" aria-label="已收藏" title="已收藏"><Icon name="star" size={11} /></span>{/if}
      <span class="play" aria-hidden="true"><Icon name="arrow-right" size={20} /></span>
    </div>
    <div class="meta">
      <div class="title">{card.title ?? card.code}</div>
      <div class="sub"><span class="mono">{card.code}</span>{#if card.year}<span class="faint"> · {card.year}</span>{/if}</div>
    </div>
  </button>
{:else}
  <div class="pcard skel" aria-hidden="true">
    <div class="poster"></div>
    <div class="meta"><div class="line"></div><div class="line short"></div></div>
  </div>
{/if}

<style>
  .pcard {
    display: flex; flex-direction: column; gap: var(--s2); width: 100%; min-width: 0;
    padding: 0; border: 0; background: transparent; text-align: left; color: inherit;
  }
  .enter { animation: enter 460ms var(--ease-out) both; animation-delay: var(--delay); }
  @keyframes enter { from { opacity: 0; transform: translateY(14px) scale(0.97); } }
  @media (prefers-reduced-motion: reduce) { .enter { animation: none; } }

  .poster {
    position: relative; width: 100%; aspect-ratio: var(--poster-ratio); overflow: hidden; border-radius: var(--r-md);
    background: var(--surface-2);
    transition: transform var(--d-base) var(--ease-out), box-shadow var(--d-base) var(--ease-out);
    will-change: transform;
  }
  .pcard:hover .poster, .pcard:focus-visible .poster { transform: translateY(-5px) scale(1.03); box-shadow: var(--shadow-3); }
  .pcard:active .poster { transform: translateY(-1px) scale(0.99); }
  .pcard:focus-visible { outline: none; }
  .pcard:focus-visible .poster { box-shadow: 0 0 0 2px var(--accent), var(--shadow-3); }

  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0; transition: opacity var(--d-slow) var(--ease-out); }
  img.ready { opacity: 1; }
  .ph {
    position: absolute; inset: 0; display: flex; flex-direction:column; align-items:center; justify-content:center; gap:var(--s3); padding: var(--s3); pointer-events:none;
    background: linear-gradient(150deg, var(--surface-3), var(--surface-1)); color: var(--text-3);
    font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: 600;
  }
  .ph { transition:opacity var(--d-slow) var(--ease-out); }
  .ph.ready { opacity:0; }
  .ph.skeleton-fill { background:linear-gradient(100deg,var(--surface-2) 30%,var(--surface-3) 50%,var(--surface-2) 70%); background-size:200% 100%; }
  .shade { position: absolute; inset: 0; background: linear-gradient(to top, rgb(0 0 0 / 0.5), transparent 38%); opacity: 0; transition: opacity var(--d-base) var(--ease-out); pointer-events: none; }
  .pcard:hover .shade, .pcard:focus-visible .shade { opacity: 1; }

  .badge {
    position: absolute; display: inline-flex; align-items: center; gap: 3px; height: 20px; padding: 0 7px;
    border-radius: var(--r-pill); font-size: 10.5px; font-weight: 600; color: #fff;
    background: rgb(12 12 16 / 0.62); backdrop-filter: blur(8px);
  }
  .score { top: var(--s2); right: var(--s2); color: var(--accent-hover); }
  .parts { left: var(--s2); top: var(--s2); }
  .play {
    position: absolute; right: var(--s3); bottom: var(--s3); width: 38px; height: 38px; border-radius: 50%;
    display: grid; place-items: center; background: var(--accent); color: var(--accent-ink);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.4); opacity: 0; transform: scale(0.7);
    transition: opacity var(--d-base) var(--ease-out), transform var(--d-base) var(--ease-spring);
  }
  .pcard:hover .play, .pcard:focus-visible .play { opacity: 1; transform: scale(1); }

  .meta { padding: 0 2px; height: 48px; overflow: hidden; }
  .meta { transition: transform var(--d-base) var(--ease-out); }
  .pcard:hover .meta { transform: translateY(2px); }
  .title { transition: color var(--d-fast); }
  .pcard:hover .title { color: var(--accent-hover); }
  .title { font-size: var(--fs-sm); font-weight: 600; line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 1; line-clamp: 1; -webkit-box-orient: vertical; overflow: hidden; }
  .sub { font-size: var(--fs-xs); color: var(--text-2); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .skel .poster, .skel .line {
    background: linear-gradient(100deg, var(--surface-2) 30%, var(--surface-3) 50%, var(--surface-2) 70%); background-size: 200% 100%;
    animation: shimmer 1.6s linear infinite; box-shadow: none;
  }
  .line { height: 11px; border-radius: 6px; margin: 6px 0 0; width: 80%; }
  .line.short { width: 45%; }
  .badge.fav { left: auto; right: 8px; top: 8px; bottom: auto; color: var(--warn); }
</style>
