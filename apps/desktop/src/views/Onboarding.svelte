<script lang="ts">
  import Icon from '../components/Icon.svelte'
  import Logo from '../components/Logo.svelte'
  import { rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'

  let busy = $state(false)

  async function run(f: () => Promise<boolean>) {
    busy = true
    try {
      if (await f()) app.toast('ok', '媒体库已就绪')
    } finally {
      busy = false
    }
  }
</script>

<div class="wrap stagger" in:rise={{ duration: 420, y: 14 }}>
  <div class="hero">
    <div class="mark"><Logo size={76} busy={busy} /></div>
    <h1>欢迎使用 kiroku</h1>
    <p class="lead">把散落的视频整理成一座有封面、有简介的本地媒体库。</p>
  </div>

  <ol class="steps">
    <li><b>1</b><span><strong>选择媒体库文件夹</strong><em>整理好的影片、NFO 与海报会放在这里,Jellyfin / Emby / Kodi 都能直接读取。</em></span></li>
    <li><b>2</b><span><strong>导入待整理的文件夹</strong><em>自动识别番号、联网刮削元数据,先预览整理计划再执行。</em></span></li>
    <li><b>3</b><span><strong>浏览与播放</strong><em>海报墙搜索筛选,一键用系统播放器打开;任何一批整理都可以撤销。</em></span></li>
  </ol>

  {#if app.media?.suggestedLibrary}
    <div class="choice">
      <button class="btn primary big" onclick={() => run(() => app.useDefaultLibraryRoot())} disabled={busy}>
        <Icon name="check" size={18} />使用默认位置
      </button>
      <span class="path mono" title={app.media.suggestedLibrary}>{app.prettyPath(app.media.suggestedLibrary)}</span>
    </div>
    <button class="btn ghost" onclick={() => run(() => app.chooseLibraryRoot())} disabled={busy}>
      <Icon name="folder" size={16} />选择其他文件夹
    </button>
  {:else}
    <button class="btn primary big" onclick={() => run(() => app.chooseLibraryRoot())} disabled={busy}>
      <Icon name="folder" size={18} />选择媒体库文件夹
    </button>
  {/if}
  <p class="faint note">默认放在系统的「影片」文件夹里,不存在会自动创建。已经有媒体库?点「选择其他文件夹」选中它,会自动从 NFO 建立索引。</p>
</div>

<style>
  .wrap { min-height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--s5); padding: var(--s6) var(--s5); text-align: center; }
  .hero { display: flex; flex-direction: column; align-items: center; gap: var(--s3); }
  .mark { filter: drop-shadow(0 14px 30px rgb(0 0 0 / 0.35)); animation: float 4s ease-in-out infinite; }
  h1 { font-size: 30px; font-weight: 700; letter-spacing: -0.015em; }
  .lead { color: var(--text-2); font-size: var(--fs-lg); max-width: 420px; }
  .steps { list-style: none; margin: var(--s3) 0; padding: 0; display: flex; flex-direction: column; gap: var(--s3); width: min(520px, 100%); text-align: left; }
  .steps li {
    display: flex; gap: var(--s4); align-items: flex-start; padding: var(--s4); background: var(--surface-1); border-radius: var(--r-md);
    animation: rise var(--d-slow) var(--ease-out) both; transition: transform var(--d-base) var(--ease-back), background var(--d-fast);
  }
  .steps li:nth-child(2) { animation-delay: 80ms; }
  .steps li:nth-child(3) { animation-delay: 160ms; }
  .steps li:hover { transform: translateX(4px); background: var(--surface-2); }
  .steps li:hover b { background: var(--accent); color: var(--accent-ink); transform: scale(1.1); }
  .steps b { flex-shrink: 0; width: 26px; height: 26px; border-radius: 50%; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent); font-size: var(--fs-sm); transition: background var(--d-base), color var(--d-base), transform var(--d-base) var(--ease-back); }
  .steps strong { display: block; font-weight: 600; }
  .steps em { display: block; font-style: normal; color: var(--text-2); font-size: var(--fs-sm); margin-top: 2px; }
  .big { height: 46px; padding: 0 var(--s6); font-size: var(--fs-lg); border-radius: var(--r-md); }
  @media (prefers-reduced-motion: reduce) { .mark, .steps li { animation: none; } }
  .note { font-size: var(--fs-sm); max-width: 440px; }
  .choice { display: flex; flex-direction: column; align-items: center; gap: var(--s2); }
  .path { font-size: var(--fs-sm); color: var(--text-2); padding: 3px 10px; border-radius: var(--r-pill); background: var(--surface-2); transition: color var(--d-fast), background var(--d-fast); }
  .choice:hover .path { color: var(--text-1); background: var(--surface-3); }
</style>
