<script lang="ts">
  import { onMount } from 'svelte'
  import StartupRecovery from './components/StartupRecovery.svelte'
  import ConfirmDialog from './components/ConfirmDialog.svelte'
  import DropZone from './components/DropZone.svelte'
  import Sidebar from './components/Sidebar.svelte'
  import Toast from './components/Toast.svelte'
  import LoadingStatus from './components/LoadingStatus.svelte'
  import { flow } from './lib/importflow.svelte'
  import { app } from './lib/state.svelte'
  import History from './views/History.svelte'
  import Import from './views/Import.svelte'
  import Library from './views/Library.svelte'
  import Onboarding from './views/Onboarding.svelte'
  import Settings from './views/Settings.svelte'

  let initialized = $state(false)
  let starting = $state(false)
  let startupError = $state('')

  // 换页后同一位置的连点可能落到新页面的海报/按钮上。
  // 只保护指针的双击时间窗口;移动到其他控件、键盘激活和稍后的点击照常执行。
  let lastClick: { view: typeof app.view; x: number; y: number; time: number } | null = null
  function guardNavigationClick(e: MouseEvent) {
    if (e.detail === 0) {
      lastClick = null
      return
    }
    if (lastClick && lastClick.view !== app.view && e.timeStamp - lastClick.time < 500
      && Math.hypot(e.clientX - lastClick.x, e.clientY - lastClick.y) <= 4) {
      e.preventDefault()
      e.stopImmediatePropagation()
      return
    }
    lastClick = { view: app.view, x: e.clientX, y: e.clientY, time: e.timeStamp }
  }

  async function start() {
    if (starting) return
    starting = true
    startupError = ''
    try {
      await app.init()
      await flow.init()
      initialized = true
    } catch (e) {
      startupError = String(e)
    } finally {
      starting = false
    }
  }

  onMount(() => {
    void start()
  })
</script>

<svelte:window onclickcapture={guardNavigationClick} />

{#if initialized}
  {#if app.needsOnboarding}
    <main class="solo"><Onboarding /></main>
  {:else}
    <div class="shell">
      <DropZone />
      <Sidebar />
      <main>
        <StartupRecovery />
        <div class="view-shell">
        {#key app.view}
          {#if app.view === 'library'}
            <Library />
          {:else if app.view === 'import'}
            <Import />
          {:else if app.view === 'history'}
            <History />
          {:else}
            <Settings />
          {/if}
        {/key}
        </div>
      </main>
    </div>
  {/if}
{:else}
  <main class="startup">
    {#if startupError}
      <h1>无法启动 kiroku</h1>
      <p role="alert">{startupError}</p>
      <button class="btn primary" onclick={start} disabled={starting}>重试</button>
    {:else}
      <LoadingStatus label="正在准备媒体库…" />
    {/if}
  </main>
{/if}

<ConfirmDialog />
<Toast />

<style>
  .shell { height: 100%; display: flex; }
  main { flex: 1; min-width: 0; height: 100%; overflow: hidden; position: relative; display:flex; flex-direction:column; }
  .view-shell { flex:1; min-height:0; overflow:auto; }
  .solo { height: 100%; overflow: auto; }
  .startup { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--s4); padding: var(--s6); text-align: center; }
  .startup p { max-width: 640px; overflow-wrap: anywhere; }
  @media (max-width: 720px) {
    main { padding-bottom: 64px; }
  }
</style>
