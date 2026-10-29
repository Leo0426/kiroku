<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import InfoTip from '../components/InfoTip.svelte'
  import CodeEditor from '../components/CodeEditor.svelte'
  import LogPanel from '../components/LogPanel.svelte'
  import Count from '../components/Count.svelte'
  import ProgressBar from '../components/ProgressBar.svelte'
  import ResultCard from '../components/ResultCard.svelte'
  import StatusPill from '../components/StatusPill.svelte'
  import { presetSettingsTab } from './Settings.svelte'
  import { api } from '../lib/api'
  import { baseName, fmtBytes, fmtDuration, shortPath } from '../lib/format'
  import { flow } from '../lib/importflow.svelte'
  import { dur, rise } from '../lib/motion'
  import { scale } from 'svelte/transition'
  import { app } from '../lib/state.svelte'
  import { CONTENT_TYPE_LABEL, SOURCE_NAMES, isVideoPath, type OrganizeMode, type ScanItem, type ScrapeStatus, type SkipReason } from '../lib/types'

  const STEPS = ['选择文件夹', '识别文件', '查找信息', '确认整理', '完成']
  const stepIndex = $derived(({ source: 0, scan: 1, scrape: 2, plan: 3, apply: 3, done: 4 } as const)[flow.step])
  const MAX_ROWS = 300

  onMount(() => flow.seedDirs())

  const root = $derived(app.settings?.library_root ?? '')
  const MODE: Record<OrganizeMode, { summary: string; progress: string }> = {
    move: { summary: '视频和字幕会移动到媒体库，原位置不再保留。', progress: '移动视频与字幕' },
    copy: { summary: '复制视频和字幕到媒体库，保留源文件。', progress: '复制视频与字幕' },
    hardlink: { summary: '创建硬链接，保留源文件；需要位于同一磁盘分区，修改任一文件会影响另一文件。', progress: '创建视频与字幕的硬链接' },
    symlink: { summary: '创建软链接，保留源文件；播放时仍需要访问源文件。', progress: '创建视频与字幕的软链接' },
  }
  const mode = $derived(app.settings?.organize_mode ?? 'move')
  let scanOpen = $state(false)
  /** 正在为哪一部指定详情页地址 */
  let urlFor = $state<string | null>(null)
  let urlText = $state('')
  let foundOpen = $state(false)
  let statusesOpen = $state(false)
  $effect(() => {
    if (flow.step !== 'scan') scanOpen = false
    if (flow.step !== 'scrape') { foundOpen = false; statusesOpen = false }
  })
  const issueCodes = $derived(flow.uniqueCodes.filter(code => {
    const kind = flow.statuses[code]?.kind
    return !!flow.notes[code] || kind === 'failed' || kind === 'notFound' || kind === 'noRoute' || kind === 'cancelled'
      || (!flow.scrape.running && kind !== 'found')
  }))
  const activeCodes = $derived(flow.uniqueCodes.filter(code => ['scraping', 'translating'].includes(flow.statuses[code]?.kind ?? '')).slice(0, 3))
  const rel = (p: string) => (root && p.startsWith(root) ? p.slice(root.length).replace(/^[\\/]/, '') : p)
  const parentOf = (p: string) => p.slice(0, Math.max(0, p.length - baseName(p).length)).replace(/[\\/]$/, '')

  function fillCookie() {
    presetSettingsTab('sources')
    app.navigate('settings')
  }

  function statusView(s: ScrapeStatus | undefined): { tone: 'neutral' | 'info' | 'ok' | 'warn' | 'danger' | 'accent'; label: string; busy: boolean; title?: string } {
    switch (s?.kind) {
      // 同一部影片的多个站点并发查询,site 为逗号分隔的站点 id
      case 'scraping': return { tone: 'info', label: `正在查询 ${s.site.split(',').map((id) => SOURCE_NAMES[id] ?? id).join('、')}`, busy: true }
      case 'translating': return { tone: 'accent', label: 'AI 汉化中', busy: true }
      case 'found': return { tone: 'ok', label: '已找到', busy: false }
      case 'notFound': return { tone: 'warn', label: '未找到信息', busy: false }
      case 'noRoute': return { tone: 'warn', label: '无可用数据源', busy: false }
      case 'failed': return { tone: 'danger', label: '查询失败', busy: false, title: s.message }
      case 'cancelled': return { tone: 'neutral', label: '已取消', busy: false }
      default: return { tone: 'neutral', label: flow.scrape.running ? '等待查询' : '未完成', busy: false }
    }
  }

  const SKIP_LABEL: Record<SkipReason, string> = { extra: '样片/预告', blacklisted: '黑名单', tooSmall: '过小' }
  const skipped = $derived(flow.scanSummary?.skipped ?? [])
  const skippedTotal = $derived(flow.scanSummary?.skippedTotal ?? 0)
  const skippedSummary = $derived(
    (Object.keys(SKIP_LABEL) as SkipReason[])
      .map((r) => [SKIP_LABEL[r], skipped.filter((f) => f.reason === r).length] as const)
      .filter(([, n]) => n > 0)
      .map(([l, n]) => `${l} ${n}`)
      .join('、'),
  )
  const MOSAIC_LABEL: Record<string, string> = { uncensored: '无码', cracked: '破解', leaked: '流出' }
  function tags(it: ScanItem): { text: string; tone: 'neutral' | 'accent' | 'info' }[] {
    const out: { text: string; tone: 'neutral' | 'accent' | 'info' }[] = []
    if (it.code && it.code.content_type !== 'censored') out.push({ text: CONTENT_TYPE_LABEL[it.code.content_type] ?? it.code.content_type, tone: 'info' })
    if (it.info.cd) out.push({ text: `CD${it.info.cd}`, tone: 'neutral' })
    if (it.info.has_subtitle) out.push({ text: '中字', tone: 'accent' })
    if (it.info.definition) out.push({ text: it.info.definition, tone: 'neutral' })
    const m = it.info.mosaic && MOSAIC_LABEL[it.info.mosaic]
    if (m) out.push({ text: m, tone: 'neutral' })
    // 类型与马赛克可能给出同一个词(如 HEYZO 的「无码」),去重以免重复 key
    return out.filter((t, i) => out.findIndex((x) => x.text === t.text) === i)
  }

  const counts = $derived.by(() => {
    const c = { found: 0, notFound: 0, failed: 0 }
    for (const code of flow.uniqueCodes) {
      const k = flow.statuses[code]?.kind
      if (k === 'found') c.found++
      else if (k === 'notFound' || k === 'noRoute') c.notFound++
      else if (k === 'failed') c.failed++
    }
    return c
  })

  // 完成后撤销本批
  let confirmUndo = $state(false)
  let undone = $state(false)
  let undoing = $state(false)
  $effect(() => {
    if (flow.step !== 'done') {
      confirmUndo = false
      undone = false
    }
  })
  async function undo() {
    const id = flow.applyDone?.journalId
    if (!id) return
    undoing = true
    try {
      const r = await api.rollback(id)
      if (r.indexError) app.toast('error', `文件撤销结果已记录，但索引更新失败：${r.indexError}`, 6000)
      undone = r.status === 'rolledBack' && r.failures.length === 0
      app.toast(r.failures.length ? 'error' : 'ok', r.failures.length ? `部分撤销失败：${r.failures[0]}` : '已撤销本批整理', 5000)
    } catch (e) {
      app.toast('error', String(e), 5000)
    } finally {
      undoing = false
      confirmUndo = false
    }
  }

  const applyTail = $derived(flow.applyLog.slice(-8).reverse())
