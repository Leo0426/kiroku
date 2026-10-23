<script lang="ts">
  import { flip } from 'svelte/animate'
  import { fly } from 'svelte/transition'
  import { dur } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import Icon from './Icon.svelte'
</script>

<div class="host" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div
      class="toast {t.kind}"
      role={t.kind === 'error' ? 'alert' : 'status'}
      in:fly={{ y: 16, duration: dur(280) }}
      out:fly={{ y: 8, duration: dur(180) }}
      animate:flip={{ duration: dur(200) }}
    >
      <Icon name={t.kind === 'ok' ? 'check' : t.kind === 'error' ? 'alert' : 'info'} size={16} />
      <span>{t.text}</span>
      {#if t.action}<button class="btn sm action" onclick={t.action.run}>{t.action.label}</button>{/if}
      <button class="x" aria-label="关闭" onclick={() => app.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .host {
    position: fixed; z-index: 100; left: 50%; bottom: var(--s5); transform: translateX(-50%);
    display: flex; flex-direction: column; gap: var(--s2); align-items: center; pointer-events: none;
    width: min(92vw, 480px);
  }
  .toast {
    pointer-events: auto; display: flex; align-items: center; gap: var(--s3);
    padding: 10px 10px 10px var(--s4); border-radius: var(--r-md);
    background: var(--surface-3); box-shadow: var(--shadow-2);
    font-size: var(--fs-sm); max-width: 100%;
    transition: transform var(--d-base) var(--ease-back);
  }
  .toast:hover { transform: translateY(-2px); }
  .toast span { flex: 1; word-break: break-word; }
  .action { flex-shrink: 0; white-space: nowrap; }
  .ok :global(svg:first-child) { color: var(--ok); }
  .error :global(svg:first-child) { color: var(--danger); }
  .info :global(svg:first-child) { color: var(--info); }
  .x { border: 0; background: transparent; color: var(--text-3); width: 24px; height: 24px; border-radius: 6px; display: grid; place-items: center; }
  .x:hover { background: var(--surface-hover); color: var(--text-1); }
</style>
