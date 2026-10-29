<script lang="ts">
  import type { Snippet } from 'svelte'
  import Icon from './Icon.svelte'

  /**
   * 标题旁的说明图标:悬停、键盘聚焦或点击时展开说明浮层。
   * 浮层按视口 fixed 定位,不撑开滚动容器、不被 overflow 裁切;靠近窗口右/下边缘时翻向另一侧。
   */
  let { label, children }: { label: string; children: Snippet } = $props()

  const id = `tip-${Math.random().toString(36).slice(2, 9)}`
  let open = $state(false)
  let right = $state(false)
  let up = $state(false)
  let x = $state(0)
  let y = $state(0)
  let host: HTMLSpanElement | undefined = $state()
  let bubble: HTMLSpanElement | undefined = $state()

  function place() {
    if (!host || !bubble) return
    const r = host.getBoundingClientRect()
    const w = bubble.offsetWidth, h = bubble.offsetHeight
    right = r.left - 8 + w > innerWidth - 12
    x = right ? Math.max(12, r.right + 8 - w) : r.left - 8
    up = r.bottom + 8 + h > innerHeight - 12 && r.top - 8 - h >= 12
    y = up ? r.top - 8 - h : r.bottom + 8
  }
  function toggle(e: MouseEvent) {
    // 放在 <summary> 里时不要顺带展开/收起 details
    e.preventDefault()
    place()
    open = !open
  }
  function outside(e: PointerEvent) {
    if (open && host && !host.contains(e.target as Node)) open = false
  }
  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={onkey} onscrollcapture={() => open && place()} onresize={() => open && place()} />

<span class="tip" class:open class:right class:up bind:this={host}>
  <button type="button" class="ico" aria-label={label} aria-describedby={id} aria-expanded={open} onpointerenter={place} onfocus={place} onclick={toggle}>
    <Icon name="info" size={15} />
  </button>
  <span class="bubble" role="tooltip" {id} bind:this={bubble} style:left="{x}px" style:top="{y}px">{@render children()}</span>
</span>

<style>
  .tip { position: relative; display: inline-flex; vertical-align: middle; flex-shrink: 0; }
  .ico {
    display: grid; place-items: center; width: 22px; height: 22px; padding: 0; border: 0; border-radius: 50%;
    background: transparent; color: var(--text-3);
    transition: color var(--d-fast) var(--ease-out), background var(--d-fast) var(--ease-out), transform var(--d-base) var(--ease-back);
  }
  .ico:hover, .ico:focus-visible, .open .ico { color: var(--accent); background: var(--accent-soft); transform: scale(1.1); }
  .ico:focus-visible { outline: none; box-shadow: 0 0 0 2px var(--accent); }

  .bubble {
    position: fixed; z-index: 70;
    width: max-content; max-width: min(360px, calc(100vw - 48px)); max-height: min(60vh, 440px); overflow-y: auto;
    padding: var(--s3) var(--s4); border-radius: var(--r-md);
    background: var(--surface-3); color: var(--text-2); box-shadow: var(--shadow-2), var(--inset-edge);
    font-size: var(--fs-sm); font-weight: 400; line-height: 1.65; text-align: left; white-space: normal; letter-spacing: normal; text-transform: none; cursor: auto;
    opacity: 0; visibility: hidden; transform: translateY(-4px) scale(0.97); transform-origin: 14px 0;
    transition: opacity var(--d-base) var(--ease-out), transform var(--d-base) var(--ease-back), visibility 0s linear var(--d-base);
  }
  .right .bubble { transform-origin: calc(100% - 14px) 0; }
  .up .bubble { transform: translateY(4px) scale(0.97); transform-origin: 14px 100%; }
  .right.up .bubble { transform-origin: calc(100% - 14px) 100%; }
  .tip:hover .bubble, .tip:focus-within .bubble, .open .bubble {
    opacity: 1; visibility: visible; transform: none; transition-delay: 0s;
  }
  .bubble :global(p + p), .bubble :global(* + ul), .bubble :global(* + dl) { margin-top: var(--s2); }
  .bubble :global(b) { color: var(--text-1); font-weight: 600; }
</style>
