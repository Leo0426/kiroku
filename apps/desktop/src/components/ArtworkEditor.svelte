<script lang="ts">
  import { onDestroy } from 'svelte'
  import { fade, scale } from 'svelte/transition'
  import { api } from '../lib/api'
  import { dur } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import type { ArtworkOriginal, CropBox } from '../lib/types'
  import Icon from './Icon.svelte'

  /**
   * 海报裁剪弹窗:裁剪框按海报比例固定大小(尽量大),只能在原图上拖动;框里的内容就是保存的海报。
   * 每次都从保留的原图裁,保存走可撤销修订并锁定海报字段。
   */
  let { code, onsaved, onclose }: { code: string; onsaved: () => void; onclose: () => void } = $props()

  /** 与核心 `organize::POSTER_RATIO` 一致(DMM 封面宽 ≈ 高 × 0.7047) */
  const RATIO = 0.7047

  let original = $state<ArtworkOriginal | null>(null)
  let source = $state('')
  let x = $state(0)
  let y = $state(0)
  let busy = $state(false)
  let saving = $state(false)
  let error = $state('')
  let img: HTMLImageElement | undefined = $state()
  let frame: HTMLDivElement | undefined = $state()
  let generation = 0
  let url = ''

  const size = $derived.by(() => {
    if (!original) return { width: 1, height: 1 }
    const { width: W, height: H } = original
    return W / H >= RATIO
      ? { width: Math.min(W, Math.round(H * RATIO)), height: H }
      : { width: W, height: Math.min(H, Math.round(W / RATIO)) }
  })
  const rect = $derived<CropBox>({ x, y, ...size })
  const maxX = $derived(original ? original.width - size.width : 0)
  const maxY = $derived(original ? original.height - size.height : 0)
  const clamp = (v: number, max: number) => Math.round(Math.min(Math.max(v, 0), max))

  function release() {
    generation++
    if (url) URL.revokeObjectURL(url)
    url = ''
  }
  onDestroy(() => {
    if (busy) void api.jobCancel()
    release()
  })

  $effect(() => {
    code
    void open()
  })

  async function open() {
    release()
    const g = generation
    original = null
    busy = true
    error = ''
    try {
      const o = await api.artworkOpen(code)
      if (g !== generation) return
      url = URL.createObjectURL(new Blob([new Uint8Array(o.image)], { type: 'image/png' }))
      source = url
      original = o
      // 上次的裁剪框:同尺寸沿用位置,否则以它的中心为准;没有时默认靠右(横版封面的正面在右半边)
      const c = o.crop
      x = clamp(c ? c.x + (c.width - size.width) / 2 : maxX, maxX)
      y = clamp(c ? c.y + (c.height - size.height) / 2 : maxY / 2, maxY)
      requestAnimationFrame(() => frame?.focus())
    } catch (e) {
      if (g === generation) error = String(e)
    } finally {
      if (g === generation) busy = false
    }
  }

  async function save() {
    if (!original || busy) return
    const g = generation
    busy = true
    saving = true
    error = ''
    try {
      const p = (await api.artworkPreview(original.token, original.root, original.code, { ...rect })).revision
      if (g !== generation) return
      const r = await api.revisionSave(p.token, p.root, p.code)
      if (g !== generation) return
      if (r.error || r.indexError) {
        error = [r.error, r.indexError && `索引更新失败：${r.indexError}`].filter(Boolean).join('；') + '。可从历史核对或撤销。'
        return
      }
      app.toast('ok', '海报已保存并锁定，可从历史撤销')
      onsaved()
      onclose()
    } catch (e) {
      if (g === generation) error = String(e)
    } finally {
      if (g === generation) { busy = false; saving = false }
    }
  }

  // 拖动:按下原图任意处开始,位移按显示尺寸换算回原图像素
  let drag = $state<{ px: number; py: number; x: number; y: number; scale: number } | null>(null)
  function down(e: PointerEvent) {
    if (!original || !img || busy || e.button !== 0) return
    e.preventDefault()
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    drag = { px: e.clientX, py: e.clientY, x, y, scale: original.width / img.clientWidth }
    frame?.focus()
  }
  function move(e: PointerEvent) {
    if (!drag) return
    x = clamp(drag.x + (e.clientX - drag.px) * drag.scale, maxX)
    y = clamp(drag.y + (e.clientY - drag.py) * drag.scale, maxY)
  }
  const up = () => (drag = null)

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (!saving) onclose()
      return
    }
    if (e.target !== frame || !original) return
    const step = e.shiftKey ? 10 : 1
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key]
    if (!d) return
    e.preventDefault()
    x = clamp(x + d[0], maxX)
    y = clamp(y + d[1], maxY)
  }

  const pct = (v: number, of: number) => `${(v / of) * 100}%`
</script>

<svelte:window onkeydown={onkey} />

