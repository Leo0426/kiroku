<script lang="ts">
  import { flip } from 'svelte/animate'
  import { scale } from 'svelte/transition'
  import { dur } from '../lib/motion'
  /** 有序站点列表:↑↓ 调整顺序,× 移除,下拉添加。就地修改传入数组。 */
  let { list, all, name, empty = '' }: { list: string[]; all: string[]; name: (id: string) => string; empty?: string } = $props()

  const rest = $derived(all.filter((x) => !list.includes(x)))

  function move(i: number, d: -1 | 1) {
    const j = i + d
    if (j < 0 || j >= list.length) return
    ;[list[i], list[j]] = [list[j], list[i]]
  }
</script>

<div class="chips">
  {#each list as id, i (id)}
    <span class="chip" animate:flip={{ duration: dur(260) }} in:scale={{ start: 0.6, duration: dur(220) }} out:scale={{ start: 0.6, duration: dur(160) }}>
      <b class="tnum">{i + 1}</b>
      {name(id)}
      <button aria-label="上移 {name(id)}" onclick={() => move(i, -1)} disabled={i === 0}>↑</button>
      <button aria-label="下移 {name(id)}" onclick={() => move(i, 1)} disabled={i === list.length - 1}>↓</button>
      <button aria-label="移除 {name(id)}" onclick={() => list.splice(i, 1)}>×</button>
    </span>
  {/each}
  {#if list.length === 0 && empty}<span class="faint empty">{empty}</span>{/if}
  {#if rest.length}
    <select
      class="add"
      aria-label="添加站点"
      value=""
      onchange={(e) => {
        if (e.currentTarget.value) list.push(e.currentTarget.value)
        e.currentTarget.value = ''
      }}
    >
      <option value="">＋ 添加</option>
      {#each rest as id (id)}<option value={id}>{name(id)}</option>{/each}
    </select>
  {/if}
</div>

<style>
  .chips { display: flex; flex-wrap: wrap; gap: var(--s2); align-items: center; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; padding: 3px 4px 3px 10px;
    background: var(--surface-3); border-radius: var(--r-pill); font-size: var(--fs-sm);
    transition: background var(--d-fast), transform var(--d-base) var(--ease-back);
  }
  .chip:hover { background: var(--surface-hover); transform: translateY(-1px); }
  .chip b { color: var(--accent); font-weight: 600; }
  .chip button {
    padding: 0 7px; border: 0; background: transparent; border-radius: var(--r-pill); line-height: 22px; color: var(--text-2);
    transition: background var(--d-fast) var(--ease-out), color var(--d-fast) var(--ease-out), transform var(--d-base) var(--ease-back);
  }
  .chip button:hover:not(:disabled) { background: var(--surface-3); color: var(--text-1); transform: scale(1.15); }
  .chip button:active:not(:disabled) { transform: scale(0.85); }
  .chip button:disabled { opacity: 0.3; }
  .empty { font-size: var(--fs-sm); }
  .add {
    border: 1px dashed var(--line-strong); border-radius: var(--r-pill); padding: 3px 8px; background: transparent;
    color: var(--text-2); font-size: var(--fs-sm);
  }
  .add:hover { border-color: var(--accent); color: var(--accent); }
</style>
