<script lang="ts">
  import { baseName } from '../lib/format'
  import { flow } from '../lib/importflow.svelte'
  import { app, type View } from '../lib/state.svelte'
  import Icon from './Icon.svelte'
  import Logo from './Logo.svelte'

  const items: { view: View; label: string; icon: 'grid' | 'download' | 'history' | 'settings' }[] = [
    { view: 'library', label: '媒体库', icon: 'grid' },
    { view: 'import', label: '导入整理', icon: 'download' },
    { view: 'history', label: '历史', icon: 'history' },
    { view: 'settings', label: '设置', icon: 'settings' },
  ]
  const index = $derived(Math.max(0, items.findIndex((i) => i.view === app.view)))
  const busy = $derived(flow.busy || app.rebuild.running)
</script>

<nav class="side" aria-label="主导航">
  <div class="brand">
    <Logo size={28} {busy} />
    <span class="word">kiroku</span>
  </div>

  <ul style="--i:{index}">
    <!-- 选中高亮块:在项目之间弹性滑动 -->
    <span class="thumb" aria-hidden="true"></span>
    {#each items as it (it.view)}
      <li>
        <button class="item ripple-host" class:active={app.view === it.view} aria-label={it.label} title={it.label} aria-current={app.view === it.view ? 'page' : undefined} onclick={() => app.navigate(it.view)}>
          <Icon name={it.icon} size={18} />
          <span class="label">{it.label}</span>
          {#if it.view === 'import' && flow.busy}<span class="dot" title="任务进行中"></span>{/if}
        </button>
      </li>
    {/each}
  </ul>

  <button class="quick btn ghost" onclick={() => app.commandsOpen = true} aria-label="快捷操作" title="快捷操作 · Ctrl / ⌘ K"><Icon name="search" size={16}/><span class="label">快捷操作</span></button>
  {#if app.settings?.library_root}
    <div class="foot" title={app.settings.library_root}>
      <span class="faint">媒体库</span>
      <span class="root">{baseName(app.settings.library_root)}</span>
    </div>
  {/if}
</nav>

<style>
  .side {
    --row: 38px;
    --gap: 2px;
    /* 影片详情是内容面板,不应挡住应用导航;全局确认框仍在更高层。 */
    position: relative; z-index: 50;
    display: flex; flex-direction: column; gap: var(--s5);
    width: var(--sidebar-w); flex-shrink: 0; padding: var(--s5) var(--s3) var(--s4);
    background: var(--surface-1); border-right:1px solid var(--line);
  }
  .quick { margin-top:auto; }.quick + .foot { margin-top:0; }
  .brand { display: flex; align-items: center; gap: var(--s3); padding: 0 var(--s2); height: 32px; cursor: default; }
  .word { font-weight: 700; font-size: var(--fs-lg); letter-spacing: 0.02em; transition: letter-spacing var(--d-slow) var(--ease-out); }
  .brand:hover .word { letter-spacing: 0.08em; }

  ul { position: relative; list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--gap); }
  .thumb {
    position: absolute; left: 0; right: 0; top: 0; height: var(--row); border-radius: var(--r-sm);
    background: var(--accent-soft);
    transform: translateY(calc(var(--i) * (var(--row) + var(--gap))));
    transition: transform var(--d-slow) var(--ease-back);
  }
  .thumb::before {
    content: ''; position: absolute; left: -12px; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--accent);
  }
  li { position: relative; z-index: 1; }
  .item {
    display: flex; align-items: center; gap: var(--s3);
    width: 100%; height: var(--row); padding: 0 var(--s3);
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--text-2); text-align: left; font-weight: 500;
    transition: color var(--d-fast) var(--ease-out), background var(--d-fast) var(--ease-out), transform var(--d-base) var(--ease-back);
  }
  .item:hover:not(.active) { background: color-mix(in srgb, var(--surface-3) 70%, transparent); color: var(--text-1); }
  .item:active { transform: scale(0.97); }
  .item.active { color: var(--accent); }
  .label { transition: transform var(--d-base) var(--ease-out); }
  .item:hover .label { transform: translateX(2px); }
  .dot { margin-left: auto; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: pulse 1.4s ease-in-out infinite; }
  .foot { margin-top: auto; display: flex; flex-direction: column; padding: var(--s3); border-radius: var(--r-sm); font-size: var(--fs-xs); min-width: 0; transition: background var(--d-fast); }
  .foot:hover { background: var(--surface-2); }
  .root { color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sm); }

  /* 中等宽度:只留图标 */
  @media (max-width: 820px) {
    .side { width: 64px; align-items: center; padding-inline: var(--s2); }
    .word, .label, .foot { display: none; }
    .brand { padding: 0; }
    ul, .item { width: 44px; }
    .item { justify-content: center; padding: 0; }
    .thumb::before { left: -8px; }
    .dot { position: absolute; top: 8px; right: 8px; margin: 0; }
  }
  /* 窄屏:底部 Tab 栏(滑块不适用,改为选中项自身高亮) */
  @media (max-width: 720px) {
    .side {
      position: fixed; z-index: 50; left: 0; right: 0; bottom: 0; top: auto; width: auto; flex-direction: row; gap: 0;
      padding: var(--s2) var(--s2) calc(var(--s2) + env(safe-area-inset-bottom)); border-top: 1px solid var(--line);
      background: color-mix(in srgb, var(--surface-1) 88%, transparent); backdrop-filter: blur(16px);
    }
    .brand, .thumb, .quick { display: none; }
    ul { flex-direction: row; width: 100%; justify-content: space-around; }
    .item { flex-direction: column; gap: 2px; width: auto; flex: 1; height: 48px; font-size: 10.5px; }
    .item .label { display: block; }
    .item.active { background: var(--accent-soft); }
  }
</style>