</script>

{#snippet scanRows(items: ScanItem[])}
  {#each items.slice(0, MAX_ROWS) as it (it.id)}
    <div class="trow scanrow">
      <div class="cell file"><span class="name" title={it.path}>{baseName(it.path)}</span><span class="faint sub">{shortPath(parentOf(it.path), 60)}</span></div>
      <div class="cell"><CodeEditor code={it.code?.id ?? null} error={flow.codeErrors[it.id]} onsubmit={(t) => flow.setCode(it.id, t)} /></div>
      <div class="cell tagcell">{#each tags(it) as t (t.text)}<StatusPill tone={t.tone}>{t.text}</StatusPill>{/each}</div>
      <div class="cell size tnum faint">{fmtBytes(it.size)}</div>
    </div>
  {/each}
  {#if items.length > MAX_ROWS}<p class="more faint">还有 {items.length - MAX_ROWS} 个文件未列出</p>{/if}
{/snippet}

{#snippet statusRows(codes: string[])}
  {#each codes as code (code)}
    {@const v = statusView(flow.statuses[code])}
    <div class="trow scraperow">
      <span class="mono code">{code}</span>
      {#key v.label}<span class="pillwrap" in:scale={{ start: 0.7, duration: dur(260) }}><StatusPill tone={v.tone} busy={v.busy} title={v.title}>{v.label}</StatusPill></span>{/key}
      {#if v.title}<span class="faint err" title={v.title}>{v.title}</span>
        {#if v.title.includes('Cookie')}<button class="btn sm" onclick={fillCookie}><Icon name="pencil" size={13} />设置数据源</button>{/if}
      {/if}
      {#if flow.notes[code]}<span class="note-warn err" title={flow.notes[code]}>{flow.notes[code]}</span>{/if}
      {#if !flow.busy && urlFor !== code}
        <button class="btn sm ghost" type="button" aria-label="为 {code} 指定详情页" title="搜索没找到或找错时，用某个站点的详情页地址刮削" onclick={() => { urlFor = code; urlText = '' }}>指定详情页</button>
      {/if}
      {#if urlFor === code}
        <form class="urlform" onsubmit={(e) => { e.preventDefault(); const url = urlText; urlFor = null; void flow.scrapeUrl(code, url) }}>
          <input class="input sm grow" type="url" placeholder="https://javdb.com/v/…" aria-label="{code} 详情页地址" bind:value={urlText} />
          <button class="btn sm" type="submit" disabled={!urlText.trim() || flow.busy}>用详情页刮削</button>
          <button class="btn sm ghost" type="button" onclick={() => (urlFor = null)}>取消</button>
        </form>
      {/if}
    </div>
  {/each}
{/snippet}

<div class="page" in:rise>
  <header>
    <h1>导入整理</h1>
    <ol class="stepper" aria-label="进度">
      {#each STEPS as label, i (label)}
        <li class:passed={i < stepIndex} class:cur={i === stepIndex} aria-current={i === stepIndex ? 'step' : undefined}>
          <span class="dot">{#if i < stepIndex}<Icon name="check" size={12} />{:else}{i + 1}{/if}</span>
          <span class="lbl">{label}</span>
        </li>
      {/each}
    </ol>
  </header>

  {#if flow.error}<div class="alert" role="alert"><Icon name="alert" size={16} />{flow.error}</div>{/if}

  {#key flow.step}
    <div class="panel stagger" in:rise={{ duration: 300, y: 10 }}>
      <!-- 1. 选择来源 -->
      {#if flow.step === 'source'}
        <section class="card pad" aria-labelledby="source-dirs-title">
          <div class="titled">
            <h2 id="source-dirs-title">待整理的文件夹</h2>
            <InfoTip label="待整理的文件夹说明">
              <p>选择存放视频的文件夹，也会查找其中的子文件夹；也可以把文件夹或单个视频拖进窗口。单个视频只导入它自己，不清理它所在的文件夹。</p>
              <p><b>支持的视频格式</b><br /><span class="mono">mp4 / mkv / avi / wmv / mov / ts / m2ts / flv / rmvb</span></p>
            </InfoTip>
          </div>
          {#each flow.dirs as d (d)}
            <div class="dirrow">
              <Icon name={isVideoPath(d) ? 'film' : 'folder'} size={16} /><span class="mono grow" title={d}>{d}</span>
              {#if isVideoPath(d)}<span class="faint">单个文件</span>{/if}
              <button class="btn ghost icon sm" aria-label="移除 {d}" onclick={() => flow.removeDir(d)}><Icon name="x" size={14} /></button>
            </div>
          {/each}
          <button class="btn" onclick={() => flow.pickDir()}><Icon name="plus" size={15} />添加文件夹</button>
        </section>

        <section class="card pad" aria-label="整理目标">
          <div class="titled">
            <h2>整理到</h2>
            <InfoTip label="整理方式说明">
              <p>{MODE[mode].summary} 影片信息和海报会保存在媒体库。</p>
              <p><b>命名规则</b><br /><span class="mono">{app.settings?.dir_template}/{app.settings?.plex.naming ? '(Plex 命名)' : app.settings?.file_template}</span></p>
              <p>可在「设置 → 命名与 NFO」修改。</p>
            </InfoTip>
          </div>
          <div class="dirrow"><Icon name="folder" size={16} /><span class="mono grow" title={root}>{root}</span></div>
        </section>

        <footer>
          <span></span>
          <button class="btn primary" disabled={flow.dirs.length === 0} onclick={() => flow.startScan()}>开始扫描<Icon name="arrow-right" size={15} /></button>
        </footer>

        <!-- 2. 扫描 -->
      {:else if flow.step === 'scan'}
        <section class="card summary">
          <div class="stat"><b><Count value={flow.items.length} /></b><span>视频文件</span></div>
          <div class="stat"><b><Count value={flow.uniqueCodes.length} /></b><span>个番号</span></div>
          <div class="stat" class:warnc={flow.unrecognized.length > 0}><b><Count value={flow.unrecognized.length} /></b><span>未识别</span></div>
          {#if skippedTotal > 0}<div class="stat"><b><Count value={skippedTotal} /></b><span>已跳过</span></div>{/if}
          <span class="grow"></span>
          {#if flow.scanRunning}
            <span class="live"><Icon name="loader" size={14} />扫描中…</span>
            <button class="btn sm" onclick={() => flow.cancel()}>停止</button>
          {/if}
        </section>

        {#if flow.scanSummary?.errorsTotal}
          <section class="card" role="alert"><p>部分路径未能读取，扫描结果不完整。已读到的文件仍可处理。</p>
            {#each flow.scanSummary.errors as issue}<p>{issue.path}：{issue.message}</p>{/each}
            {#if flow.scanSummary.errorsTotal > flow.scanSummary.errors.length}<p>另有 {flow.scanSummary.errorsTotal - flow.scanSummary.errors.length} 处读取错误。</p>{/if}
          </section>
        {/if}
        {#if flow.scanSummary?.cancelled}<p class="hint">扫描已停止，仅显示已读到的文件。</p>{/if}

        {#if flow.unrecognized.length}
          <section class="card" aria-label="需要确认的文件">
            <div class="section-head"><h2>这些文件还没识别出番号</h2><p class="hint">可以填写番号，也可以先跳过，继续处理已识别的文件。</p></div>
            <div class="table">{@render scanRows(flow.unrecognized)}</div>
          </section>
        {:else if !flow.scanRunning && flow.items.length}
          <p class="hint">所有文件都已识别，可以继续查找影片信息。</p>
        {/if}
        {#if flow.recognized.length}
          <details class="card disclosure" bind:open={scanOpen}>
            <summary>查看已识别文件 · {flow.recognized.length} 个</summary>
            {#if scanOpen}<section class="table" aria-label="扫描结果">{@render scanRows(flow.recognized)}</section>{/if}
          </details>
        {:else if !flow.scanRunning && flow.items.length === 0 && !flow.scanSummary?.errorsTotal && !flow.scanSummary?.cancelled}
          <p class="hint">没有找到视频文件。请选择其他文件夹，或检查扫描过滤规则。</p>
        {/if}

        {#if skippedTotal > 0}
          <details class="card skipped">
            <summary>
              <Icon name="info" size={14} />
              <span>已跳过 {skippedTotal} 个文件 · {skippedSummary}</span>
            </summary>
            <ul>
              {#each skipped as f (f.path)}
                <li>
                  <span class="name" title={f.path}>{baseName(f.path)}</span>
                  <StatusPill tone="neutral">{SKIP_LABEL[f.reason]}</StatusPill>
                  <span class="faint tnum">{fmtBytes(f.size)}</span>
                </li>
              {/each}
              {#if skippedTotal > skipped.length}<li class="faint">还有 {skippedTotal - skipped.length} 个未列出</li>{/if}
            </ul>
            <p class="hint">规则可在「设置 → 扫描」修改。</p>
          </details>
        {/if}

        <footer>
          <button class="btn ghost" disabled={flow.busy} onclick={() => flow.reset()}><Icon name="arrow-left" size={15} />重新选择</button>
          <div class="right">
            {#if flow.unrecognized.length > 0 && !flow.scanRunning}<span class="faint note">未识别的 {flow.unrecognized.length} 个文件会被跳过</span>{/if}
            <button class="btn primary" disabled={flow.scanRunning || flow.recognized.length === 0} onclick={() => flow.startScrape()}>
              查找影片信息（{flow.uniqueCodes.length} 部）<Icon name="arrow-right" size={15} />
            </button>
          </div>
        </footer>

        <!-- 3. 刮削 -->
      {:else if flow.step === 'scrape'}
        <section class="card pad">
          <div class="prog-head">
            <h2>{flow.scrape.running ? '正在查找影片信息' : flow.scrapeSummary?.cancelled ? '已停止' : '影片信息查询完成'}</h2>
            <span class="tnum muted">{flow.scrape.done} / {flow.scrape.total}</span>
          </div>
          <ProgressBar value={flow.scrape.total ? flow.scrape.done / flow.scrape.total : 0} active={flow.scrape.running} label="影片信息查询进度" />
          <div class="prog-foot">
            <span class="faint">
              {#if flow.scrape.running}{flow.eta !== null ? `预计还需 ${fmtDuration(flow.eta)}` : '正在估算剩余时间…'}{:else}找到 {counts.found} · 未找到 {counts.notFound} · 失败 {counts.failed}{/if}
            </span>
            {#if flow.scrape.running}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
          </div>
          <p class="hint">{flow.scrape.running ? '可以切换到其他页面，查询会继续进行。' : flow.foundIds.length ? '找到信息的影片可以继续预览整理，其余影片可重试或暂时跳过。' : '还没有可整理的影片，请重试查询或返回检查番号。'}</p>
        </section>

        {#if flow.scrape.running && activeCodes.length}
          <section class="card table" aria-label="正在查询的影片">{@render statusRows(activeCodes)}</section>
        {/if}
        {#if issueCodes.length}
          <section class="card" aria-label="需要处理的影片">
            <div class="section-head"><div class="section-title"><h2>需要处理 · {issueCodes.length} 部</h2>
              {#if !flow.scrape.running && flow.retryCodes.length}<button class="btn sm" onclick={() => flow.startScrape(true)}>重试未完成（{flow.retryCodes.length} 部）</button>{/if}
            </div><p class="hint">查询失败或未找到信息的影片会跳过；已找到信息的影片仍可继续整理。</p></div>
            <div class="table">{@render statusRows(issueCodes)}</div>
          </section>
        {/if}
        {#if flow.found.length}
          <details class="card disclosure" bind:open={foundOpen}>
            <summary>查看已找到的影片 · {flow.found.length} 部</summary>
            <section class="results" aria-label="已找到的影片">
              {#if foundOpen}<div class="grid">{#each flow.found as f (f.code)}<ResultCard code={f.code} p={f.preview} />{/each}</div>{/if}
            </section>
          </details>
        {/if}
        <details class="card disclosure" bind:open={statusesOpen}>
          <summary>查看全部查询状态</summary>
          {#if statusesOpen}<section class="table" aria-label="全部查询状态">{@render statusRows(flow.uniqueCodes)}</section>{/if}
        </details>

        <footer>
          <button class="btn ghost" disabled={flow.busy} onclick={() => (flow.step = 'scan')}><Icon name="arrow-left" size={15} />上一步</button>
          <button class="btn primary" disabled={flow.scrape.running || flow.planning || flow.foundIds.length === 0} onclick={() => flow.buildPlan()}>
            {flow.planning ? '正在准备预览…' : '预览整理'}<Icon name="arrow-right" size={15} />
          </button>
          {#if flow.planning}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
        </footer>

        <!-- 4. 整理计划 -->
      {:else if flow.step === 'plan'}
        <section class="card summary">
          <div class="stat"><b><Count value={flow.selectedCount} /></b><span>将整理的文件</span></div>
          {#if flow.conflictCount}<div class="stat warnc"><b><Count value={flow.conflictCount} /></b><span>有冲突，跳过</span></div>{/if}
          {#if flow.uniqueCodes.length > counts.found}<div class="stat"><b><Count value={flow.uniqueCodes.length - counts.found} /></b><span>未找到信息</span></div>{/if}
          <span class="grow"></span>
          <button class="btn sm ghost" onclick={() => flow.setAll(true)}>全选</button>
          <button class="btn sm ghost" onclick={() => flow.setAll(false)}>全不选</button>
        </section>

        <section class="card table" aria-label="整理计划">
          {#each flow.plans as p (p.itemId)}
            <label class="trow planrow" class:off={!flow.picked[p.itemId]} class:conflict={p.conflict}>
              <input class="checkbox" type="checkbox" disabled={!!p.conflict} bind:checked={flow.picked[p.itemId]} aria-label="整理 {p.code}" />
              <div class="cell head">
                <span class="mono code">{p.code}</span>
                <span class="ttl">{p.title ?? ''}</span>
              </div>
              <div class="cell paths">
                <span class="faint from" title={p.from}>{baseName(p.from)}</span>
                <span class="mono to" title={p.to}>→ {rel(p.to)}</span>
                {#each p.conflicts ?? [] as c (c.path)}<span class="mono conflict-path" title={c.path}>{c.kind === 'invalidLibrary' ? '库不可用' : c.kind === 'targetExists' ? '已占用' : '本批重复'}：{rel(c.path)}</span>{/each}
              </div>
              <div class="cell end">
                {#if p.conflict}
                  <StatusPill tone="warn" title={p.notices?.join('\n') || (p.conflicts ?? []).map((c) => c.path).join('\n') || p.to}>{p.conflict === 'invalidLibrary' ? '请先检查媒体库' : p.conflict === 'targetExists' ? '目标已存在' : '批内重名'}</StatusPill>

                {:else if p.newVersion}
                  <StatusPill tone="info" title={p.notices?.join('\n') ?? ''}>新版本 · 需手动勾选</StatusPill>
                {:else if p.notices?.length}
                  <span class="faint extras" title={p.notices.join('\n')}>{p.notices[0]}</span>
                {:else if p.extras.length}
                  <span class="faint extras" title={p.extras.join('\n')}>+ {p.extras.length} 个文件</span>
                {/if}
              </div>
            </label>
          {/each}
        </section>

        <footer>
          <button class="btn ghost" onclick={() => (flow.step = 'scrape')}><Icon name="arrow-left" size={15} />上一步</button>
          <div class="right">
            <span class="faint note" role="note" aria-label="文件处理方式">{MODE[mode].summary} 可在「历史」中撤销。{#if mode === 'move' && app.settings?.cleanup_source}空出来的原文件夹会移到废纸篓。{/if}</span>
            <button class="btn primary" disabled={flow.selectedCount === 0} onclick={() => flow.startApply()}>确认整理（{flow.selectedCount} 个文件）</button>
          </div>
        </footer>

        <!-- 4b. 执行中 -->
      {:else if flow.step === 'apply'}
        {@const downloading = flow.applyProgress?.phase === 'images'}
        {@const indexing = flow.applyProgress?.phase === 'index'}
        {@const progress = downloading ? flow.applyProgress! : flow.apply}
        <section class="card pad">
          <div class="prog-head">
            <h2>{flow.apply.running ? (downloading ? '正在下载图片' : indexing ? '正在更新媒体库' : '正在整理') : '整理已结束'}</h2>
            <span class="tnum muted">{progress.done} / {progress.total}{downloading ? ' 张已处理' : ''}</span>
          </div>
          <ProgressBar value={progress.total ? progress.done / progress.total : 0} active={flow.apply.running} label={downloading ? '图片下载进度' : '整理进度'} />
          <div class="prog-foot">
            {#if downloading}
              <span class="faint" role="status" aria-label="图片下载状态">下载海报与剧照{flow.applyProgress?.code ? ` · ${flow.applyProgress.code}` : ''}{flow.applyProgress?.failed ? ` · ${flow.applyProgress.failed} 张未下载成功` : ''}</span>
            {:else}
              <span class="faint" role="status" aria-label="当前整理方式">{indexing ? '正在更新影片索引' : `${MODE[flow.applyMode].progress}，保存影片信息与海报`}</span>
            {/if}
            {#if flow.apply.running}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
          </div>
        </section>
        <section class="card table" aria-label="整理日志">
          {#each applyTail as e (e.itemId)}
            <div class="trow logrow" class:bad={!e.ok}>
              <Icon name={e.ok ? 'check' : 'alert'} size={14} />
              <span class="mono code">{e.code}</span>
              <span class="faint err">{e.error ?? (e.warnings.length ? `完成,但有 ${e.warnings.length} 条警告` : '完成')}</span>
            </div>
          {/each}
        </section>
        {#if app.settings?.log.app}<LogPanel lines={flow.logLines} live={flow.apply.running} />{/if}

        <!-- 5. 完成 -->
      {:else}
        {@const d = flow.applyDone}
        <section class="card done">
          <div class="big-icon" class:warn={!!d && (d.failed.length > 0 || d.cancelled)}><Icon name={d && (d.failed.length || d.cancelled) ? 'alert' : 'check'} size={30} /></div>
          <h2>{d?.cancelled ? '已停止' : '整理完成'}</h2>
          {#if d?.indexError}<p role="alert">文件整理结果已记录，但索引更新失败：{d.indexError}。请到设置重建索引。</p>{/if}
          <p class="muted">成功整理 <b class="tnum">{d?.applied ?? 0}</b> 个文件{#if d && d.failed.length}，失败 <b class="tnum">{d.failed.length}</b> 个{/if}</p>
          {#if d && d.failed.length}
            <ul class="fails">{#each d.failed as [code, msg]}<li><span class="mono">{code}</span> <span class="faint">{msg}</span></li>{/each}</ul>
          {/if}
          {#if d && d.cleaned.length}
            <p class="faint cleaned">已把 {d.cleaned.length} 个空出来的原文件夹移到废纸篓</p>
          {/if}
          {#if undone}<StatusPill tone="neutral">本批已撤销</StatusPill>{/if}
          <div class="done-actions">
            <button class="btn primary" onclick={() => app.navigate('library')}><Icon name="grid" size={15} />查看媒体库</button>
            {#if d && d.applied > 0 && !undone}
              {#if confirmUndo}
                <button class="btn danger" disabled={undoing} onclick={undo}><Icon name="undo" size={15} />确认撤销</button>
                <button class="btn ghost" onclick={() => (confirmUndo = false)}>取消</button>
              {:else}
                <button class="btn" onclick={() => (confirmUndo = true)}><Icon name="undo" size={15} />撤销本批</button>
              {/if}
            {/if}
            <button class="btn ghost" onclick={() => flow.reset()}>再来一批</button>
          </div>
        </section>
        {#if app.settings?.log.app && flow.logLines.length}<LogPanel lines={flow.logLines} open={false} />{/if}
      {/if}
    </div>
  {/key}
</div>

<style>
  .page { max-width: 980px; margin: 0 auto; padding: var(--s6) var(--s5) var(--s7); }
  header { display: flex; align-items: center; justify-content: space-between; gap: var(--s5); flex-wrap: wrap; margin-bottom: var(--s5); }
  h1 { font-size: var(--fs-2xl); font-weight: 700; letter-spacing: -0.015em; }
  h2 { font-size: var(--fs-lg); font-weight: 600; }

  .stepper { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; gap: var(--s2); }
  .stepper li { display: flex; align-items: center; gap: var(--s2); color: var(--text-3); font-size: var(--fs-sm); white-space: nowrap; }
  /* 连线:底色线 + 一层从左到右「长」满的强调色 */
  .stepper li + li::before {
    content: ''; width: 24px; height: 2px; border-radius: 2px; margin-right: var(--s2);
    background: linear-gradient(var(--accent), var(--accent)) no-repeat left / 0% 100%, var(--line);
    transition: background-size var(--d-slow) var(--ease-out);
  }
  .stepper li.passed + li::before, .stepper li.cur::before { background-size: 100% 100%, auto; }
  .dot { width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; font-size: 11px; font-weight: 600; background: var(--surface-3); transition: background var(--d-base) var(--ease-out), color var(--d-base), transform var(--d-base) var(--ease-spring); }
  .passed .dot { background: var(--accent-soft); color: var(--accent); }
  .cur { color: var(--text-1); font-weight: 600; }
  .cur .dot { background: var(--accent); color: var(--accent-ink); transform: scale(1.12); animation: breathe 2.2s ease-in-out infinite; }
  @keyframes breathe { 50% { box-shadow: 0 0 0 5px var(--accent-soft); } }

  .panel { display: flex; flex-direction: column; gap: var(--s4); }
  .pillwrap { display: inline-flex; }
  .note-warn { color: var(--warn); }
  .results { padding:0 var(--s4) var(--s4); }
  .results .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: var(--s3); }
  .pad { padding: var(--s5); display: flex; flex-direction: column; gap: var(--s3); }
  .hint { color: var(--text-2); font-size: var(--fs-sm); }
  .titled { display: flex; align-items: center; gap: var(--s1); }
  .disclosure summary { cursor:pointer; padding:var(--s4); font-size:var(--fs-sm); color:var(--text-2); }
  .disclosure summary:hover { color:var(--text-1); }
  .disclosure p { margin-top:var(--s2); overflow-wrap:anywhere; }
  .section-head { padding:var(--s4); display:flex; flex-direction:column; gap:var(--s2); }
  .section-title { display:flex; justify-content:space-between; align-items:center; gap:var(--s3); flex-wrap:wrap; }
  .skipped > .hint { padding:0 var(--s4) var(--s4); }
  .dirrow { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3); background: var(--surface-2); border-radius: var(--r-sm); min-width: 0; }
  .grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pad > .btn { align-self: flex-start; }

  footer { display: flex; align-items: center; justify-content: space-between; gap: var(--s3); position: sticky; bottom: 0; padding: var(--s3) 0; background: linear-gradient(transparent, var(--bg) 35%); }
  .right { display: flex; align-items: center; justify-content:flex-end; gap: var(--s4); min-width:0; }
  .right > .btn { flex-shrink:0; }
  .note { font-size: var(--fs-sm); }
  .alert { display: flex; align-items: center; gap: var(--s2); margin-bottom: var(--s4); padding: var(--s3) var(--s4); background: var(--danger-soft); color: var(--danger); border-radius: var(--r-sm); }

  .summary { display: flex; align-items: center; gap: var(--s6); padding: var(--s4) var(--s5); }
  .stat { display: flex; flex-direction: column; line-height: 1.2; }
  .stat b { font-size: var(--fs-xl); font-weight: 700; }
  .stat span { font-size: var(--fs-xs); color: var(--text-2); }
  .stat.warnc b { color: var(--warn); }
  .live { display: inline-flex; align-items: center; gap: 6px; color: var(--info); font-size: var(--fs-sm); }

  .table { padding: var(--s1); max-height: 52vh; overflow-y: auto; }
  .trow { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3); border-radius: var(--r-sm); transition: background var(--d-fast); }
  .trow:hover { background: var(--surface-2); }
  .cell { min-width: 0; }
  .code { color: var(--accent); font-weight: 600; }
  .scanrow .file { flex: 1; display: flex; flex-direction: column; }
  .scanrow .cell:nth-child(2) { width: 230px; flex-shrink: 0; }
  .tagcell { display: flex; gap: var(--s1); flex-wrap: wrap; width: 180px; flex-shrink: 0; }
  .size { width: 72px; text-align: right; flex-shrink: 0; font-size: var(--fs-sm); }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sm); }
  .sub { font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .more { padding: var(--s3); text-align: center; font-size: var(--fs-sm); }
  .skipped { margin-top: var(--s3); padding: 0; font-size: var(--fs-sm); }
  .skipped summary { display: flex; align-items: center; gap: var(--s2); padding: var(--s3) var(--s4); cursor: pointer; color: var(--text-2); list-style: none; }
  .skipped summary::-webkit-details-marker { display: none; }
  .skipped[open] summary { border-bottom: 1px solid var(--line); }
  .skipped ul { list-style: none; margin: 0; padding: var(--s2) var(--s4) var(--s3); display: flex; flex-direction: column; gap: var(--s1); max-height: 240px; overflow: auto; }
  .skipped li { display: flex; align-items: center; gap: var(--s3); }
  .skipped .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .prog-head { display: flex; justify-content: space-between; align-items: baseline; }
  .prog-foot { display: flex; justify-content: space-between; align-items: center; font-size: var(--fs-sm); min-height: 28px; }
  .scraperow .code { width: 150px; flex-shrink: 0; }
  .err { font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }

  .planrow { cursor: pointer; }
  .planrow.off .head, .planrow.off .paths { opacity: 0.45; }
  .planrow.conflict { cursor: default; }
  .planrow .head { width: 220px; flex-shrink: 0; display: flex; flex-direction: column; }
  .ttl { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .paths { flex: 1; display: flex; flex-direction: column; font-size: var(--fs-xs); }
  .planrow.conflict .paths { opacity: 1; }
  .conflict-path { color: var(--warn); }
  .paths span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .end { width: 108px; flex-shrink: 0; text-align: right; }
  .extras { font-size: var(--fs-xs); }

  .logrow { font-size: var(--fs-sm); animation: logIn 300ms var(--ease-out); }
  .logrow :global(svg) { color: var(--ok); }
  .logrow.bad :global(svg) { color: var(--danger); }
  @keyframes logIn { from { opacity: 0; transform: translateY(-6px); } }

  .done { display: flex; flex-direction: column; align-items: center; text-align: center; gap: var(--s3); padding: var(--s7) var(--s5); }
  .big-icon { width: 68px; height: 68px; border-radius: 50%; display: grid; place-items: center; background: var(--ok-soft); color: var(--ok); animation: pop 520ms var(--ease-back); }
  .big-icon :global(path) { stroke-dasharray: 30; stroke-dashoffset: 30; animation: draw 520ms 180ms var(--ease-out) forwards; }
  @keyframes draw { to { stroke-dashoffset: 0; } }
  .big-icon.warn { background: var(--warn-soft); color: var(--warn); }
  .cleaned { font-size: var(--fs-sm); }
  .done-actions { display: flex; gap: var(--s2); flex-wrap: wrap; justify-content: center; margin-top: var(--s3); }
  .fails { list-style: none; margin: 0; padding: var(--s3) var(--s4); background: var(--danger-soft); border-radius: var(--r-sm); text-align: left; font-size: var(--fs-sm); max-width: 560px; width: 100%; }

  @media (max-width: 820px) {
    .stepper .lbl { display: none; }
    .tagcell, .scanrow .cell:nth-child(2) { width: auto; }
    .planrow { flex-wrap: wrap; }
    .planrow .head { width: auto; }
    .paths { flex-basis: 100%; order: 3; }
  }
  @media (max-width: 720px) {
    .page { padding: var(--s4) var(--s4) 96px; }
    .summary { gap: var(--s4); flex-wrap: wrap; }
    .right { flex-wrap:wrap; }
    footer { align-items:flex-end; }
    .scanrow { flex-wrap:wrap; }
    .scanrow .file { flex-basis:100%; }
    .scraperow { flex-wrap:wrap; }
    .scraperow .err { flex-basis:100%; white-space:normal; overflow-wrap:anywhere; }
  }
  .urlform { display: flex; gap: var(--s1); flex-basis: 100%; flex-wrap: wrap; }
  .urlform .input { min-width: 220px; }
</style>
