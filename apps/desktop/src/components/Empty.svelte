<script lang="ts">
  import type { Snippet } from 'svelte'
  import Icon from './Icon.svelte'

  let {
    icon = 'film',
    title,
    description,
    children,
  }: { icon?: 'film' | 'search' | 'history' | 'download' | 'folder'; title: string; description?: string; children?: Snippet } = $props()
</script>

<div class="empty stagger">
  <div class="art"><Icon name={icon} size={28} /></div>
  <h3>{title}</h3>
  {#if description}<p>{description}</p>{/if}
  {#if children}<div class="actions">{@render children()}</div>{/if}
</div>

<style>
  .empty { display: flex; flex-direction: column; align-items: center; text-align: center; gap: var(--s2); padding: var(--s7) var(--s5); max-width: 420px; margin: 0 auto; }
  .art {
    width: 64px; height: 64px; border-radius: 20px; display: grid; place-items: center;
    background: var(--surface-2); color: var(--text-2); margin-bottom: var(--s2);
    animation: rise var(--d-slow) var(--ease-out) both, float 3.6s 600ms ease-in-out infinite;
    transition: transform var(--d-base) var(--ease-back), color var(--d-base);
  }
  .art:hover { transform: scale(1.08) rotate(-6deg); color: var(--accent); }
  @media (prefers-reduced-motion: reduce) { .art { animation: none; } }
  h3 { font-size: var(--fs-lg); font-weight: 600; }
  p { color: var(--text-2); font-size: var(--fs-sm); }
  .actions { display: flex; gap: var(--s2); margin-top: var(--s3); flex-wrap: wrap; justify-content: center; }
</style>
