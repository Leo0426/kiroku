<script lang="ts">
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'

  /** 行内番号编辑:未识别的文件直接给输入框,已识别的点铅笔改。提交后由父级决定成功与否。 */
  let {
    code,
    draft = $bindable(),
    error = '',
    disabled = false,
    onsubmit,
  }: { code: string | null; draft?: string; error?: string; disabled?: boolean; onsubmit: (text: string) => Promise<boolean> } = $props()

  let input = $state<HTMLInputElement>()
  let busy = $state(false)
  const open = $derived(draft !== undefined || code === null)

  async function edit() {
    draft = code ?? ''
    await tick()
    input?.focus()
    input?.select()
  }

  async function submit(e: Event) {
    e.preventDefault()
    if (!draft?.trim() || busy || disabled) return
    busy = true
    const ok = await onsubmit(draft)
    busy = false
    if (ok) draft = undefined
  }
</script>

{#if open}
  <form class="edit" onsubmit={submit}>
    <input
      class="input sm"
      type="text"
      placeholder="输入番号,如 ABC-123"
      aria-label="手动输入番号"
      bind:this={input}
      value={draft ?? ''}
      oninput={event => draft = event.currentTarget.value}
      {disabled}
      aria-invalid={error ? 'true' : undefined}
    />
    <button class="btn sm" type="submit" disabled={!draft?.trim() || busy || disabled}>确定</button>
    {#if code !== null}<button class="btn sm ghost" type="button" onclick={() => draft = undefined}>取消</button>{/if}
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </form>
{:else}
  <span class="shown">
    <span class="mono code">{code}</span>
    <button class="pencil" type="button" aria-label="修改番号 {code}" title="修改番号" {disabled} onclick={edit}>
      <Icon name="pencil" size={12} />
    </button>
  </span>
{/if}

<style>
  .edit { display: flex; align-items: center; gap: var(--s1); flex-wrap: wrap; animation: pop 280ms var(--ease-back); transform-origin: left center; }
  .err { animation: rise 240ms var(--ease-out) both; }
  .input.sm { height: 28px; width: 150px; font-size: var(--fs-sm); padding: 0 var(--s2); }
  .err { flex-basis: 100%; color: var(--danger); font-size: var(--fs-xs); line-height: 1.3; }
  .shown { display: inline-flex; align-items: center; gap: var(--s1); }
  .code { color: var(--accent); font-weight: 600; }
  .pencil {
    display: grid; place-items: center; width: 22px; height: 22px; border: 0; border-radius: 6px;
    background: transparent; color: var(--text-3); opacity: 0;
    transition: opacity var(--d-fast), background var(--d-fast);
  }
  :global(.scanrow:hover) .pencil, .pencil:focus-visible { opacity: 1; }
  @media (hover: none) { .pencil { opacity: 1; } }
  .pencil:hover { background: var(--surface-3); color: var(--text-1); }
</style>
