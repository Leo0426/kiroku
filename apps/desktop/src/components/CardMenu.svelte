<script lang="ts">
  import { tick } from 'svelte'
  import { fade } from 'svelte/transition'
  import { api } from '../lib/api'
  import { setFavorite } from '../lib/favorite'
  import { dur } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import type { MovieCard } from '../lib/types'
  import Icon from './Icon.svelte'

  /** 海报右键菜单。x / y 是视口坐标,菜单会被夹回窗口内。 */
  let { card, x, y, onopen, oncrop, ontrashed, onclose }: {
    card: MovieCard
    x: number
    y: number
    onopen: (code: string) => void
    oncrop: (code: string) => void
    ontrashed: (code: string) => void
    onclose: () => void
  } = $props()

  let menu: HTMLDivElement | undefined = $state()
  let left = $state(0)
  let top = $state(0)

  $effect(() => {
    const at = [x, y]
    void tick().then(() => {
      if (!menu) return
      const { width, height } = menu.getBoundingClientRect()
      left = Math.max(8, Math.min(at[0], innerWidth - width - 8))
      top = Math.max(8, Math.min(at[1], innerHeight - height - 8))
      menu.querySelector<HTMLButtonElement>('button')?.focus()
    })
  })

  // 菜单先关掉(组件随之卸载),动作里只用这里取下的快照
  async function run(action: (c: MovieCard) => Promise<void>) {
    const c = { ...card }
    onclose()
    try { await action(c) } catch (e) { app.toast('error', String(e), 5000) }
  }

  const play = () => run(async (card) => {
    await api.play(card.code, card.parts > 1 ? 1 : undefined)
    app.toast('info', '已交给系统播放器打开')
  })
  const reveal = () => run((card) => api.reveal(card.code))
  const favorite = () => run(async (card) => {
    await setFavorite(card.code, !card.favorite)
    app.toast('ok', card.favorite ? '已取消收藏，可从历史撤销' : '已收藏，可从历史撤销')
  })
  const trash = (keepMedia: boolean) => run(async (card) => {
    const name = card.title ? `${card.code} ${card.title}` : card.code
    const ok = await app.confirm(keepMedia ? {
      title: '只删除资料？',
      body: `「${name}」的 NFO、海报、背景图等资料会被移到系统废纸篓，影片从媒体库移除。\n视频和字幕文件留在原文件夹里不动。`,
      confirmLabel: '删除资料',
    } : {
      title: '删除影片和文件？',
      body: `「${name}」所在的整个影片文件夹（视频、字幕、NFO、图片）会被移到系统废纸篓，并从媒体库移除。\n需要时可以在废纸篓里「放回原处」，再重建索引恢复。`,
      confirmLabel: '删除影片和文件',
    })
    if (!ok) return
    const r = await api.libraryTrash(card.code, keepMedia)
    ontrashed(card.code)
    app.toast('ok', r.missing ? `${card.code} 的文件夹已不在磁盘上，已从媒体库移除` : keepMedia ? `已将 ${card.code} 的资料移到废纸篓，视频保留` : `已将 ${card.code} 移到废纸篓`)
  })

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onclose(); return }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const items = [...(menu?.querySelectorAll<HTMLButtonElement>('button') ?? [])]
    const i = items.indexOf(document.activeElement as HTMLButtonElement)
    items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus()
  }
</script>

<svelte:window onkeydown={onkey} onblur={onclose} onresize={onclose} />

<div class="catcher" role="presentation" onpointerdown={onclose} oncontextmenu={(e) => { e.preventDefault(); onclose() }}></div>
<div class="menu" role="menu" aria-label="{card.code} 操作" bind:this={menu} style:left="{left}px" style:top="{top}px" transition:fade={{ duration: dur(120) }}>
  <button role="menuitem" onclick={() => { const { code } = card; onclose(); onopen(code) }}><Icon name="info" size={14} />查看详情</button>
  <button role="menuitem" onclick={play}><Icon name="play" size={13} />播放</button>
  <button role="menuitem" onclick={reveal}><Icon name="folder" size={14} />在文件夹中显示</button>
  <button role="menuitem" onclick={() => { const { code } = card; onclose(); oncrop(code) }}><Icon name="pencil" size={14} />裁剪海报…</button>
  <button role="menuitem" onclick={favorite}><Icon name="star" size={14} />{card.favorite ? '取消收藏' : '收藏'}</button>
  <hr />
  <button role="menuitem" class="danger" onclick={() => trash(true)}><Icon name="trash" size={14} />只删除资料…</button>
  <button role="menuitem" class="danger" onclick={() => trash(false)}><Icon name="trash" size={14} />删除影片和文件…</button>
</div>

<style>
  .catcher { position: fixed; inset: 0; z-index: 60; }
  .menu {
    position: fixed; z-index: 61; min-width: 184px; padding: 4px;
    border-radius: var(--r-md); background: var(--surface-1); box-shadow: var(--shadow-3), var(--inset-edge);
  }
  button {
    display: flex; align-items: center; gap: var(--s2); width: 100%; padding: 7px 10px;
    border: 0; border-radius: var(--r-sm); background: transparent; color: inherit; font-size: var(--fs-sm); text-align: left;
  }
  button :global(svg) { color: var(--text-2); flex-shrink: 0; }
  button:hover, button:focus-visible { background: var(--surface-hover); outline: none; }
  .danger, .danger :global(svg) { color: var(--danger); }
  .danger:hover, .danger:focus-visible { background: var(--danger-soft); }
  hr { margin: 4px 6px; border: 0; border-top: 1px solid var(--line); }
</style>
