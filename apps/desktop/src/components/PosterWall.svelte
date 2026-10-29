<script lang="ts">
  import { onMount, tick, untrack, type Snippet } from 'svelte'
  import { appearance } from '../lib/appearance.svelte'
  import { posterRequest } from '../lib/poster-requests'
  import { PosterPages, POSTER_PAGE_LIMIT, POSTER_REQUEST_LIMIT, type PosterPage } from '../lib/poster-pages'
  import type { MovieCard, Query } from '../lib/types'
  import type { Layout, Density } from '../lib/library-browser'
  import PosterCard from './PosterCard.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  /**
   * 虚拟化海报墙:只渲染可视行(+上下缓冲),其余用一个等高的占位撑出滚动条。
   * 数据按页(120 条)向后端取,滚动到哪取到哪。
   */
  let {
    query,
    version,
    onopen,
    onmenu,
    ontotal,
    onloading,
    empty,
    layout = 'posters', density = 'comfortable', initialScroll = 0, onscrollposition,
  }: {
    query: Query
    version: number
    onopen: (code: string, index?: number) => void
    onmenu?: (card: MovieCard, x: number, y: number) => void
    ontotal?: (total: number) => void
    onloading?: (loading: boolean) => void
    empty?: Snippet
    layout?: Layout
    density?: Density
    initialScroll?: number
    onscrollposition?: (value: number) => void
  } = $props()

  const PAGE = 120
  const GAP = 20
  const META = 48
  const OVERSCAN = 2

  let scroller: HTMLDivElement
  let spacer: HTMLDivElement | undefined = $state()
  let spacerHeight = $state(0)
  let width = $state(0)
  let height = $state(0)
  let scrollTop = $state(0)
  let baseW = $state(168)
  let cardGap = $state(8)
  let total = $state(0)
  let focusIndex = $state(0)
  let focusTarget = $state<number | null>(null)
  let loaded = $state(false)
  const cache = new PosterPages<MovieCard>()
  let pages = $state.raw<Record<number, PosterPage<MovieCard>>>({})
  let fresh = $state(true)
  let failures = $state<Record<number, string>>({})
  let pending = $state<Record<number, boolean>>({})

  let gen = $state(0)
  let refreshZero = $state(false)
  let controller = new AbortController()
  let lastKey = ''
  const inflight = new Set<string>()
  let freshTimer: ReturnType<typeof setTimeout> | undefined
  let restore = untrack(() => initialScroll)
  let restorePending = true

  const targetW = $derived(density === 'compact' ? 128 : density === 'large' ? 240 : baseW)
  const cols = $derived(layout === 'list' ? 1 : Math.max(1, Math.floor((width + GAP) / (targetW + GAP))))
  const cw = $derived(Math.max(1, (width - (cols - 1) * GAP) / cols))
  const rowH = $derived(layout === 'list' ? 72 * appearance.value.textScale / 100 + 12 : (cw * 438) / 309 + cardGap + META * appearance.value.textScale / 100 + GAP)
  const skeletonCount = $derived(cols * Math.max(1, Math.min(3, Math.ceil(height / rowH))))
  const rows = $derived(Math.ceil(total / cols))
  const first = $derived(Math.max(0, Math.floor(scrollTop / rowH) - OVERSCAN))
  const last = $derived(Math.min(rows, Math.ceil((scrollTop + height) / rowH) + OVERSCAN))
  const indices = $derived(
    Array.from({ length: Math.max(0, Math.min(total, last * cols) - first * cols) }, (_, k) => first * cols + k),
  )

  const requiredPages = $derived.by(() => {
    if (!total) return [0]
    const from = Math.min(first * cols, total - 1)
    const to = Math.max(from, Math.min(total, last * cols) - 1)
    const required = new Set<number>()
    if (refreshZero) required.add(0)
    for (let p = Math.floor(from / PAGE); p <= Math.floor(to / PAGE); p++) required.add(p)
    return [...required]
  })
  const busy = $derived(requiredPages.some(p => pending[p] || (pages[p]?.generation !== gen && !failures[p])))
  $effect(() => { onloading?.(busy) })

  const cardAt = (i: number): MovieCard | null => pages[Math.floor(i / PAGE)]?.items[i % PAGE] ?? null
  function publish() { pages = cache.snapshot() }

  // Queue only the current viewport, and retain old invocations in the global
  // request budget until they settle. Native invoke cannot abort a list query.
  function pump() {
    for (const p of requiredPages) {
      if (inflight.size >= POSTER_REQUEST_LIMIT) break
      if (cache.fresh(p, gen) || failures[p] || inflight.has(`${gen}:${p}`)) continue
      void fetchPage(p)
    }
  }
  async function fetchPage(p: number) {
    const g = gen
    const key = `${g}:${p}`
    inflight.add(key)
    pending[p] = true
    try {
      const r = await posterRequest({ ...$state.snapshot(query), offset: p * PAGE, limit: PAGE }, controller.signal)
      if (g !== gen) return
      total = r.total
      if (p === 0) refreshZero = false
      cache.set(p, r.items, g, requiredPages)
      publish()
      delete failures[p]
      if (!loaded) {
        fresh = true
        clearTimeout(freshTimer)
        freshTimer = setTimeout(() => (fresh = false), 1100)
      }
      loaded = true
      ontotal?.(r.total)
    } catch (e) {
      if (g === gen && requiredPages.includes(p)) failures[p] = String(e)
    } finally {
      inflight.delete(key)
      if (g === gen) delete pending[p]
      // An unmounted wall retires its generation and must never start new work.
      if (alive) pump()
    }
  }
  let alive = true
  function retry() { failures = {}; pump() }

  // Keep visible old cards during refresh; other generations cannot overwrite
  // new queries, and revisiting an evicted page fetches the current generation.
  $effect(() => {
    const key = JSON.stringify(query)
    version
    untrack(() => {
      gen++
      controller.abort()
      controller = new AbortController()
      pending = {}
      failures = {}
      const queryChanged = key !== lastKey
      if (queryChanged && lastKey) { restore = 0; restorePending = false }
      lastKey = key
      if (queryChanged) {
        focusTarget = null
        cache.clear()
        focusIndex = 0
        loaded = false
        total = 0
        scrollTop = 0
        if (scroller) scroller.scrollTop = 0
        fresh = true
        clearTimeout(freshTimer)
      } else cache.retain(requiredPages)
      refreshZero = true
      publish()
      pump()
    })
  })
  let presentation = ''
  $effect(() => {
    const key = `${layout}:${density}`
    untrack(() => {
      if (presentation && presentation !== key && scroller) {
        scroller.scrollTop = 0; scrollTop = 0; onscrollposition?.(0)
      }
      presentation = key
    })
  })
  $effect(() => {
    if (!loaded || !scroller || !height || !restorePending) return
    untrack(() => {
      restorePending = false
      scroller.scrollTop = Math.min(restore, Math.max(0, rows * rowH - height))
      scrollTop = scroller.scrollTop
    })
  })
  $effect(() => {
    const required = requiredPages
    gen
    untrack(() => {
      cache.touch(required)
      cache.trim(required)
      publish()
      failures = Object.fromEntries(Object.entries(failures).filter(([p]) => required.includes(Number(p))))
      pump()
    })
  })

  $effect(() => {
    if (!spacer) return
    const observer = new ResizeObserver(([entry]) => spacerHeight = entry.contentRect.height)
    observer.observe(spacer)
    return () => observer.disconnect()
  })
  // A layout switch can resize the spacer after the key event. Repeat positioning
  // when its real geometry changes, before loading and focusing the target card.
  $effect(() => {
    const target = focusTarget
    spacerHeight; rowH; height; scrollTop
    if (target === null || !scroller) return
    untrack(() => {
      if (document.activeElement !== scroller) return
      const top = Math.floor(target / cols) * rowH
      if (top < scrollTop || top + rowH > scrollTop + height) scroller.scrollTop = Math.max(0, top - (height - rowH) / 2)
      scrollTop = scroller.scrollTop
    })
  })
  function keyNavigate(e: KeyboardEvent) {
    if (e.isComposing || e.altKey || e.metaKey || e.shiftKey || !total) return
    const from = (e.target as HTMLElement).closest<HTMLElement>('[data-card-index]')
    if (!from && e.target !== scroller) return
    const index = focusTarget ?? (from ? Number(from.dataset.cardIndex) : focusIndex)
    const delta = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowDown' ? cols : e.key === 'ArrowUp' ? -cols : 0
    if (!delta && e.key !== 'Home' && e.key !== 'End') return
    e.preventDefault()
    const target = Math.max(0, Math.min(total - 1, e.key === 'Home' ? 0 : e.key === 'End' ? total - 1 : index + delta))
    focusIndex = target
    focusTarget = target
    scroller.focus({ preventScroll:true })
    const top = Math.floor(target / cols) * rowH
    if (top < scrollTop || top + rowH > scrollTop + height) scroller.scrollTop = Math.max(0, top - (height - rowH) / 2)
    scrollTop = scroller.scrollTop
  }
  $effect(() => {
    const target = focusTarget
    if (target === null || !indices.includes(target) || pages[Math.floor(target / PAGE)]?.generation !== gen || !cardAt(target)) return
    const generation = gen
    void tick().then(() => {
      if (focusTarget !== target || gen !== generation || document.activeElement !== scroller) return
      scroller.querySelector<HTMLElement>(`[data-card-index="${target}"]`)?.focus({ preventScroll:true })
      focusTarget = null
    })
  })

  onMount(() => {
    scroller.addEventListener('keydown', keyNavigate)
    const measure = () => {
      const style = getComputedStyle(scroller)
      baseW = parseFloat(style.getPropertyValue('--card-w')) || 168
      cardGap = parseFloat(style.getPropertyValue('--s2')) || 8
    }
    const ro = new ResizeObserver(([e]) => {
      width = e.contentRect.width
      height = e.contentRect.height
      measure()
    })
    ro.observe(scroller)
    measure()
    return () => {
      scroller.removeEventListener('keydown', keyNavigate)
      alive = false
      gen++
      controller.abort()
      cache.clear()
      publish()
      ro.disconnect()
      clearTimeout(freshTimer)
    }
  })
