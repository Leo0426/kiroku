<script lang="ts">
  import { onMount } from 'svelte'
  import { fade } from 'svelte/transition'
  import Icon from './Icon.svelte'
  import { onPathDrag } from '../lib/dragdrop'
  import { flow } from '../lib/importflow.svelte'
  import { dur } from '../lib/motion'
  import { app } from '../lib/state.svelte'

  let dragging = $state(false)

  onMount(() => {
    let stop: (() => void) | null = null
    let disposed = false
    void onPathDrag((e) => {
      dragging = e.type === 'enter'
      // 进行中的一批不受拖入影响:不换页、不改列表
      if (e.type !== 'drop' || !flow.canAcceptDrop) return
      app.navigate('import')
      void flow.dropPaths(e.paths)
    }).then((un) => (disposed ? un() : (stop = un)))
    return () => {
      disposed = true
      stop?.()
    }
  })
</script>

{#if dragging}
  <div class="drop" class:blocked={!flow.canAcceptDrop} role="status" transition:fade={{ duration: dur(140) }}>
    <div class="card">
      <Icon name={flow.canAcceptDrop ? 'folder' : 'alert'} size={28} />
      {#if flow.canAcceptDrop}
        <h2>松开以添加待整理文件夹</h2>
        <p>可以拖入文件夹（会查找子文件夹）或单个视频文件。</p>
      {:else}
        <h2>现在不能添加文件夹</h2>
        <p>当前这批还没整理完。完成后或回到导入页的「选择文件夹」再拖入。</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .drop {
    position: fixed; inset: 0; z-index: 90; display: grid; place-items: center; padding: var(--s5);
    background: var(--scrim); backdrop-filter: blur(3px); pointer-events: none;
  }
  .card {
    display: flex; flex-direction: column; align-items: center; gap: var(--s2); text-align: center;
    width: min(420px, 100%); padding: var(--s6) var(--s5); border-radius: var(--r-lg);
    background: var(--surface-1); box-shadow: var(--shadow-3), inset 0 0 0 2px var(--accent);
    color: var(--accent);
  }
  .blocked .card { box-shadow: var(--shadow-3), inset 0 0 0 2px var(--warn); color: var(--warn); }
  h2 { font-size: var(--fs-lg); font-weight: 600; color: var(--text-1); }
  p { color: var(--text-2); font-size: var(--fs-sm); line-height: 1.7; }
</style>
