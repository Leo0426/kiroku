<script lang="ts">
  import { tick } from 'svelte'
  import { app, type View } from '../lib/state.svelte'
  import { api } from '../lib/api'
  import Icon from './Icon.svelte'
  let dialog: HTMLDialogElement
  let input: HTMLInputElement
  let text = $state('')
  let selected = $state(0)
  const mac = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform)
  const modifier = mac ? '⌘' : 'Ctrl'
  const navigate = (view: View) => { close(); app.navigate(view) }
  const actions = [
    { label: '打开媒体库', keywords: 'library browse meitiku', hint: `${modifier} 1`, run: () => navigate('library') },
    { label: '导入整理', keywords: 'import scan daoru', hint: `${modifier} 2`, run: () => navigate('import') },
    { label: '查看历史与撤销', keywords: 'history undo lishi', hint: `${modifier} 3`, run: () => navigate('history') },
    { label: '打开设置', keywords: 'settings preferences shezhi', hint: `${modifier} 4`, run: () => navigate('settings') },
    { label: '打开运行日志文件夹', keywords: 'logs debug rizhi', hint: '', run: async () => { close(); try { await api.openLogDir() } catch (e) { app.toast('error', String(e), 5000) } } },
  ]
  const matches = $derived(actions.filter(a => text.trim().toLowerCase().split(/\s+/).every(w => `${a.label} ${a.keywords}`.toLowerCase().includes(w))))
  $effect(() => { text; selected = 0 })
  $effect(() => {
    if (app.commandsOpen && dialog && !dialog.open) { text = ''; selected = 0; dialog.showModal(); void tick().then(() => input?.focus()) }
    else if (!app.commandsOpen && dialog?.open) dialog.close()
  })
  function close() { app.commandsOpen = false }
  function globalKey(e: KeyboardEvent) {
    if (e.isComposing || e.defaultPrevented || app.dialog || app.needsOnboarding) return
    const editable = (e.target as HTMLElement | null)?.closest('input, textarea, select, [contenteditable="true"]')
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
      e.preventDefault(); app.commandsOpen = !app.commandsOpen
    } else if (!editable && !app.commandsOpen && !document.querySelector('dialog[open]') && (e.metaKey || e.ctrlKey) && !e.altKey && /^[1-4]$/.test(e.key)) {
      e.preventDefault(); navigate((['library', 'import', 'history', 'settings'] as View[])[Number(e.key) - 1])
    } else if (!editable && e.key === '?' && !e.metaKey && !e.ctrlKey && !document.querySelector('[aria-label="影片详情"], dialog[open]')) {
      e.preventDefault(); app.commandsOpen = true
    }
  }
  function key(e: KeyboardEvent) {
    e.stopPropagation()
    if (e.isComposing) return
    if (e.key === 'Tab') {
      const focusable = [...dialog.querySelectorAll<HTMLElement>('input:not([disabled]), button:not([disabled]):not([tabindex="-1"])')]
      const index = focusable.indexOf(document.activeElement as HTMLElement)
      if (e.shiftKey && index <= 0) { e.preventDefault(); focusable.at(-1)?.focus() }
      else if (!e.shiftKey && index === focusable.length - 1) { e.preventDefault(); focusable[0]?.focus() }
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); close() }
    if (e.key === 'Escape') { e.preventDefault(); close() }
    if (e.target === input && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault(); selected = matches.length ? (selected + (e.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length : 0
    }
    if (e.target === input && e.key === 'Enter') { e.preventDefault(); void matches[selected]?.run() }
  }
</script>

<svelte:window onkeydown={globalKey}/>
<dialog bind:this={dialog} aria-label="快捷操作" oncancel={close} onclose={() => { if (!dialog.open) close() }} onkeydown={key} onclick={e => { if (e.target === dialog) { const r = dialog.getBoundingClientRect(); if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) close() } }}>
  <div class="search"><Icon name="search" size={18}/><input bind:this={input} bind:value={text} aria-label="搜索操作" placeholder="搜索操作…" role="combobox" aria-expanded="true" aria-controls="command-results" aria-activedescendant={matches[selected] ? `command-${selected}` : undefined} autocomplete="off"/><button class="btn sm ghost" aria-label="关闭快捷操作" onclick={close}>Esc</button></div>
  <div role="listbox" aria-label="操作" id="command-results">
    {#each matches as action, i}<div role="option" aria-selected={selected === i} id="command-{i}"><button tabindex="-1" onclick={action.run} onpointermove={() => selected = i}><span>{action.label}</span><kbd>{action.hint}</kbd></button></div>{/each}
    {#if !matches.length}<p class="no-results">没有匹配的操作，试试「媒体库」或「设置」。</p>{/if}
  </div>
  <footer><span>↑ ↓ 选择 · Enter 执行 · Esc 关闭</span><span>{modifier} K 快捷操作 · / 搜索媒体库 · ? 帮助</span></footer>
</dialog>

<style>
  dialog { margin:clamp(24px,14vh,160px) auto 0; padding:0; width:min(560px,calc(100vw - 32px)); max-height:calc(100dvh - 48px); overflow:auto; border:1px solid var(--line-strong); border-radius:var(--r-lg); color:var(--text-1); background:var(--surface-1); box-shadow:var(--shadow-3); }
  dialog::backdrop { background:var(--scrim); backdrop-filter:blur(5px); }
  .search { display:flex; align-items:center; gap:12px; padding:16px; border-bottom:1px solid var(--line); }.search input { flex:1; width:0; min-width:0; background:none; border:0; color:inherit; font:inherit; outline:none; }
  [role="listbox"] { padding:8px; }[role="option"] button { display:flex; align-items:center; justify-content:space-between; gap:12px; width:100%; min-height:46px; padding:10px 12px; border:0; border-radius:var(--r-sm); background:none; color:inherit; text-align:left; }
  [aria-selected="true"] button { background:var(--accent-soft); color:var(--accent); }kbd { white-space:nowrap; color:var(--text-2); font-size:var(--fs-xs); }
  footer { display:flex; flex-direction:column; gap:5px; padding:12px 20px; border-top:1px solid var(--line); color:var(--text-2); font-size:var(--fs-xs); }.no-results { padding:20px; color:var(--text-2); }
</style>