</script>

<div class="scroller" tabindex="-1" role="region" aria-label="影片浏览，方向键移动，Home 和 End 跳到首尾" data-layout={layout} data-density={density} data-cached-pages={Object.keys(pages).length} data-cached-cards={Object.values(pages).reduce((n, p) => n + p.items.length, 0)} data-page-budget={Math.max(POSTER_PAGE_LIMIT, requiredPages.length)} aria-busy={busy} onwheel={() => focusTarget = null} onpointerdown={() => focusTarget = null} bind:this={scroller} onscroll={(e) => { scrollTop = e.currentTarget.scrollTop; if (!restorePending) onscrollposition?.(scrollTop) }}>
  <div class="loading-note">
    {#if busy}<LoadingStatus label={loaded ? '正在更新影片列表…' : query.text ? '正在查找匹配的影片…' : '正在读取媒体库…'} />{/if}
  </div>
  {#if Object.keys(failures).length}
    <div class="load-error" role="alert">
      <p>{loaded ? '部分影片未能更新，已显示读取成功的内容。' : '无法读取媒体库。'} {Object.values(failures)[0]}</p>
      <button class="btn" onclick={retry} disabled={busy}>{busy ? '正在重试…' : '重试'}</button>
    </div>
  {/if}
  {#if !loaded && !Object.keys(failures).length}
    <div class="skeleton-grid" style="grid-template-columns:repeat({cols},minmax(0,1fr));gap:{GAP}px" aria-hidden="true">
      {#each Array.from({ length: skeletonCount }) as _, i (i)}<PosterCard card={null} {layout} />{/each}
    </div>
  {:else if loaded && total === 0 && !Object.keys(failures).length}
    <div class="empty">{@render empty?.()}</div>
  {:else}
    <div class="spacer" bind:this={spacer} style="height:{rows * rowH}px">
      <div class="grid" style="transform:translateY({first * rowH}px);grid-template-columns:repeat({cols},minmax(0,1fr));gap:{layout === 'list' ? 12 : GAP}px">
        {#each indices as i (`${i}:${cardAt(i)?.code ?? 'placeholder'}`)}
          <PosterCard card={cardAt(i)} {layout} index={i} tabIndex={i === (indices.includes(focusIndex) ? focusIndex : indices[0]) ? 0 : -1} onfocus={() => { focusIndex = i; focusTarget = null }} delay={fresh ? Math.min(i - first * cols, 14) * 30 : -1} onopen={code => onopen(code, i)} {onmenu} />
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  /* 可视行的位置由虚拟列表控制，浏览器锚定补偿会重复调整滚动位置。 */
  .scroller { position:relative; flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; overflow-anchor:none; padding: var(--s2) var(--s5) var(--s7); }
  .spacer { position: relative; }
  .loading-note { position:sticky; top:0; height:0; z-index:2; display:flex; justify-content:flex-end; pointer-events:none; }
  .loading-note :global(.loading-status) { padding:4px 12px; border-radius:var(--r-pill); background:var(--surface-1); box-shadow:var(--shadow-1); }
  .skeleton-grid { display:grid; }
  .grid { position: absolute; left: 0; right: 0; top: 0; display: grid; align-content: start; will-change: transform; }
  .empty { padding-top: var(--s7); }
  .load-error { display: flex; align-items: center; gap: var(--s3); padding: var(--s4); margin-bottom: var(--s3); background: var(--danger-soft); border-radius: var(--r-sm); }
  .load-error p { flex: 1; overflow-wrap: anywhere; }
  @media (max-width: 720px) { .scroller { padding-inline: var(--s4); } }
</style>