<div class="scrim" role="presentation" onclick={() => !saving && onclose()} transition:fade={{ duration: dur(160) }}></div>
<div class="dlg" role="dialog" aria-modal="true" aria-label="裁剪海报" tabindex="-1" transition:scale={{ start: 0.97, duration: dur(200) }}>
  <header>
    <div>
      <h2>裁剪海报 <span class="mono code">{code}</span></h2>
      <p class="muted">拖动原图上的框选择海报区域，框里的内容就是保存后的海报。方向键微调，按住 Shift 移动更快。</p>
    </div>
    <button class="btn icon ghost" aria-label="关闭裁剪" onclick={onclose} disabled={saving}><Icon name="x" size={18} /></button>
  </header>

  {#if !original}
    <div class="loading">{#if error}<p role="alert">{error}</p><button class="btn" onclick={open}>重试</button>{:else}<p class="muted">正在读取原图…</p>{/if}</div>
  {:else}
    <div class="body">
      <div class="stage">
        <div class="plane" class:dragging={drag} role="presentation" onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}>
          <img bind:this={img} src={source} alt="裁剪原图" draggable="false" />
          <div
            class="frame"
            bind:this={frame}
            role="slider"
            tabindex="0"
            aria-label="裁剪框位置"
            aria-valuemin={0}
            aria-valuemax={Math.max(maxX, maxY)}
            aria-valuenow={maxX ? x : y}
            aria-valuetext="X {x} · Y {y}"
            style:left={pct(x, original.width)}
            style:top={pct(y, original.height)}
            style:width={pct(size.width, original.width)}
            style:height={pct(size.height, original.height)}
          ><span class="grip" aria-hidden="true"></span></div>
        </div>
      </div>
      <aside>
        <div
          class="result"
          role="img"
          aria-label="裁剪结果"
          style:background-image={`url("${source}")`}
          style:background-size={`${(original.width / size.width) * 100}% ${(original.height / size.height) * 100}%`}
          style:background-position={`${maxX ? (x / maxX) * 100 : 0}% ${maxY ? (y / maxY) * 100 : 0}%`}
        ></div>
        <p class="faint tnum" aria-label="裁剪位置">X {x} · Y {y} · {size.width} × {size.height}</p>
        <p class="faint tnum">原图 {original.width} × {original.height} 像素</p>
        {#if original.source}<p class="source">图片来源：{original.source}</p>{/if}
      </aside>
    </div>
    {#if error}<p role="alert" class="err">{error}</p>{/if}
    <footer>
      <span class="muted">保存后锁定海报字段，可从历史撤销。</span>
      <button class="btn" onclick={onclose} disabled={saving}>取消</button>
      <button class="btn primary" onclick={save} disabled={busy}>{saving ? '正在保存…' : '保存海报'}</button>
    </footer>
  {/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 70; background: var(--scrim); backdrop-filter: blur(3px); }
  .dlg {
    position: fixed; z-index: 71; left: 50%; top: 50%; transform: translate(-50%, -50%);
    width: min(980px, calc(100vw - 32px)); max-height: calc(100dvh - 32px); overflow: auto; outline: none;
    display: flex; flex-direction: column; gap: var(--s4); padding: var(--s5);
    border-radius: var(--r-lg); background: var(--surface-1); box-shadow: var(--shadow-3), var(--inset-edge);
  }
  header { display: flex; justify-content: space-between; align-items: flex-start; gap: var(--s3); }
  h2 { font-size: var(--fs-lg); font-weight: 600; }
  .code { color: var(--accent); font-size: var(--fs-sm); margin-left: var(--s2); }
  header p { font-size: var(--fs-sm); margin-top: 2px; }
  .loading { min-height: 240px; display: grid; place-content: center; justify-items: center; gap: var(--s3); }
  .body { display: flex; gap: var(--s5); align-items: flex-start; }
  .stage { flex: 1; min-width: 0; display: flex; justify-content: center; }
  .plane { position: relative; overflow: hidden; border-radius: var(--r-sm); touch-action: none; user-select: none; cursor: grab; line-height: 0; }
  .plane.dragging { cursor: grabbing; }
  .plane img { display: block; max-width: 100%; max-height: min(62dvh, 560px); }
  .frame {
    position: absolute; outline: none;
    border: 2px solid #fff; box-shadow: 0 0 0 9999px rgb(0 0 0 / 0.55), 0 0 0 1px rgb(0 0 0 / 0.35);
  }
  .frame:focus-visible { border-color: var(--accent); }
  .grip { position: absolute; left: 50%; top: 50%; width: 28px; height: 28px; margin: -14px 0 0 -14px; border-radius: 50%; background: rgb(255 255 255 / 0.85); box-shadow: var(--shadow-1); opacity: 0.85; }
  aside { width: 172px; flex-shrink: 0; display: flex; flex-direction: column; gap: var(--s2); }
  .result { width: 100%; aspect-ratio: var(--poster-ratio); border-radius: var(--r-md); background-repeat: no-repeat; background-color: var(--surface-3); box-shadow: var(--shadow-2, var(--shadow-1)); }
  aside p { font-size: var(--fs-xs); }
  .source { overflow-wrap: anywhere; color: var(--text-3); }
  .err { color: var(--danger); font-size: var(--fs-sm); }
  footer { display: flex; align-items: center; justify-content: flex-end; gap: var(--s2); flex-wrap: wrap; }
  footer span { margin-right: auto; font-size: var(--fs-sm); }
  @media (max-width: 720px) {
    .body { flex-direction: column; align-items: stretch; }
    aside { width: 100%; flex-direction: row; flex-wrap: wrap; align-items: center; }
    .result { width: 96px; }
  }
</style>
