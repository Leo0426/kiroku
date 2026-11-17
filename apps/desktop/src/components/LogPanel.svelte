<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/state.svelte'
  import type { LogLineEvent } from '../lib/types'

  /** 运行日志面板:新行到来时自动滚到底(用户往上翻时不打扰),可只看警告与错误、复制全部。 */
  let { lines, open = true, live = false }: { lines: LogLineEvent[]; open?: boolean; live?: boolean } = $props()

  let problemsOnly = $state(false)
  let box = $state<HTMLElement>()
  let follow = true
  let expanded = $state(true)
  let page = $state(0)
  const PAGE_SIZE = 200
  $effect(() => { expanded = open })

  const shown = $derived(problemsOnly ? lines.filter((l) => l.level === 'warn' || l.level === 'error') : lines)
  const problems = $derived(lines.filter((l) => l.level === 'warn' || l.level === 'error').length)

  const lastPage = $derived(Math.max(0, Math.ceil(shown.length / PAGE_SIZE) - 1))
  const currentPage = $derived(Math.min(page, lastPage))
  const end = $derived(shown.length - currentPage * PAGE_SIZE)
  const start = $derived(Math.max(0, end - PAGE_SIZE))
  const visible = $derived(shown.slice(start, end))

  function goToPage(next: number) {
    page = next
    follow = next === 0
  }

  function onScroll() {
    if (box) follow = box.scrollHeight - box.scrollTop - box.clientHeight < 24
  }

  $effect(() => {
    // Bound layout work and yield to a frame instead of forcing layout in a microtask.
    void visible
    if (!expanded || currentPage !== 0 || !follow) return
    const frame = requestAnimationFrame(() => {
      if (box) box.scrollTop = box.scrollHeight
    })
    return () => cancelAnimationFrame(frame)
  })

  async function copyAll() {
    const text = lines.map((l) => `${l.time} ${l.level.toUpperCase().padEnd(5)} ${l.target.padEnd(8)} ${l.message}`).join('\n')
    try {
      await navigator.clipboard.writeText(text)
      app.toast('ok', `已复制 ${lines.length} 行日志`)
    } catch {
      app.toast('error', '复制失败')
    }
  }
</script>

<details class="card log" bind:open={expanded}>
  <summary>
    <Icon name={live ? 'loader' : 'info'} size={14} />
    <span>运行日志</span>
    <span class="faint tnum">{lines.length} 行{#if problems} · <span class="warn">{problems} 条警告/错误</span>{/if}</span>
    <span class="grow"></span>
    <label class="only" title="只显示警告与错误">
      <input class="checkbox" type="checkbox" bind:checked={problemsOnly} onchange={() => goToPage(0)} />只看问题
    </label>
    <button class="btn sm ghost" type="button" disabled={!lines.length} onclick={(e) => { e.preventDefault(); copyAll() }}>复制</button>
  </summary>
  {#if expanded}
  <div class="pages">
    <span class="faint tnum">{shown.length ? start + 1 : 0}–{end} / {shown.length} 行</span>
    <button class="btn sm ghost" disabled={currentPage >= lastPage} onclick={() => goToPage(currentPage + 1)}>较早日志</button>
    <button class="btn sm ghost" disabled={currentPage === 0} onclick={() => goToPage(currentPage - 1)}>较新日志</button>
    <button class="btn sm ghost" disabled={currentPage === 0} onclick={() => goToPage(0)}>最新日志</button>
  </div>
  <div class="lines mono" bind:this={box} onscroll={onScroll} role="log" aria-live="off">
    {#each visible as l (l)}
      <div class="line {l.level}"><span class="t">{l.time}</span><span class="tag">{l.target}</span><span class="msg">{l.message}</span></div>
    {:else}
      <div class="line faint">{problemsOnly ? '没有警告或错误' : '暂无日志'}</div>
    {/each}
  </div>
  {/if}
</details>

<style>
  .log { padding: 0; margin-top: var(--s3); font-size: var(--fs-sm); }
  summary { display: flex; align-items: center; gap: var(--s2); padding: var(--s3) var(--s4); cursor: pointer; color: var(--text-2); list-style: none; }
  summary::-webkit-details-marker { display: none; }
  .log[open] summary { border-bottom: 1px solid var(--line); }
  .grow { flex: 1; }
  .warn { color: var(--warn); }
  .only { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-xs); cursor: pointer; }
  .lines { max-height: 320px; overflow: auto; padding: var(--s2) var(--s4) var(--s3); font-size: 12px; line-height: 1.6; }
  .pages { display: flex; align-items: center; flex-wrap: wrap; gap: var(--s2); padding: var(--s2) var(--s4); }
  .line { display: grid; grid-template-columns: 84px 64px minmax(0, 1fr); gap: var(--s3); white-space: pre-wrap; word-break: break-all; }
  .t { color: var(--text-3); flex-shrink: 0; }
  .tag { color: var(--text-3); width: 64px; flex-shrink: 0; }
  .msg { color: var(--text-1); min-width: 0; }
  .debug .msg { color: var(--text-3); }
  .warn .msg { color: var(--warn); }
  .error .msg { color: var(--danger); }
</style>
