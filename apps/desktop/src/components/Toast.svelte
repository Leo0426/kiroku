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
      aria-label={t.title}
      in:fly={{ y: 16, duration: dur(280) }}
      out:fly={{ y: 8, duration: dur(180) }}
      animate:flip={{ duration: dur(200) }}
    >
      <Icon name={t.kind === 'ok' ? 'check' : t.kind === 'error' ? 'alert' : 'info'} size={16} />
      <div class="message">
        {#if t.title}<strong>{t.title}</strong>{/if}
        <p>{t.text}</p>
        {#if t.action}<button class="btn sm action" onclick={t.action.run}>{t.action.label}</button>{/if}
      </div>
      <button class="x" aria-label="关闭通知" onclick={() => app.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .host {
    position: fixed; z-index: 100; right: var(--s5); bottom: var(--s5);
    display: flex; flex-direction: column; gap: var(--s2); align-items: flex-end; pointer-events: none;
    width: min(calc(100vw - 32px), 420px);
  }
  .toast {
    pointer-events: auto; display: flex; align-items: flex-start; gap: var(--s3);
    padding: var(--s3) var(--s3) var(--s3) var(--s4); border-radius: var(--r-md);
    background: var(--surface-1); border: 1px solid var(--line-strong); box-shadow: var(--shadow-3);
    font-size: var(--fs-sm); width: 100%;
  }
  .message { flex: 1; min-width: 0; display: grid; gap: var(--s1); }
  .message p { white-space: pre-line; overflow-wrap: anywhere; }
  .action { justify-self: start; }
  .toast > :global(svg) { margin-top: 2px; }
  .ok :global(svg:first-child) { color: var(--ok); }
  .error :global(svg:first-child) { color: var(--danger); }
  .info :global(svg:first-child) { color: var(--info); }
  .x { flex-shrink: 0; border: 0; background: transparent; color: var(--text-3); width: 24px; height: 24px; border-radius: 6px; display: grid; place-items: center; }
  .x:hover { background: var(--surface-hover); color: var(--text-1); }
  @media (max-width: 720px) { .host { right: 16px; bottom: calc(64px + var(--s3) + env(safe-area-inset-bottom)); } }
</style>
