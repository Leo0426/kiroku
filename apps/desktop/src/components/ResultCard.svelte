<script lang="ts">
  import { fmtDuration } from '../lib/format'
  import { SOURCE_NAMES, type Preview } from '../lib/types'
  import Icon from './Icon.svelte'

  /** 刮削时实时出现的结果卡:横版封面只露出右侧(正面)部分,像海报 */
  let { code, p }: { code: string; p: Preview } = $props()
  let loaded = $state(false)
  let failed = $state(false)
  $effect(() => { p.cover; loaded = false; failed = false })
</script>

<article class="rc">
  <div class="cover" class:landscape={p.coverIsLandscape}>
    {#if p.cover && !failed}
      {#key p.cover}
      <img class:ready={loaded} src={p.cover} alt="" loading="lazy" referrerpolicy="no-referrer" onload={() => (loaded = true)} onerror={() => (failed = true)} />
      {/key}
    {/if}
    {#if !loaded}<div class="ph mono" class:skeleton-fill={!!p.cover && !failed}>{failed ? '图片加载失败' : !p.cover ? '暂无封面' : code}</div>{/if}
    {#if p.score !== null}<span class="score tnum"><Icon name="star" size={10} />{(p.score / 10).toFixed(1)}</span>{/if}
    <span class="check" aria-hidden="true"><Icon name="check" size={12} /></span>
  </div>
  <div class="body">
    <span class="code mono">{code}</span>
    <h4 title={p.originalTitle ?? ''}>{p.title ?? p.originalTitle ?? code}</h4>
    <p class="meta">
      {#if p.releaseDate}<span>{p.releaseDate}</span>{/if}
      {#if p.runtimeMin}<span>{fmtDuration(p.runtimeMin * 60)}</span>{/if}
      {#if p.studio}<span>{p.studio}</span>{/if}
    </p>
    {#if p.actors.length}<p class="actors">{p.actors.join('、')}</p>{/if}
    <p class="src">{p.sources.map((s) => SOURCE_NAMES[s] ?? s).join(' · ')}</p>
  </div>
</article>

<style>
  .rc {
    display: flex; gap: var(--s3); padding: var(--s3); border-radius: var(--r-md); background: var(--surface-1); min-width: 0;
    animation: cardIn 520ms var(--ease-back) both;
    transition: transform var(--d-base) var(--ease-back), background var(--d-fast);
  }
  .rc:hover { transform: translateY(-3px); background: var(--surface-2); }
  @keyframes cardIn { from { opacity: 0; transform: translateY(12px) scale(0.94); } }
  .cover { position: relative; flex-shrink: 0; width: 72px; aspect-ratio: var(--poster-ratio); border-radius: var(--r-sm); overflow: hidden; background: var(--surface-3); }
  img { width: 100%; height: 100%; object-fit: cover; display: block; opacity: 0; transform: scale(1.06); transition: opacity var(--d-slow) var(--ease-out), transform 900ms var(--ease-out); }
  /* 横版封面(封底|封面):只露出右侧的正面 */
  .landscape img { object-position: right center; }
  img.ready { opacity: 1; transform: scale(1); }
  .rc:hover img.ready { transform: scale(1.05); }
  .ph { position: absolute; inset: 0; display: grid; place-items: center; font-size: 10px; color: var(--text-3); }
  .score { position: absolute; top: 4px; left: 4px; display: inline-flex; align-items: center; gap: 2px; padding: 1px 5px; border-radius: var(--r-pill); font-size: 10px; font-weight: 600; color: var(--accent-hover); background: rgb(12 12 16 / 0.62); }
  .check {
    position: absolute; right: 4px; bottom: 4px; width: 18px; height: 18px; border-radius: 50%; display: grid; place-items: center;
    background: var(--ok); color: #0b0b0e; animation: pop 420ms 220ms var(--ease-back) both;
  }
  .body { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .code { color: var(--accent); font-weight: 600; font-size: var(--fs-xs); }
  h4 { margin: 0; font-size: var(--fs-sm); font-weight: 600; line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  p { margin: 0; font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta span + span::before { content: '·'; margin: 0 5px; color: var(--text-3); }
  .src { color: var(--text-3); margin-top: auto; }
  @media (prefers-reduced-motion: reduce) { .rc, .check { animation: none; } }
</style>
