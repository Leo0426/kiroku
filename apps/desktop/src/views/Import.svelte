<script lang="ts">
  import Icon from '../components/Icon.svelte'
  import InfoTip from '../components/InfoTip.svelte'
  import CodeEditor from '../components/CodeEditor.svelte'
  import ImportScanFiles from '../components/ImportScanFiles.svelte'
  import ImportRuleEditor from '../components/ImportRuleEditor.svelte'
  import ImportOutcome from '../components/ImportOutcome.svelte'
  import ImportPlanReview from '../components/ImportPlanReview.svelte'
  import LogPanel from '../components/LogPanel.svelte'
  import Count from '../components/Count.svelte'
  import ProgressBar from '../components/ProgressBar.svelte'
  import PopupNotice from '../components/PopupNotice.svelte'
  import LoadingStatus from '../components/LoadingStatus.svelte'
  import ResultCard from '../components/ResultCard.svelte'
  import StatusPill from '../components/StatusPill.svelte'
  import { presetSettingsTab } from './Settings.svelte'
  import { baseName, fmtBytes, fmtDuration, shortPath } from '../lib/format'
  import NasDiagnostics from '../components/NasDiagnostics.svelte'
  import { flow } from '../lib/importflow.svelte'
  import { dur, rise } from '../lib/motion'
  import { scale } from 'svelte/transition'
  import { app } from '../lib/state.svelte'
  import { libraryLocationSetting } from '../lib/settings-search'
  import BatchHistory from '../components/BatchHistory.svelte'
  import { CONTENT_TYPE_LABEL, SOURCE_NAMES, type OrganizeMode, type ScanItem, type ScrapeStatus, type SkipReason } from '../lib/types'

  const STEPS = ['选择来源', '核对资料', '预览整理', '完成']
  const stepIndex = $derived(({ source: 0, scan: 1, scrape: 1, plan: 2, apply: 2, done: 3 } as const)[flow.step])

  const root = $derived(app.settings?.library_root ?? '')
  const MODE: Record<OrganizeMode, { summary: string; progress: string }> = {
    move: { summary: '视频和字幕会移动到媒体库，原位置不再保留。', progress: '移动视频与字幕' },
    copy: { summary: '复制视频和字幕到媒体库，保留源文件。', progress: '复制视频与字幕' },
    hardlink: { summary: '创建硬链接，保留源文件；需要位于同一磁盘分区，修改任一文件会影响另一文件。', progress: '创建视频与字幕的硬链接' },
    symlink: { summary: '创建软链接，保留源文件；播放时仍需要访问源文件。', progress: '创建视频与字幕的软链接' },
  }
  const mode = $derived(flow.effectiveRules?.mode ?? 'move')
  const destinationTitle: Record<OrganizeMode, string> = { move: '移动到媒体库', copy: '复制到媒体库', hardlink: '在媒体库创建硬链接', symlink: '在媒体库创建软链接' }
  let scanOpen = $state(false)
  let codeDrafts = $state<Record<number, string | undefined>>({})
  /** 正在为哪一部指定详情页地址 */
  let urlFor = $state<string | null>(null)
  let urlText = $state('')
  let foundOpen = $state(false)
  let statusesOpen = $state(false)
  let showAllLocal = $state(false)
  $effect(() => {
    if (flow.step !== 'scan') { scanOpen = false; showAllLocal = false; codeDrafts = {} }
    if (flow.step !== 'scrape') { foundOpen = false; statusesOpen = false }
  })
  const issueCodes = $derived(flow.uniqueCodes.filter(code => {
    const kind = flow.statuses[code]?.kind
    return kind === 'failed' || kind === 'notFound' || kind === 'noRoute' || kind === 'cancelled'
      || (!flow.scrape.running && kind !== 'found')
  }))
  const activeCodes = $derived(flow.uniqueCodes.filter(code => ['scraping', 'translating'].includes(flow.statuses[code]?.kind ?? '')).slice(0, 3))
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
  let supplementBusy = $state(false)
  let confirmUndo = $state(false)
  $effect(() => {
    if (!flow.batchInCurrentLibrary) confirmUndo = false
  })
  async function undo() {
    const id = flow.applyDone?.journalId
    if (id) await flow.undoJournal(flow.applyRoot, id)
    confirmUndo = false
  }

  const applyTail = $derived(flow.applyLog.slice(-8).reverse())
