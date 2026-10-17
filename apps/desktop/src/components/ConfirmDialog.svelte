<script lang="ts">
  import { fade, scale } from 'svelte/transition'
  import { dur } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import Icon from './Icon.svelte'

  let confirmBtn: HTMLButtonElement | undefined = $state()
  $effect(() => {
    confirmBtn?.focus()
  })

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape') app.dialog?.resolve(false)
  }
</script>

<svelte:window onkeydown={onkey} />

{#if app.dialog}
  {@const d = app.dialog}
  <div class="scrim" role="presentation" onclick={() => d.resolve(false)} transition:fade={{ duration: dur(160) }}></div>
  <div class="dlg" role="alertdialog" aria-modal="true" aria-labelledby="dlg-title" aria-describedby="dlg-body" transition:scale={{ start: 0.96, duration: dur(200) }}>
    <div class="head">
      <span class="ico"><Icon name="alert" size={18} /></span>
      <h2 id="dlg-title">{d.title}</h2>
    </div>
    <p id="dlg-body">{d.body}</p>
    <div class="actions">
      <button class="btn" onclick={() => d.resolve(false)}>{d.cancelLabel ?? '取消'}</button>
      <button class="btn primary" bind:this={confirmBtn} onclick={() => d.resolve(true)}>{d.confirmLabel ?? '确定'}</button>
    </div>
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; z-index: 80; background: var(--scrim); backdrop-filter: blur(3px); }
  .dlg {
    position: fixed; z-index: 81; left: 50%; top: 50%; transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 32px)); padding: var(--s5); border-radius: var(--r-lg);
    max-height: calc(100dvh - 32px); overflow: auto;
    background: var(--surface-1); box-shadow: var(--shadow-3), var(--inset-edge);
  }
  .head { display: flex; align-items: center; gap: var(--s3); margin-bottom: var(--s3); }
  .ico { width: 32px; height: 32px; border-radius: 10px; display: grid; place-items: center; background: var(--warn-soft); color: var(--warn); flex-shrink: 0; }
  h2 { font-size: var(--fs-lg); font-weight: 600; line-height: 1.3; }
  p { color: var(--text-2); font-size: var(--fs-sm); line-height: 1.7; white-space: pre-line; }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
