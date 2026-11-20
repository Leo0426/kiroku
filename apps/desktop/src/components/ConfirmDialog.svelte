<script lang="ts">
  import { app } from '../lib/state.svelte'
  import Icon from './Icon.svelte'

  let dialog: HTMLDialogElement
  let confirmBtn: HTMLButtonElement | undefined = $state()
  $effect(() => {
    if (app.dialog && !dialog.open) {
      dialog.showModal()
      confirmBtn?.focus()
    } else if (!app.dialog && dialog.open) dialog.close()
  })
</script>

<dialog class="app-dialog" bind:this={dialog} role="alertdialog" aria-labelledby="dlg-title" aria-describedby="dlg-body"
  oncancel={(e) => { e.preventDefault(); app.dialog?.resolve('dismiss') }}
  onclick={(e) => { if (e.target === dialog) app.dialog?.resolve('dismiss') }}>
  {#if app.dialog}
    {@const d = app.dialog}
    <div class="content">
      <div class="head">
        <span class="ico"><Icon name="alert" size={18} /></span>
        <h2 id="dlg-title">{d.title}</h2>
      </div>
      <p id="dlg-body">{d.body}</p>
      <div class="actions">
        <button class="btn" onclick={() => d.resolve('alternative')}>{d.cancelLabel ?? '取消'}</button>
        <button class="btn primary" bind:this={confirmBtn} onclick={() => d.resolve('confirm')}>{d.confirmLabel ?? '确定'}</button>
      </div>
    </div>
  {/if}
</dialog>

<style>
  .content { padding: var(--s5); }
  .head { display: flex; align-items: center; gap: var(--s3); margin-bottom: var(--s3); }
  .ico { width: 32px; height: 32px; border-radius: 10px; display: grid; place-items: center; background: var(--warn-soft); color: var(--warn); flex-shrink: 0; }
  h2 { font-size: var(--fs-lg); font-weight: 600; line-height: 1.3; }
  p { color: var(--text-2); font-size: var(--fs-sm); line-height: 1.7; white-space: pre-line; }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