</script>

{#snippet scanRow(it: ScanItem)}
  <div class="trow scanrow">
    <div class="cell file"><span class="name" title={it.path}>{baseName(it.path)}</span><span class="faint sub">{shortPath(parentOf(it.path), 60)}</span>
      {#if it.code && flow.localByCode[it.code.id]?.preview.localAssets}<span class="faint sub">{flow.localByCode[it.code.id].preview.localAssets}</span>{/if}
    </div>
    <div class="cell"><CodeEditor code={it.code?.id ?? null} bind:draft={codeDrafts[it.id]} error={flow.codeErrors[it.id]} disabled={flow.busy} onsubmit={(t) => flow.setCode(it.id, t)} /></div>
    <div class="cell tagcell">{#each tags(it) as t (t.text)}<StatusPill tone={t.tone}>{t.text}</StatusPill>{/each}</div>
    <div class="cell size tnum faint">{fmtBytes(it.size)}</div>
  </div>
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

  {#if flow.step==='source'||flow.step==='plan'}<NasDiagnostics compact/>{/if}
  {#if flow.step==='source' && flow.cachedSourcesOnly}
  <label class="hint"><input type="checkbox" bind:checked={flow.cachedSourcesOnly} disabled={flow.busy}/> 只复用已有来源缓存（缺少来源缓存的影片跳过；图片和翻译仍可能联网）</label>
  {/if}
  {#if flow.error}<PopupNotice text={flow.error} />{/if}

  {#key flow.step}
    <div class="panel stagger" in:rise={{ duration: 300, y: 10 }}>
      <!-- 1. 选择来源 -->
      {#if flow.step === 'source'}
        <section class="card pad" aria-labelledby="source-dirs-title">
          <div class="titled">
            <h2 id="source-dirs-title">待整理来源</h2>
            <InfoTip label="待整理来源说明">
              <p>添加文件夹会扫描其中的子文件夹。也可以选择一个或多个视频，只整理选中的文件，保留它们所在的文件夹。</p>
              <p>成功抓取的信息和图片会自动缓存。重启后重新扫描并开始查找信息，已缓存的请求会跳过，失败部分会重试。</p>
              <p><b>支持的视频格式</b><br /><span class="mono">mp4 / mkv / avi / wmv / mov / ts / m2ts / flv / rmvb</span></p>
            </InfoTip>
          </div>
          {#each flow.sources as source (source.path)}
            <div class="dirrow">
              <Icon name={source.kind === 'video' ? 'film' : 'folder'} size={16} /><span class="mono grow" title={source.path}>{source.path}</span>
              {#if source.kind === 'video'}<span class="faint">单个文件</span>{/if}
              <button class="btn ghost icon sm" aria-label="移除 {source.path}" onclick={() => flow.removeSource(source.path)}><Icon name="x" size={14} /></button>
            </div>
          {/each}
          <div class="source-picker" class:empty={!flow.sources.length}>
            {#if !flow.sources.length}<span class="folder-mark"><Icon name="folder" size={28} /></span>{/if}
            <div class="source-actions">
              <button class="btn" onclick={() => flow.pickSources('folder')}><Icon name="folder" size={15} />添加文件夹</button>
              <button class="btn" onclick={() => flow.pickSources('video')}><Icon name="film" size={15} />添加视频文件</button>
            </div>
            {#if flow.readingSources}<LoadingStatus label="正在检查所选来源…" />
            {:else}<span class="hint">也可以将文件夹或视频拖入窗口</span>{/if}
          </div>
        </section>

        <section class="card pad" aria-label="整理目标">
          <div class="titled">
            <h2>{destinationTitle[mode]}</h2>
            <InfoTip label="整理方式说明">
              <p>{MODE[mode].summary} 影片信息和海报会保存在媒体库。</p>
              <p><b>命名规则</b><br /><span class="mono">{flow.effectiveRules?.directoryTemplate}/{flow.effectiveRules?.plexNaming ? '(Plex 命名)' : flow.effectiveRules?.fileTemplate}</span></p>
              <p>可为本批调整文件处理方式和命名。</p>
            </InfoTip>
          </div>
          <div class="dirrow"><Icon name="folder" size={16} /><span class="mono grow" title={root}>{root}{flow.effectiveRules?.subdirectory ? `/${flow.effectiveRules.subdirectory}` : ''}</span></div>
          <ImportRuleEditor />
        </section>

        <footer>
          <span></span>
          <button class="btn primary" disabled={flow.sources.length === 0 || flow.editingRules || flow.busy} onclick={() => flow.startScan()}>开始扫描<Icon name="arrow-right" size={15} /></button>
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
            <LoadingStatus label="正在扫描视频和本地资料…" />
            <button class="btn sm" onclick={() => flow.cancel()}>停止</button>
          {/if}
        </section>
        {#if flow.scanRunning}<ProgressBar label="文件扫描进度" />{/if}

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
            <ImportScanFiles items={flow.unrecognized} row={scanRow} />
          </section>
        {/if}
        {#if !flow.scanRunning && flow.localReady.length}
          <section class="card local-preparation" aria-labelledby="local-import-title">
            <div class="section-head">
              <div class="section-title">
                <h2 id="local-import-title">本地资料</h2>
                {#if flow.localReady.length > 1}<button class="btn sm" type="button" disabled={flow.busy} onclick={() => flow.rescrape()}>批量重新抓取</button>{/if}
              </div>
              {#if flow.scrape.running}
                <div class="prog-head"><span>正在重新抓取 · {flow.scrape.done} / {flow.scrape.total}</span><button class="btn sm" type="button" onclick={() => flow.cancel()}>停止</button></div>
                <ProgressBar value={flow.scrape.done / flow.scrape.total} active label="重新抓取进度" />
              {:else}
                <p class="hint">已就绪 {flow.readyCount} 部{#if flow.retryCodes.length}{' · '}待补齐 {flow.retryCodes.length} 部{/if}</p>
              {/if}
            </div>
            <div class="grid local-results">
              {#each (showAllLocal ? flow.localReady : flow.localReady.slice(0, 6)) as item (item.code)}
                <ResultCard code={item.code} p={flow.foundByCode[item.code] ?? item.preview} note={flow.notes[item.code]} busy={flow.busy} onrefresh={() => flow.rescrape(item.code)} />
              {/each}
            </div>
            {#if flow.localReady.length > 6}<button class="btn sm ghost local-more" type="button" onclick={() => (showAllLocal = !showAllLocal)}>{showAllLocal ? '收起本地资料' : `查看全部 ${flow.localReady.length} 部`}</button>{/if}
          </section>
        {:else if !flow.scanRunning && flow.recognized.length}
          <p class="hint">有 {flow.retryCodes.length} 部待补齐资料。{#if flow.localImports.length}已有的图片会随整理复用。{/if}</p>
        {/if}
        {#if flow.recognized.length}
          <details class="card disclosure" bind:open={scanOpen}>
            <summary>查看已识别文件 · {flow.recognized.length} 个</summary>
            {#if scanOpen}<section aria-label="扫描结果"><ImportScanFiles items={flow.recognized} row={scanRow} /></section>{/if}
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

        {#if !flow.scanRunning && flow.retryCodes.length}
          <details class="card disclosure" open={flow.cachedSourcesOnly}>
            <summary>查询选项</summary>
            <label class="hint cache-option"><input class="checkbox" type="checkbox" bind:checked={flow.cachedSourcesOnly} disabled={flow.busy} />仅使用站点资料缓存</label>
            <p class="hint cache-hint">缺少缓存的影片会列入待处理。图片下载和 AI 汉化仍可能联网。</p>
          </details>
        {/if}

        <footer>
          <button class="btn ghost" disabled={flow.busy} onclick={() => flow.reset('reselectSources')}><Icon name="arrow-left" size={15} />重新选择</button>
          <div class="right">
            {#if flow.unrecognized.length > 0 && !flow.scanRunning}<span class="faint note">未识别的 {flow.unrecognized.length} 个文件会被跳过</span>{/if}
            {#if flow.readyCount && flow.retryCodes.length}<button class="btn" disabled={flow.busy} onclick={() => flow.buildPlan('scan')}>仅预览就绪影片</button>{/if}
            <button class="btn primary" disabled={flow.busy || flow.recognized.length === 0} onclick={() => flow.prepareImport()}>
              {#if flow.planning}正在准备预览…{:else if flow.scrape.running}正在重新抓取…{:else if flow.readyCount && !flow.retryCodes.length}预览整理{:else if flow.readyCount}补齐 {flow.retryCodes.length} 部并预览{:else}查找影片信息（{flow.uniqueCodes.length} 部）{/if}<Icon name={flow.planning || flow.scrape.running ? 'loader' : 'arrow-right'} size={15} />
            </button>
            {#if flow.planning}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
          </div>
        </footer>

        <!-- 3. 刮削 -->
      {:else if flow.step === 'scrape'}
        <section class="card pad">
          <div class="prog-head">
            <h2>{flow.scrape.running ? '正在查找影片信息' : flow.scrapeSummary?.cancelled ? '已停止' : '影片信息查询完成'}</h2>
            <span class="tnum muted">{flow.scrape.done} / {flow.scrape.total}</span>
          </div>
          <ProgressBar value={flow.scrape.total ? flow.scrape.done / flow.scrape.total : flow.scrape.running ? undefined : 0} active={flow.scrape.running} label="影片信息查询进度" />
          <div class="prog-foot">
            <span class="faint">
              {#if flow.scrape.running}{flow.eta !== null ? `预计还需 ${fmtDuration(flow.eta)}` : '正在估算剩余时间…'}{:else}找到 {counts.found} · 未找到 {counts.notFound} · 失败 {counts.failed}{/if}
            </span>
            {#if flow.scrape.running}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
          </div>
          {#if flow.scrapeSummary?.cache}
          {@const cache=flow.scrapeSummary.cache}
          <p class="hint" aria-label="来源缓存摘要">来源缓存：复用 {cache.reused} · 缺失 {cache.missing} · 配置失效 {cache.invalidated} · 损坏 {cache.corrupt} · 不可读 {cache.unreadable}；仅缓存模式跳过 {cache.skippedMovies} 部。统计按来源请求计。</p>
          {/if}
          <p class="hint">{flow.scrape.running ? '可以切换到其他页面，查询会继续进行。' : flow.foundIds.length ? '找到信息的影片可以继续预览整理，其余影片可重试或暂时跳过。' : '还没有可整理的影片，请重试查询或返回检查番号。'}</p>
        </section>

        {#if flow.scrape.running && activeCodes.length}
          <section class="card table" aria-label="正在查询的影片">{@render statusRows(activeCodes)}</section>
        {/if}
        {#if issueCodes.length}
          <section class="card" aria-label="需要处理的影片">
            <div class="section-head"><div class="section-title"><h2>需要处理 · {issueCodes.length} 部</h2>
              {#if !flow.scrape.running && flow.retryCodes.length}<button class="btn sm" onclick={() => flow.startScrape()}>重试未完成（{flow.retryCodes.length} 部）</button>{/if}
            </div><p class="hint">查询失败或未找到信息的影片会跳过；已找到信息的影片仍可继续整理。</p></div>
            <div class="table">{@render statusRows(issueCodes)}</div>
          </section>
        {/if}
        {#if flow.found.length}
          <details class="card disclosure" bind:open={foundOpen}>
            <summary>查看已找到的影片 · {flow.found.length} 部</summary>
            <section class="results" aria-label="已找到的影片">
              {#if foundOpen}<div class="grid">{#each flow.found as f (f.code)}<ResultCard code={f.code} p={f.preview} note={flow.notes[f.code]} busy={flow.busy} onrefresh={f.preview.localAssets ? () => flow.rescrape(f.code) : undefined} />{/each}</div>{/if}
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
            {flow.planning ? '正在准备预览…' : '预览整理'}<Icon name={flow.planning ? 'loader' : 'arrow-right'} size={15} />
          </button>
          {#if flow.planning}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
        </footer>

        <!-- 4. 整理计划 -->
      {:else if flow.step === 'plan'}
        <section class="card pad" aria-label="本批规则">
          <div class="titled"><h2>{destinationTitle[mode]}</h2></div>
          <span class="mono grow">{root}{flow.effectiveRules?.subdirectory ? `/${flow.effectiveRules.subdirectory}` : ''}</span>
          <ImportRuleEditor disabled={supplementBusy} />
          {#if flow.planning}<LoadingStatus label="正在重新检查目标…" />{/if}
        </section>
        <ImportPlanReview bind:supplementBusy {root} omittedCount={flow.uniqueCodes.length - counts.found} />

        <footer>
          <button class="btn ghost" disabled={flow.busy || supplementBusy} onclick={() => flow.backFromPlan()}><Icon name="arrow-left" size={15} />上一步</button>
          <div class="right">
            <span class="faint note" role="note" aria-label="文件处理方式">{MODE[mode].summary} 可在「历史」中撤销。{#if mode === 'move' && app.settings?.cleanup_source}空出来的原文件夹会移到废纸篓。{/if}</span>
            <button class="btn primary" disabled={supplementBusy || flow.selectedCount === 0 || flow.editingRules || flow.busy || !flow.planToken} onclick={() => flow.startApply()}>确认整理（{flow.selectedCount} 个文件）</button>
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
          <ProgressBar value={progress.total ? progress.done / progress.total : flow.apply.running ? undefined : 0} active={flow.apply.running} label={downloading ? '图片下载进度' : '整理进度'} />
          <div class="prog-foot">
            {#if downloading}
              <span class="faint" role="status" aria-label="图片下载状态">下载海报与剧照{flow.applyProgress?.code ? ` · ${flow.applyProgress.code}` : ''}{flow.applyProgress?.failed ? ` · ${flow.applyProgress.failed} 张未下载成功` : ''}</span>
            {:else}
              <span class="faint" role="status" aria-label="当前整理方式">{indexing ? '正在更新影片索引' : `${MODE[flow.applyMode].progress}，保存影片信息与海报`}</span>
            {/if}
            {#if flow.apply.running}<button class="btn sm" onclick={() => flow.cancel()}>停止</button>{/if}
          </div>
        </section>
        {#if applyTail.length}
          <section class="card table" aria-label="整理日志">
            {#each applyTail as e (e.itemId)}
              <div class="trow logrow" class:bad={!e.ok}>
                <Icon name={e.ok ? 'check' : 'alert'} size={14} />
                <span class="mono code">{e.code}</span>
                <span class="faint err">{e.error ?? (e.warnings.length ? `完成,但有 ${e.warnings.length} 条警告` : '完成')}</span>
              </div>
            {/each}
          </section>
        {/if}
        {#if app.settings?.log.app}<LogPanel lines={flow.logLines} live={flow.apply.running} />{/if}

        <!-- 5. 完成 -->
      {:else}
        {@const d = flow.applyDone}
        <section class="card done">
          {#if d}
            <ImportOutcome result={d} events={flow.applyLog} plans={flow.plans} rollback={flow.batchRollback}
              undone={flow.batchUndone} currentLibrary={flow.batchInCurrentLibrary} />
          {/if}
          <div class="done-actions">
            {#if flow.applyRoot && !flow.batchInCurrentLibrary}
              <button class="btn primary" onclick={() => app.navigate('settings', libraryLocationSetting)}><Icon name="settings" size={15} />前往媒体库设置</button>
            {:else}
              <button class="btn primary" onclick={() => app.navigate('library')}><Icon name="grid" size={15} />查看媒体库</button>
            {/if}
            {#if flow.batchInCurrentLibrary && d?.journalId && d.canRollback && !flow.batchUndone}
              {#if flow.undoing?.id === d.journalId && flow.undoing.root === flow.applyRoot}
                <button class="btn" disabled>撤销中…</button>
              {:else if confirmUndo}
                <button class="btn danger" disabled={!!flow.undoing} onclick={undo}><Icon name="undo" size={15} />确认撤销</button>
                <button class="btn ghost" onclick={() => (confirmUndo = false)}>取消</button>
              {:else}
                <button class="btn" disabled={!!flow.undoing} onclick={() => (confirmUndo = true)}><Icon name="undo" size={15} />{flow.batchRollback ? '重试撤销' : '撤销本批'}</button>
              {/if}
            {/if}
            <button class="btn ghost" disabled={!!flow.undoing} onclick={() => flow.reset()}>再来一批</button>
          </div>
          {#if d}
            {#key d}<BatchHistory journalId={d.journalId} root={flow.applyRoot} />{/key}
          {/if}
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
  .local-results { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr)); gap: var(--s3); padding: 0 var(--s4) var(--s4); }
  .local-more { margin: 0 var(--s4) var(--s4); }
  .cache-option { display: flex; align-items: center; gap: var(--s2); padding: 0 var(--s4); }
  .cache-hint { padding: 0 var(--s4) var(--s3); }
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
  .source-picker { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
  .source-actions { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--s2); }
  .source-picker.empty { flex-direction: column; justify-content: center; padding: var(--s5); gap: var(--s2); background: var(--surface-2); border: 1px dashed var(--line-strong); border-radius: var(--r-md); }
  .folder-mark { color: var(--accent); margin-bottom: var(--s1); }

  footer { display: flex; align-items: center; justify-content: space-between; gap: var(--s3); position: sticky; bottom: 0; z-index: 1; padding: var(--s3) 0; border-top: 1px solid var(--line); background: var(--bg); }
  .right { display: flex; align-items: center; justify-content:flex-end; gap: var(--s4); min-width:0; }
  .right > .btn { flex-shrink:0; }
  .note { font-size: var(--fs-sm); }

  .summary { display: flex; align-items: center; gap: var(--s6); padding: var(--s4) var(--s5); }
  .stat { display: flex; flex-direction: column; line-height: 1.2; }
  .stat b { font-size: var(--fs-xl); font-weight: 700; }
  .stat span { font-size: var(--fs-xs); color: var(--text-2); }
  .stat.warnc b { color: var(--warn); }

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

  .logrow { font-size: var(--fs-sm); animation: logIn 300ms var(--ease-out); }
  .logrow :global(svg) { color: var(--ok); }
  .logrow.bad :global(svg) { color: var(--danger); }
  @keyframes logIn { from { opacity: 0; transform: translateY(-6px); } }

  .done { display: flex; flex-direction: column; align-items: center; text-align: center; gap: var(--s3); padding: var(--s7) var(--s5); }
  .done-actions { display: flex; gap: var(--s2); flex-wrap: wrap; justify-content: center; margin-top: var(--s3); }

  @media (max-width: 820px) {
    .stepper li:not(.cur) .lbl { position: absolute; width: 1px; height: 1px; padding: 0; clip-path: inset(50%); overflow: hidden; }
    .stepper li + li::before { width: 12px; margin-right: 0; }
    .tagcell, .scanrow .cell:nth-child(2) { width: auto; }
  }
  @media (max-width: 720px) {
    .page { padding: var(--s4) var(--s4) 96px; }
    .summary { gap: var(--s4); flex-wrap: wrap; }
    .right { flex-wrap:wrap; gap: var(--s2); }
    .right .note { flex-basis: 100%; }
    footer { align-items:flex-end; }
    .scanrow { flex-wrap:wrap; }
    .scanrow .file { flex-basis:100%; }
    .scraperow { flex-wrap:wrap; }
    .scraperow .err { flex-basis:100%; white-space:normal; overflow-wrap:anywhere; }
  }
  .urlform { display: flex; gap: var(--s1); flex-basis: 100%; flex-wrap: wrap; }
  .urlform .input { min-width: 220px; }
</style>
