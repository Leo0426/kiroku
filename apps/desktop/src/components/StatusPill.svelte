<script lang="ts">
  import type { Snippet } from 'svelte'
  import Icon from './Icon.svelte'

  let {
    tone = 'neutral',
    busy = false,
    title,
    children,
  }: { tone?: 'neutral' | 'info' | 'ok' | 'warn' | 'danger' | 'accent'; busy?: boolean; title?: string; children: Snippet } = $props()
</script>

<span class="pill {tone}" {title}>
  {#if busy}<Icon name="loader" size={12} />{/if}
  {@render children()}
</span>

<style>
  .pill {
    display: inline-flex; align-items: center; gap: 5px;
    height: 22px; padding: 0 9px;
    border-radius: var(--r-pill);
    font-size: var(--fs-xs); font-weight: 500; line-height: 1; white-space: nowrap;
    background: var(--surface-3); color: var(--text-2);
    transition: background var(--d-base) var(--ease-out), color var(--d-base) var(--ease-out), transform var(--d-base) var(--ease-back);
    animation: pop 300ms var(--ease-back);
  }
  .pill:hover { transform: translateY(-1px); }
  .info { background: var(--info-soft); color: var(--info); }
  .ok { background: var(--ok-soft); color: var(--ok); }
  .warn { background: var(--warn-soft); color: var(--warn); }
  .danger { background: var(--danger-soft); color: var(--danger); }
  .accent { background: var(--accent-soft); color: var(--accent); }
</style>
