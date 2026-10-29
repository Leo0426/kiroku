<script lang="ts" module>
  export type SettingsTab = 'general' | 'organize' | 'naming' | 'scan' | 'names' | 'badges' | 'sources' | 'ai' | 'plex' | 'data' | 'advanced'
  /** 离开设置页再回来时停在上次的分类 */
  let lastTab: SettingsTab = 'general'
  /** 其他页面跳到设置的某个分类前调用(下一次打开设置页时生效) */
  export function presetSettingsTab(t: SettingsTab) {
    lastTab = t
  }
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { slide } from 'svelte/transition'
  import { findSettings, type SettingLink } from '../lib/settings-search'
  import AppearanceSettings from '../components/AppearanceSettings.svelte'
  import AiSettings from '../components/AiSettings.svelte'
  import NameRules from '../components/NameRules.svelte'
  import Icon from '../components/Icon.svelte'
  import InfoTip from '../components/InfoTip.svelte'
  import LibraryMaintenance from '../components/LibraryMaintenance.svelte'
  import SourceAuth from '../components/SourceAuth.svelte'
  import SourceChecks from '../components/SourceChecks.svelte'
  import SiteList from '../components/SiteList.svelte'
  import { api } from '../lib/api'
  import { flow } from '../lib/importflow.svelte'
  import { dur, rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import { CONTENT_TYPES, FIELDS, SOURCE_NAMES, SOURCE_SUPPORT, SOURCE_HINTS, type BadgeKind, type Corner, type PlexTest, type ServerTest, type Settings, type SourceCfg, type SourceProxy } from '../lib/types'

  let s = $state<Settings | null>(null)
  let saved = $state('')
  let path = $state('')
  let error = $state('')
  let saving = $state(false)
  let loading = $state(false)
  let rootChanging = $state(false)
  const rootLocked = $derived(rootChanging || saving || flow.busy || app.rebuild.running || (flow.step !== 'source' && flow.step !== 'done'))

  const name = (id: string) => SOURCE_NAMES[id] ?? id
  const ids = $derived(s ? s.sources.map((x) => x.id) : [])
  const dirty = $derived(s !== null && JSON.stringify(s) !== saved)

  // ---- 分类 ----
  const TABS: { key: SettingsTab; label: string; icon: 'folder' | 'image' | 'globe' | 'sparkles' | 'tv' | 'sliders'; desc: string }[] = [
    { key: 'general', label: '常规', icon: 'folder', desc: '选择整理后的影片存放位置' },
    { key: 'organize', label: '整理', icon: 'folder', desc: '文件处理方式与附加图片' },
    { key: 'naming', label: '命名与 NFO', icon: 'sliders', desc: '目录、文件名与影片信息语言' },
    { key: 'scan', label: '扫描', icon: 'folder', desc: '跳过样片、广告和过小的视频' },
    { key: 'names', label: '名称规则', icon: 'sliders', desc: '统一演员、标签、制作商、发行商与系列' },
    { key: 'badges', label: '封面角标', icon: 'image', desc: '选择角标位置与大小' },
    { key: 'sources', label: '数据源', icon: 'globe', desc: '刮削站点、请求间隔与代理' },
    { key: 'ai', label: 'AI 汉化', icon: 'sparkles', desc: '把日文标题、简介翻成中文' },
    { key: 'plex', label: '媒体服务器', icon: 'tv', desc: 'Plex、Jellyfin、Emby 自动刷新' },
    { key: 'data', label: '生成数据', icon: 'folder', desc: '查看并删除可安全清理的原图、备份和运行日志' },
    { key: 'advanced', label: '高级', icon: 'sliders', desc: '维护、日志与刮削规则' },
  ]
  let tab = $state<SettingsTab>(lastTab)
  const tabIndex = $derived(TABS.findIndex((t) => t.key === tab))
  const current = $derived(TABS[tabIndex])

  let search = $state('')
  const matches = $derived(findSettings(search))
  async function locate(item: SettingLink) {
    go(item.tab)
    if (item.target === 'settings-routes') rulesOpen = true
    search = ''
    await tick()
    const target = item.target ? document.getElementById(item.target) : null
    const focus = target && !target.matches(':disabled') ? target : document.querySelector<HTMLElement>('.pane-head')
    if (focus && !focus.matches('input, select, textarea, button')) focus.tabIndex = -1
    focus?.focus({ preventScroll:true })
    ;(target ?? focus)?.scrollIntoView({ block:'center' })
  }

  function go(t: SettingsTab) {
    tab = lastTab = t
  }

  /** 每个分类负责的设置项;用于标记「未保存」和「恢复本页默认」 */
  const KEYS: Record<SettingsTab, (keyof Settings)[]> = {
    general: [],
    data: [],
    organize: ['organize_mode', 'download_extrafanart', 'cleanup_source', 'notify_on_finish'],
    naming: ['nfo_language', 'dir_template', 'file_template'],
    scan: ['scan'],
    names: ['name_rules'],
    badges: ['badges'],
    sources: ['sources', 'routes', 'min_interval_secs', 'proxy', 'solver'],
    ai: ['ai'],
    plex: ['plex', 'media_server'],
    advanced: ['routes', 'field_priority', 'field_blacklist', 'log'],
  }
  const part = (v: Settings, t: SettingsTab) => JSON.stringify(KEYS[t].map((k) => v[k]))
  const changed = $derived.by(() => {
    const out = new Set<SettingsTab>()
    if (!s || !saved) return out
    const base = JSON.parse(saved) as Settings
    for (const t of TABS) if (part(s, t.key) !== part(base, t.key)) out.add(t.key)
    return out
  })

  function navKey(e: KeyboardEvent) {
    const d = e.key === 'ArrowDown' || e.key === 'ArrowRight' ? 1 : e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? -1 : 0
    if (!d) return
    e.preventDefault()
    const next = TABS[(tabIndex + d + TABS.length) % TABS.length].key
    go(next)
    ;(e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-tab="${next}"]`)?.focus()
  }

  const logDir = $derived(/settings\.json$/.test(path) ? path.replace(/settings\.json$/, 'logs') : '')
  const DEFAULT_FILE_TPL = '{code}[-CD{cd?}][-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}]'
  const DEFAULT_EXTRA = '(?i)sample|trailer|preview|预告|預告|样片|樣本|样本'

  // ---- 封面角标(对齐 amane:每类一角,同角按此顺序向内叠) ----
  const CORNERS: { key: Corner; label: string }[] = [
    { key: 'top_left', label: '左上' },
    { key: 'top_right', label: '右上' },
    { key: 'bottom_left', label: '左下' },
    { key: 'bottom_right', label: '右下' },
  ]
  const KINDS: { key: BadgeKind; label: string; stamp: string }[] = [
    { key: 'subtitle', label: '中字', stamp: '字幕' },
    { key: 'uncensored', label: '无码', stamp: '无码' },
    { key: 'cracked', label: '破解', stamp: '破解' },
    { key: 'leaked', label: '流出', stamp: '流出' },
    { key: 'definition', label: '清晰度', stamp: '4K' },
  ]
  const stampDir = $derived(/settings\.json$/.test(path) ? path.replace(/settings\.json$/, 'watermarks') : '')

  let rulesOpen = $state(false)
  /** 跟随全局时省略字段,与后端保存格式一致 */
  function setSourceProxy(src: SourceCfg, mode: SourceProxy['mode']) {
    if (mode === 'global') delete src.proxy
    else src.proxy = mode === 'custom' ? { mode, url: src.proxy?.mode === 'custom' ? src.proxy.url : '' } : { mode }
  }

  async function clearTranslationCache() {
    try { await api.clearTranslationCache(); app.toast('ok', '译文缓存已清空') }
    catch (e) { app.toast('error', String(e), 5000) }
  }

  async function editSourceRules() {
    go('advanced')
    rulesOpen = true
    await tick()
    document.getElementById('settings-routes')?.scrollIntoView({ block: 'start' })
  }
  let more = $state(false)
  let showCookie = $state(false)

  function addSourceRoutes(id: string) {
    if (!s) return
    for (const type of SOURCE_SUPPORT[id] ?? []) {
      s.routes[type] ??= []
      if (!s.routes[type].includes(id)) s.routes[type].push(id)
    }
  }
  const sourceRoutes = (id: string) => CONTENT_TYPES.filter(ct => s?.routes[ct.key]?.includes(id)).map(ct => ct.label).join('、')

  /** 补齐所有路由/字段的空列表,编辑时就地修改;保存时再剔除空项 */
  function normalized(v: Settings): Settings {
    for (const ct of CONTENT_TYPES) v.routes[ct.key] ??= []
    for (const f of FIELDS) {
      v.field_priority[f.key] ??= []
      v.field_blacklist[f.key] ??= []
    }
    return v
  }

  async function load() {
    if (loading) return
    loading = true
    error = ''
    try {
      s = normalized(await api.getSettings())
      saved = JSON.stringify(s)
      path = await api.settingsPath()
    } catch (e) {
      error = String(e)
    } finally {
      loading = false
    }
  }

  onMount(() => {
    void load()
    const guard = () => {
      if (saving) {
        app.toast('info', '正在保存设置，请稍候')
        return false
      }
      return !dirty || app.confirm({
        title: '设置尚未保存',
        body: '离开会丢弃本次修改。可以继续编辑并保存，或放弃更改后离开。',
        confirmLabel: '放弃更改并离开',
        cancelLabel: '继续编辑',
      })
    }
    app.beforeNavigate = guard
    return () => {
      if (app.beforeNavigate === guard) app.beforeNavigate = null
    }
  })

  function beforeUnload(e: BeforeUnloadEvent) {
    if (dirty || saving) e.preventDefault()
  }

  /** 保存前去掉空列表;库根目录和上次目录以全局状态为准(它们不在本页编辑) */
  function toSave(v: Settings): Settings {
    const prune = (m: Record<string, string[]>) => Object.fromEntries(Object.entries(m).filter(([, l]) => l.length))
    return {
      ...v,
      field_priority: prune(v.field_priority),
      field_blacklist: prune(v.field_blacklist),
      library_root: app.settings?.library_root ?? null,
      source_dirs: [...(app.settings?.source_dirs ?? [])],
    }
  }

  async function save() {
    if (!s || saving) return
    const snapshot = $state.snapshot(s) as Settings
    saving = true
    error = ''
    try {
      await api.saveSettings(toSave(snapshot))
      saved = JSON.stringify(snapshot)
      await app.refreshSettings()
      app.toast('ok', '设置已保存')
    } catch (e) {
      error = String(e)
    } finally {
      saving = false
    }
  }

  /** 只把当前分类恢复为默认值,其他分类不动(仍需点保存) */
  async function reset() {
    if (!s) return
    const d = normalized(await api.defaultSettings()) as unknown as Record<string, unknown>
    const cur = s as unknown as Record<string, unknown>
    for (const k of KEYS[tab]) cur[k] = d[k]
    error = ''
    app.toast('info', `「${current.label}」已恢复默认,保存后生效`)
  }

  // ---- Plex ----
  let plexTesting = $state(false)
  let plexResult = $state<PlexTest | null>(null)
  let plexError = $state('')
  let showToken = $state(false)

  async function testPlex() {
    if (!s) return
    plexTesting = true
    plexError = ''
    plexResult = null
    try {
      plexResult = await api.plexTest(s.plex.url, s.plex.token)
    } catch (e) {
      plexError = String(e)
    } finally {
      plexTesting = false
    }
  }

  // ---- Jellyfin / Emby ----
  let serverTesting = $state(false)
  let serverResult = $state<ServerTest | null>(null)
  let serverError = $state('')
  let showServerKey = $state(false)
  const serverName = $derived(s?.media_server.kind === 'emby' ? 'Emby' : 'Jellyfin')

  async function testServer() {
    if (!s) return
    serverTesting = true
    serverError = ''
    serverResult = null
    try {
      serverResult = await api.mediaServerTest($state.snapshot(s.media_server))
    } catch (e) {
      serverError = String(e)
    } finally {
      serverTesting = false
    }
  }

  async function changeRoot(useDefault = false) {
    if (rootLocked) return
    rootChanging = true
    try {
      if (await (useDefault ? app.useDefaultLibraryRoot() : app.chooseLibraryRoot())) app.toast('ok', '已切换媒体库')
    } catch (e) {
      error = String(e)
    } finally {
      rootChanging = false
    }
  }
  let templateSample=$state<import('../lib/types').TemplatePreview|null>(null)
  $effect(()=>{const directory=s?.dir_template,file=s?.plex.naming?'{code}[-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}][ {title}][ - cd{cd?}]':s?.file_template;let alive=true;const timer=setTimeout(()=>{if(directory!=null&&file!=null)api.templatePreview(directory,file).then(p=>{if(alive)templateSample=p})},150);return ()=>{alive=false;clearTimeout(timer)}})
</script>

<svelte:window onbeforeunload={beforeUnload} />

<div class="page" in:rise>
  {#if s}
    <header>
      <div>
        <div class="tipped"><h1>设置</h1><InfoTip label="设置说明"><p>按分类调整，统一保存。</p></InfoTip></div>
      </div>
      <div class="actions">
        <button class="btn ghost" onclick={reset} disabled={saving || KEYS[tab].length === 0} title="只恢复「{current.label}」分类">恢复本页默认</button>
        <button class="btn primary save" class:pending={dirty} onclick={save} disabled={!dirty || saving || rootChanging}>
          <Icon name="check" size={14} />{saving ? '保存中…' : dirty ? `保存更改(${changed.size})` : '已保存'}
        </button>
      </div>
    </header>

    <div class="settings-search">
      <input class="input" type="search" aria-label="搜索设置" placeholder="搜索设置，例如代理、字体、模板" bind:value={search} onkeydown={e => { if (e.key === 'Escape') search = ''; if (e.key === 'Enter' && !e.isComposing && matches[0]) void locate(matches[0]) }} />
      {#if search.trim()}
        <div class="search-results" aria-label="设置搜索结果">
          <p class="muted" role="status">{matches.length ? `${matches.length} 项匹配设置` : '没有匹配项，试试类别或功能名称。'}</p>
          {#each matches as item}<button class="btn ghost" onclick={() => locate(item)}>{item.label}<span class="faint">{TABS.find(t => t.key === item.tab)?.label}</span></button>{/each}
        </div>
      {/if}
    </div>
    {#if error}<div class="error" role="alert" aria-label="保存失败"><Icon name="alert" size={16} />{error}</div>{/if}

    <div class="layout">
      <div class="cats" role="tablist" aria-orientation="vertical" aria-label="设置分类" tabindex="-1" style="--i:{tabIndex}" onkeydown={navKey}>
        {#each TABS as t (t.key)}
          <button
            class="cat ripple-host" class:active={tab === t.key} data-tab={t.key}
            role="tab" aria-label={t.label} aria-selected={tab === t.key} tabindex={tab === t.key ? 0 : -1}
            onclick={() => go(t.key)}
          >
            <Icon name={t.icon} size={16} />
            <span class="cat-label">{t.label}</span>
            {#if changed.has(t.key)}<span class="dot" title="有未保存的更改"></span>
            {:else if t.key === 'badges' && s.badges.enabled}<span class="on">开</span>
            {:else if t.key === 'ai' && s.ai.enabled}<span class="on">开</span>
            {:else if t.key === 'plex' && (s.plex.enabled || s.plex.naming || s.media_server.enabled)}<span class="on">开</span>{/if}
          </button>
        {/each}
      </div>

      {#key tab}
      <div class="pane stagger" role="tabpanel" aria-label={current.label} in:rise={{ y: 10, duration: 280 }}>
        <div class="pane-head">
          <div class="tipped"><h2 class="pane-title">{current.label}</h2><InfoTip label="{current.label}说明"><p>{current.desc}</p></InfoTip></div>
        </div>
    {#if tab === 'data'}<LibraryMaintenance {dirty} onpolicySaved={() => { if (s && app.settings) { s.retention = { ...app.settings.retention }; saved = JSON.stringify(s) } }} />{/if}
    {#if tab === 'names'}<NameRules bind:rules={s.name_rules} {dirty}/>{/if}
    {#if tab === 'general'}
    <section class="card">
      <div class="tipped"><h2>媒体库</h2><InfoTip label="媒体库说明"><p>整理后的影片和海报将保存到此目录。</p></InfoTip></div>
      <div class="row">
        <span class="label">位置</span>
        <span class="grow root mono">{app.settings?.library_root ?? '未设置'}</span>
        {#if app.media?.suggestedLibrary && app.settings?.library_root !== app.media.suggestedLibrary}
          <button class="btn sm ghost" title={app.media.suggestedLibrary} disabled={rootLocked} onclick={() => changeRoot(true)}>使用默认位置</button>
        {/if}
        <button class="btn sm" disabled={rootLocked} onclick={() => changeRoot()}><Icon name="folder" size={14} />更改</button>
      </div>
      {#if flow.step !== 'source' && flow.step !== 'done'}<p class="hint">完成或重置当前导入后，才能更换媒体库。</p>{/if}
      <div class="row">
        <span class="label with-tip">索引<InfoTip label="索引说明"><p>从影片信息重建媒体库列表。</p></InfoTip></span>
        <span class="grow"></span>
        <button class="btn sm" onclick={() => app.rebuildIndex()} disabled={!app.settings?.library_root || app.rebuild.running || rootChanging}>
          <Icon name="refresh" size={14} />{app.rebuild.running ? `重建中 ${app.rebuild.done}/${app.rebuild.total}` : '重建索引'}
        </button>
      </div>
    </section>

    <AppearanceSettings />

    {:else if tab === 'organize'}
    <section class="card">
      <h2>文件整理</h2>
      <div class="row">
        <label class="label" for="organize-mode">整理方式</label>
        <select id="organize-mode" class="input" bind:value={s.organize_mode} aria-label="整理方式">
          <option value="move">移动（默认）</option><option value="copy">复制，保留源</option><option value="hardlink">硬链接，保留源（同卷）</option><option value="symlink">软链接，保留源</option>
        </select>
      </div>
      {#if s.organize_mode !== 'move'}<p class="hint">视频与字幕保留在原位置，执行和撤销都不会清理源目录。{s.organize_mode === 'hardlink' ? '硬链接共享内容，修改任一文件会影响另一文件；跨卷会明确失败，请选择复制。' : s.organize_mode === 'symlink' ? '软链接依赖源位置，移动、删除或修改源文件会阻止播放；源盘离线时保留库索引。' : '复制需要与媒体大小相同的新增空间。'}</p>{/if}
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" disabled={s.organize_mode !== 'move'} bind:checked={s.cleanup_source} />
        <span>整理后清理原文件夹</span>
      </label><InfoTip label="整理后清理原文件夹说明"><p>仅将已整理完的源子文件夹移到废纸篓，可找回。</p></InfoTip></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.download_extrafanart} />
        <span>下载剧照</span>
      </label><InfoTip label="下载剧照说明"><p>最多 12 张，开启后整理会更慢。</p></InfoTip></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.notify_on_finish} />
        <span>任务完成时发系统通知</span>
      </label><InfoTip label="任务完成时发系统通知说明"><p>批量刮削、整理和重建索引结束时，如果窗口不在前台就提醒。</p></InfoTip></div>
    </section>

    {:else if tab === 'naming'}
    <section class="card">
      <div class="tipped"><h2>命名模板</h2><InfoTip label="模板语法与占位符"><p>actors/actresses 用「、」连接；runtime 为分钟，score 为 0–100 平均分。raw_name 是来源文件无扩展名名称，raw_dir 是来源父文件夹名称；ext 仅能用于目录模板，文件扩展名自动附加。每个名称最多 240 UTF-8 字节，实际截短将在整理预览中显示。</p><p>
        占位符:{'{code} {number} {title} {original_title} {actor} {actors} {actress} {actresses} {year} {release} {studio} {publisher} {series} {directors} {runtime} {score} {content_type} {prefix} {suffix} {raw_name} {raw_dir} {ext}'};{'{actor}'} 取首个女优;目录模板用 / 分层。
        文件相位(可空):{'{cd?}'} 分集号、{'{sub?}'} 中字为 C、{'{mosaic?}'} 为 censored / uncensored / cracked / leaked、{'{def?}'} 清晰度。
        可选组 <span class="mono">[ - {'{series}'}]</span>:值缺失、或组内可空占位符全为空时整段省略;<span class="mono">[[…]]</span> 保留时外加方括号。
        值映射 <span class="mono">{'{mosaic?|cracked=U,censored=}'}</span> 把原值改写成自定义文字。
      </p>
<p>默认文件名保留分集与中字/破解/流出标记,如 <span class="mono">SSIS-001-UC.mp4</span>、<span class="mono">SSIS-001-CD2-C.mp4</span>;整理后再扫描或重建索引仍能识别。</p></InfoTip></div>
      <div class="row">
        <label class="label" for="tpl">目录模板</label>
        <input id="tpl" class="input grow mono" type="text" bind:value={s.dir_template} />
      </div>
      <div class="row">
        <label class="label" for="file-tpl">文件名模板</label>
        <input id="file-tpl" class="input grow mono" type="text" spellcheck="false" bind:value={s.file_template} disabled={s.plex.naming} />
        <button class="btn sm ghost" type="button" disabled={s.plex.naming || s.file_template === DEFAULT_FILE_TPL} onclick={() => (s!.file_template = DEFAULT_FILE_TPL)}>恢复默认</button>
      </div>
      {#if templateSample}<div aria-label="模板实时预览">{#if templateSample.error}<p role="alert">{templateSample.error}</p>{:else}<p class="hint">样例目录：{templateSample.directory}<br/>样例文件：{templateSample.file}</p>{/if}{#each templateSample.warnings as warning}<p>{warning}</p>{/each}</div>{/if}
      <div class="row"><span class="label with-tip"><label for="nfo-language">新整理 NFO 语言</label><InfoTip label="NFO 语言与播放器说明"><p>设置保存后用于新整理计划。已有影片请在详情中预览并导出；缺少对应语言时回退当前文本，人工锁定字段优先。Plex 使用文件名及海报；Kodi 使用 movie.nfo 时需启用「影片位于匹配标题的独立目录」。</p></InfoTip></span><select id="nfo-language" class="input" bind:value={s.nfo_language}><option value="current">当前文本</option><option value="zh-Hans">简体中文</option><option value="original">来源原文</option></select></div>
      {#if s.plex.naming}<p class="hint">已开启「Plex 友好命名」,文件名固定为 <span class="mono">番号-标记 标题 - cdN</span>,此模板不生效。</p>{/if}

    </section>
    {:else if tab === 'scan'}
    <section class="card">
      <div class="tipped"><h2>扫描</h2><InfoTip label="扫描说明"><p>命中规则的视频会跳过，可在扫描结果中查看。</p></InfoTip></div>
      <div class="row">
        <span class="label with-tip"><label for="scan-extra">样片 / 预告片</label><InfoTip label="扫描规则写法"><p>使用正则匹配文件名（含扩展名）；(?i) 表示不分大小写。样片规则留空则不跳过，黑名单每行一条。</p></InfoTip></span>
        <input id="scan-extra" class="input grow mono" type="text" spellcheck="false" placeholder="留空则不跳过" bind:value={s.scan.extra_pattern} />
        <button class="btn sm ghost" type="button" disabled={s.scan.extra_pattern === DEFAULT_EXTRA} onclick={() => (s!.scan.extra_pattern = DEFAULT_EXTRA)}>恢复默认</button>
      </div>
      <div class="row top">
        <label class="label" for="scan-black">黑名单<span class="sub">广告等,每行一条</span></label>
        <textarea id="scan-black" class="input grow mono" rows="3" spellcheck="false" placeholder={'例:最新地址\n(?i)^ads?[-_. ]'} value={s.scan.blacklist.join('\n')} oninput={(e) => (s!.scan.blacklist = e.currentTarget.value.split('\n').map((l) => l.trim()).filter(Boolean))}></textarea>
      </div>
      <div class="row">
        <span class="label with-tip"><label for="scan-min">最小体积</label><InfoTip label="最小体积说明"><p>0 为不限制，短片建议保持 0。</p></InfoTip></span>
        <input id="scan-min" class="input num" type="number" min="0" step="10" value={s.scan.min_video_mb} oninput={(e) => (s!.scan.min_video_mb = Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)))} />
        <span class="faint hint2">MB</span>
      </div>
    </section>

    {:else if tab === 'badges'}
    <section class="card">
      <div class="tipped"><label class="switch">
        <input class="checkbox" type="checkbox" bind:checked={s.badges.enabled} />
        <span>给封面加角标</span>
      </label><InfoTip label="给封面加角标说明"><p>只影响之后整理的封面。同一角的角标自动叠放。</p></InfoTip></div>
      <div class="badge-layout">
        <div class="grow">
          <div class="row">
            <span class="label">贴在</span>
            <label class="switch inline" title="poster.jpg"><input class="checkbox" type="checkbox" bind:checked={s.badges.poster} disabled={!s.badges.enabled} /><span>海报</span></label>
            <label class="switch inline" title="fanart.jpg,Plex / Jellyfin 用作背景"><input class="checkbox" type="checkbox" bind:checked={s.badges.fanart} disabled={!s.badges.enabled} /><span>横版封面</span></label>
          </div>
          <div class="row">
            <span class="label with-tip"><label for="badge-scale">大小</label><InfoTip label="角标大小说明"><p>按图片高度计算比例。</p></InfoTip></span>
            <input id="badge-scale" class="grow range" type="range" min="0.03" max="0.25" step="0.01" bind:value={s.badges.scale} disabled={!s.badges.enabled} />
            <span class="num-out mono">{Math.round(s.badges.scale * 100)}%</span>
          </div>
          {#each KINDS as k (k.key)}
            <div class="row">
              <label class="label" for="badge-{k.key}">{k.label}</label>
              <select id="badge-{k.key}" class="input" bind:value={s.badges.corners[k.key]} disabled={!s.badges.enabled}>
                {#each CORNERS as c (c.key)}<option value={c.key}>{c.label}</option>{/each}
                <option value={null}>不显示</option>
              </select>
            </div>
          {/each}
          <div class="tipped"><label class="switch">
            <input class="checkbox" type="checkbox" bind:checked={s.badges.external_subtitle} disabled={!s.badges.enabled} />
            <span>外挂中文字幕也算中字</span>
          </label><InfoTip label="外挂中文字幕也算中字说明"><p>视频文件名没有 -C 等标记，但随迁的字幕名带 .chs/.cht/.zh 等或含「中文」时，同样贴中字角标。</p></InfoTip></div>
        </div>
        <div class="badge-preview" class:off={!s.badges.enabled} style="--bh: {(187 * s.badges.scale).toFixed(1)}px" aria-hidden="true">
          {#each CORNERS as c (c.key)}
            <div class="stack {c.key}">
              {#each KINDS.filter((k) => s!.badges.corners[k.key] === c.key) as k (k.key)}<span class="mark {k.key}">{k.stamp}</span>{/each}
            </div>
          {/each}
        </div>
      </div>
      <div class="tip-links">
        <span class="with-tip">识别规则<InfoTip label="识别规则"><p>文件名或上级文件夹名中出现以下写法时贴对应角标(不分大小写)。普通有码影片不贴。多分片影片任一分片带标记即贴,清晰度取最高档。</p>
      <ul class="rules">
        <li><span class="mark subtitle">字幕</span><span class="mono">-C</span><span class="mono">-UC</span><span class="mono">-CH</span><span>字幕 / 中文</span></li>
        <li><span class="mark uncensored">无码</span><span>无码番号(HEYZO、六位日期号等)/ 无码 / uncensored</span></li>
        <li><span class="mark cracked">破解</span><span class="mono">-U</span><span class="mono">-UC</span><span class="mono">-UMR</span><span>破解 / cracked</span></li>
        <li><span class="mark leaked">流出</span><span class="mono">-LEAK</span><span class="mono">-LEAKED</span><span>流出</span></li>
        <li><span class="mark definition">4K</span><span class="mono">4K</span><span class="mono">8K</span><span class="mono">2160p</span><span>内置 4K / 8K;其他清晰度需自备角标图</span></li>
      </ul>
      <p>例:<span class="mono">SSIS-001-UC-4K.mp4</span> → 字幕 + 破解 + 4K;<span class="mono">HEYZO-1234-leak.mp4</span> → 无码 + 流出。破解与流出同时出现时只贴破解。</p></InfoTip></span>
        <span class="with-tip">自定义角标图<InfoTip label="自定义角标图"><p>在下面的文件夹里放同名 PNG(透明背景)即可替换内置样式;清晰度还可以补上内置没有的档位。</p>
      {#if stampDir}<div class="row"><span class="label">文件夹</span><span class="grow root mono">{stampDir}</span></div>{/if}
      <dl class="stamp-files">
        {#each [['subtitle.png', '中字'], ['uncensored.png', '无码'], ['cracked.png', '破解'], ['leaked.png', '流出'], ['4k.png / 8k.png', '清晰度(内置)'], ['1080p.png、720p.png、hd.png…', '其他清晰度,内置没有,放了才贴']] as [f, d] (f)}
          <dt class="mono">{f}</dt><dd>{d}</dd>
        {/each}
      </dl></InfoTip></span>
      </div>
    </section>

    {:else if tab === 'sources'}
    <SourceChecks {dirty} />
    <section class="card">
      <div class="tipped"><h2>站点</h2><InfoTip label="站点说明"><p>启用站点后需加入路线并保存。Cookie 明文保存在本机。</p></InfoTip></div>
      {#each s.sources as src (src.id)}
        <div class="source-entry">
        <div class="row">
          <label class="switch">
            <input class="checkbox" type="checkbox" bind:checked={src.enabled} />
            <span>{name(src.id)}</span>
          </label>
          {#if SOURCE_HINTS[src.id]}<InfoTip label="{name(src.id)} 说明"><p>{SOURCE_HINTS[src.id]}</p></InfoTip>{/if}
          <input class="input grow" type="text" bind:value={src.base} disabled={!src.enabled} aria-label="{name(src.id)} 域名" />
        </div>
        {#if SOURCE_HINTS[src.id]}
          {#if src.enabled}
            <div class="row">
              <span class="hint">{sourceRoutes(src.id) ? `已加入路线：${sourceRoutes(src.id)}` : '尚未加入任何路线，启用后仍不会请求。'}</span>
              <button class="btn sm ghost" type="button" onclick={() => addSourceRoutes(src.id)}>加入支持类型路线</button>
              <button class="btn sm ghost" type="button" onclick={editSourceRules}>编辑路线与字段优先级</button>
            </div>
          {/if}
        {/if}
        {#if src.id === 'dmm'}
          <div class="row">
            <label class="label" for="dmm-api">数字影片 API</label>
            <input id="dmm-api" class="input grow" disabled={!src.enabled} placeholder="https://api.video.dmm.co.jp/graphql" value={src.api_base ?? ''} oninput={(e) => (src.api_base = e.currentTarget.value.trim() || undefined)} />
          </div>
        {/if}
        {#if src.id !== 'javdb' && SOURCE_HINTS[src.id] && src.enabled}
          <div class="row">
            <label class="label" for={`cookie-${src.id}`}>Cookie（可选）</label>
            <input id={`cookie-${src.id}`} class="input grow mono" type={showCookie ? 'text' : 'password'} autocomplete="off" spellcheck="false" value={src.cookie ?? ''} oninput={(e) => (src.cookie = e.currentTarget.value.trim() || undefined)} />
            <button class="btn sm ghost" type="button" onclick={() => (showCookie = !showCookie)}>{showCookie ? '隐藏' : '显示'}</button>
          </div>
        {/if}
        {#if src.id === 'javdb' && src.enabled}
          <div class="cookie" transition:slide={{ duration: dur(220) }}>
            <div class="row">
              <span class="label with-tip"><label for="javdb-cookie">登录 Cookie<span class="sub">可选</span></label><InfoTip label="Cookie 获取方法"><p>FC2 等影片的详情页要求登录。获取方法:在浏览器登录 JavDB → 打开开发者工具「网络」面板 → 刷新页面 → 点第一个请求,复制请求头里 <span class="mono">Cookie</span> 的整行内容粘贴到这里。Cookie 以明文保存在本机设置文件里;过期后刮削会提示重新填写。</p></InfoTip></span>
              <input
                id="javdb-cookie" class="input grow mono" type={showCookie ? 'text' : 'password'} autocomplete="off" spellcheck="false"
                placeholder="_jdb_session=…"
                value={src.cookie ?? ''} oninput={(e) => (src.cookie = e.currentTarget.value.trim() || undefined)}
              />
              <button class="btn sm ghost" type="button" onclick={() => (showCookie = !showCookie)}>{showCookie ? '隐藏' : '显示'}</button>
              <span class="state" class:ok={!!src.cookie}>{src.cookie ? '已填写' : '未登录'}</span>
            </div>
          </div>
        {/if}
        {#if src.enabled}
          <div class="row">
            <label class="label" for={`proxy-${src.id}`}>代理</label>
            <select id={`proxy-${src.id}`} class="input" aria-label="{name(src.id)} 代理" value={src.proxy?.mode ?? 'global'}
              onchange={(e) => setSourceProxy(src, e.currentTarget.value as SourceProxy['mode'])}>
              <option value="global">跟随全局代理</option>
              <option value="direct">直连</option>
              <option value="custom">自定义</option>
            </select>
            {#if src.proxy?.mode === 'custom'}
              <input class="input grow" type="text" aria-label="{name(src.id)} 代理地址" placeholder="http://127.0.0.1:7890"
                value={src.proxy.url} oninput={(e) => (src.proxy = { mode: 'custom', url: e.currentTarget.value.trim() })} />
            {/if}
          </div>
          {#if s.solver.url.trim() && src.id !== 'dmm'}
            <label class="switch extra">
              <input class="checkbox" type="checkbox" aria-label="{name(src.id)} 经求解器访问" checked={!!src.solver}
                onchange={(e) => { if (e.currentTarget.checked) src.solver = true; else delete src.solver }} />
              <span>经求解器访问</span>
            </label>
          {/if}
          <SourceAuth sourceId={src.id} label={name(src.id)} settings={s} />
        {/if}
        </div>
      {/each}
    </section>

    <section class="card">
      <h2>网络</h2>
      <div class="row">
        <span class="label with-tip"><label for="interval">同站点请求间隔(秒)</label><InfoTip label="请求间隔说明"><p>建议至少 5 秒，过快可能被站点限制。</p></InfoTip></span>
        <input id="interval" class="input num tnum" type="number" min="1" bind:value={s.min_interval_secs} />
      </div>
      <div class="row">
        <label class="label" for="proxy">代理</label>
        <input
          id="proxy" class="input grow" type="text" placeholder="http://127.0.0.1:7890(留空不使用)"
          value={s.proxy ?? ''} oninput={(e) => (s!.proxy = e.currentTarget.value.trim() || null)}
        />
      </div>
      <div class="row">
        <span class="label with-tip"><label for="solver">求解器地址<span class="sub">可选</span></label><InfoTip label="求解器说明"><p>给被 Cloudflare 挑战拦住的站点（如 JavLibrary）用。需要你自己运行 FlareSolverr，kiroku 只调用这个地址；在上面的站点里单独开启「经求解器访问」。图片仍直接下载。</p></InfoTip></span>
        <input
          id="solver" class="input grow mono" type="text" placeholder="http://127.0.0.1:8191/v1"
          value={s.solver.url} oninput={(e) => (s!.solver.url = e.currentTarget.value.trim())}
        />
      </div>
      {#if s.solver.url.trim()}
        <div class="row">
          <label class="label" for="solver-timeout">求解超时(秒)</label>
          <input id="solver-timeout" class="input num tnum" type="number" min="1" bind:value={s.solver.timeout_secs} />
        </div>
      {/if}
    </section>

    {:else if tab === 'advanced'}
    <section class="card">
      <div class="tipped"><h2>日志</h2><InfoTip label="日志说明"><p>记录运行过程，方便排查问题。</p></InfoTip><span class="tip-links"><span class="with-tip">设置文件位置<InfoTip label="设置文件位置"><p class="mono file-path">{path}</p></InfoTip></span></span></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.log.app} />
        <span>在 App 内显示</span>
      </label><InfoTip label="在 App 内显示说明"><p>在整理进度中查看、复制日志。</p></InfoTip></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.log.file} />
        <span>写入日志文件</span>
      </label><InfoTip label="写入日志文件说明"><p>按天保存为 <span class="mono">kiroku-日期.log</span>{#if logDir},位于 <span class="mono">{logDir}</span>{/if}。</p></InfoTip></div>
      <div class="row"><button class="btn sm" type="button" onclick={() => api.openLogDir().catch((e) => (error = String(e)))}><Icon name="folder" size={14} />打开日志文件夹</button></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.log.console} />
        <span>输出到终端</span>
      </label><InfoTip label="输出到终端说明"><p>从终端启动应用时可见。</p></InfoTip></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.log.verbose} />
        <span>详细模式</span>
      </label><InfoTip label="详细模式说明"><p>记录请求详情，日志会更多。</p></InfoTip></div>
    </section>

    <LibraryMaintenance {dirty} onpolicySaved={() => { if (s && app.settings) { s.retention = { ...app.settings.retention }; saved = JSON.stringify(s) } }} />
    <details id="settings-routes" class="card help" open={rulesOpen}>
      <summary>内容类型路由<InfoTip label="内容类型路由说明"><p>番号会被识别为某种内容类型,只请求该类型路由里启用的站,按顺序排列。</p></InfoTip></summary>
      {#each CONTENT_TYPES as ct (ct.key)}
        <div class="field">
          <div class="field-name">{ct.label}<span class="sub">{ct.hint}</span></div>
          <SiteList list={s.routes[ct.key]} all={ids} {name} empty="未配置,该类型不会刮削" />
        </div>
      {/each}
    </details>

    <details class="card help" open={rulesOpen}>
      <summary>字段优先级<InfoTip label="字段优先级说明"><p>每个字段取链上第一个非空来源(封面、预告片等 URL 类字段沿链合并)。「提前」把站点排到路由最前;「屏蔽」让该字段不用某站,与「提前」冲突时屏蔽优先。</p></InfoTip></summary>
      <div class="grid head"><span>字段</span><span>提前</span><span>屏蔽</span></div>
      {#each FIELDS as f (f.key)}
        <div class="field grid">
          <div class="field-name">{f.label}</div>
          <SiteList list={s.field_priority[f.key]} all={ids} {name} empty="默认" />
          <SiteList list={s.field_blacklist[f.key]} all={ids} {name} empty="无" />
        </div>
      {/each}
    </details>

    {:else if tab === 'ai'}
    <section class="card">
      <AiSettings bind:cfg={s.ai} />
      <div class="row">
        <span class="with-tip grow">译文缓存<InfoTip label="译文缓存说明"><p>已翻译的标题和简介会缓存在本机，同一服务、模型和生成选项下的原文可复用缓存。</p></InfoTip></span>
        <button class="btn sm" type="button" onclick={clearTranslationCache}>清空译文缓存</button>
      </div>
    </section>

    {:else if tab === 'plex'}
    <section class="card">
      <div class="tipped"><h2>Plex</h2><InfoTip label="Plex 媒体库配置方法"><p>Plex 不读 NFO:它按文件名识别片名,用目录里的 poster.jpg / fanart.jpg 作封面和背景。建议在 Plex 里把媒体库文件夹建成影片库,代理选「Personal Media / 个人媒体」并勾选「使用本地资源」。</p></InfoTip></div>
      <div class="tipped extra"><label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.plex.naming} />
        <span>Plex 友好命名</span>
      </label><InfoTip label="Plex 友好命名说明"><p>之后整理的文件名包含标题与分集号，并覆盖自定义文件名模板。</p></InfoTip></div>
      <label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.plex.enabled} />
        <span>整理和撤销后通知 Plex 刷新</span>
      </label>
      {#if s.plex.enabled}
      <div class="plexbox" transition:slide={{ duration: dur(240) }}>
        <div class="row">
          <label class="label" for="plex-url">服务器地址</label>
          <input id="plex-url" class="input grow" type="text" placeholder="http://127.0.0.1:32400" bind:value={s.plex.url} disabled={!s.plex.enabled} />
        </div>
        <div class="row">
          <span class="label with-tip"><label for="plex-token">X-Plex-Token</label><InfoTip label="如何获取 Plex Token"><p>在 Plex 网页版任一影片上点「…→ 获取信息 → 查看 XML」,地址栏里 <span class="mono">X-Plex-Token=</span> 后面那串就是。</p></InfoTip></span>
          <input id="plex-token" class="input grow mono" type={showToken ? 'text' : 'password'} autocomplete="off" spellcheck="false" bind:value={s.plex.token} disabled={!s.plex.enabled} />
          <button class="btn sm ghost" type="button" onclick={() => (showToken = !showToken)} disabled={!s.plex.enabled}>{showToken ? '隐藏' : '显示'}</button>
          <button class="btn sm" onclick={testPlex} disabled={!s.plex.enabled || plexTesting}>
            <Icon name={plexTesting ? 'loader' : 'refresh'} size={14} />测试连接
          </button>
        </div>
        {#if plexError}<p class="plex-err" role="alert">{plexError}</p>{/if}
        {#if plexResult}
          <ul class="sections">
            {#each plexResult.sections as sec (sec.key)}
              <li class:hit={sec.title === plexResult.matched}>
                <span class="sec-title">{sec.title}</span>
                <span class="faint">{sec.kind}</span>
                <span class="faint mono loc">{sec.locations.join(' · ')}</span>
                {#if sec.title === plexResult.matched}<span class="tag">包含媒体库</span>{/if}
              </li>
            {/each}
          </ul>
          {#if !plexResult.matched}<p class="plex-err">连接成功,但没有分区包含当前媒体库目录。请在 Plex 中把媒体库文件夹加入一个影片库,否则刷新不会生效。</p>{/if}
        {/if}
      </div>
      {/if}
    </section>
    <section class="card" aria-label="Jellyfin 与 Emby">
      <div class="tipped"><h2>Jellyfin / Emby</h2><InfoTip label="Jellyfin / Emby 配置方法"><p>Jellyfin 和 Emby 读取影片目录里的 NFO、海报与背景图。在服务器里把媒体库文件夹建成「电影」库并启用 NFO 元数据读取；API Key 在服务器后台「API 密钥」中创建。</p></InfoTip></div>
      <label class="switch extra">
        <input class="checkbox" type="checkbox" bind:checked={s.media_server.enabled} />
        <span>整理和撤销后通知 Jellyfin / Emby 刷新</span>
      </label>
      {#if s.media_server.enabled}
      <div class="plexbox" transition:slide={{ duration: dur(240) }}>
        <div class="row">
          <label class="label" for="server-kind">服务器类型</label>
          <select id="server-kind" class="input" bind:value={s.media_server.kind}><option value="jellyfin">Jellyfin</option><option value="emby">Emby</option></select>
        </div>
        <div class="row">
          <label class="label" for="server-url">服务器地址</label>
          <input id="server-url" class="input grow" type="text" placeholder="http://127.0.0.1:8096" bind:value={s.media_server.url} />
        </div>
        <div class="row">
          <label class="label" for="server-key">API Key</label>
          <input id="server-key" class="input grow mono" type={showServerKey ? 'text' : 'password'} autocomplete="off" spellcheck="false" bind:value={s.media_server.api_key} />
          <button class="btn sm ghost" type="button" onclick={() => (showServerKey = !showServerKey)}>{showServerKey ? '隐藏' : '显示'}</button>
          <button class="btn sm" onclick={testServer} disabled={serverTesting}>
            <Icon name={serverTesting ? 'loader' : 'refresh'} size={14} />测试连接
          </button>
        </div>
        <div class="row">
          <span class="label with-tip"><label for="server-local">本机路径前缀</label><InfoTip label="路径前缀说明"><p>服务器在 Docker 或 NAS 上看到的路径与本机不同时填写，两者都留空表示路径相同。</p></InfoTip></span>
          <input id="server-local" class="input grow mono" type="text" placeholder="/Volumes/NAS/av" spellcheck="false" bind:value={s.media_server.local_prefix} />
        </div>
        <div class="row">
          <label class="label" for="server-remote">服务器路径前缀</label>
          <input id="server-remote" class="input grow mono" type="text" placeholder="/media/av" spellcheck="false" bind:value={s.media_server.server_prefix} />
        </div>
        {#if serverError}<p class="plex-err" role="alert">{serverError}</p>{/if}
        {#if serverResult}
          <p class="hint">已连接 {serverResult.serverName || serverName} {serverResult.version}</p>
          <ul class="sections">
            {#each serverResult.libraries as lib (lib.name)}
              <li class:hit={lib.name === serverResult.matched}>
                <span class="sec-title">{lib.name}</span>
                <span class="faint mono loc">{lib.locations.join(' · ')}</span>
                {#if lib.name === serverResult.matched}<span class="tag">包含媒体库</span>{/if}
              </li>
            {/each}
          </ul>
          {#if !serverResult.matched}<p class="plex-err">连接成功，但没有媒体库包含当前媒体库目录。请检查路径前缀映射，或在 {serverName} 中把媒体库文件夹加入一个媒体库，否则刷新不会生效。</p>{/if}
        {/if}
      </div>
      {/if}
    </section>
    {/if}
      </div>
      {/key}
    </div>
  {:else if error}
    <div class="error" role="alert">无法读取设置：{error}</div>
    <button class="btn" onclick={load} disabled={loading}>重试</button>
  {:else}
    <p class="muted">加载中…</p>
  {/if}
</div>

<style>
  .settings-search { position:relative; margin:0 var(--s5) var(--s3); flex-shrink:0; }
  .settings-search > input { width:100%; }
  .search-results { position:absolute; top:calc(100% + 4px); left:0; right:0; z-index:10; background:var(--surface-1); border:1px solid var(--line-strong); border-radius:var(--r-md); box-shadow:var(--shadow-2); padding:var(--s3); max-height:45vh; overflow:auto; }
  .search-results button { width:100%; height:auto; min-height:34px; justify-content:space-between; text-align:left; white-space:normal; }

  .page { max-width: 1000px; margin: 0 auto; padding: var(--s6) var(--s5) var(--s7); }
  header { display: flex; justify-content: space-between; align-items: flex-start; gap: var(--s4); margin-bottom: var(--s4); position: sticky; top: 0; z-index: 5; padding: var(--s3) 0; background: linear-gradient(var(--bg) 70%, transparent); }
  h1 { font-size: var(--fs-2xl); font-weight: 700; letter-spacing: -0.01em; }
  h2 { font-size: var(--fs-lg); font-weight: 600; margin-bottom: var(--s1); }
  .path { font-size: var(--fs-xs); margin-top: 2px; max-width: 460px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; gap: var(--s2); flex-shrink: 0; }
  .save.pending { animation: pop 320ms var(--ease-back); }

  /* 左侧分类 + 右侧内容 */
  .layout { display: grid; grid-template-columns: 148px minmax(0, 1fr); gap: var(--s5); align-items: start; }
  .cats {
    --row: 38px; --gap: 2px;
    position: sticky; top: 84px; display: flex; flex-direction: column; gap: var(--gap);
  }
  .cat {
    position: relative; z-index: 1; display: flex; align-items: center; gap: var(--s3);
    height: var(--row); padding: 0 var(--s3); border: 0; border-radius: var(--r-sm);
    background: transparent; color: var(--text-2); font-weight: 500; text-align: left;
    transition: color var(--d-fast) var(--ease-out), background var(--d-fast) var(--ease-out), transform var(--d-base) var(--ease-back);
  }
  .cat:hover:not(.active) { background: color-mix(in srgb, var(--surface-3) 70%, transparent); color: var(--text-1); }
  .cat:active { transform: scale(0.97); }
  .cat.active { color: var(--accent); background: var(--accent-soft); }
  .cat-label { flex: 1; transition: transform var(--d-base) var(--ease-out); }
  .cat:hover .cat-label { transform: translateX(2px); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: pop 300ms var(--ease-back); }
  .on { font-size: 10.5px; font-weight: 600; color: var(--ok); background: var(--ok-soft); padding: 1px 6px; border-radius: var(--r-pill); animation: pop 300ms var(--ease-back); }
  .pane { min-width: 0; }
  .pane-head { margin-bottom: var(--s4); }
  .pane-title { font-size: var(--fs-xl); font-weight: 700; margin: 0; }
  .pane-head p { font-size: var(--fs-sm); margin-top: 2px; }
  .more {
    display: inline-flex; align-items: center; gap: var(--s2); margin-top: var(--s3); padding: 4px 6px 4px 2px;
    border: 0; background: transparent; color: var(--text-2); font-size: var(--fs-sm); border-radius: var(--r-sm);
    transition: color var(--d-fast), background var(--d-fast);
  }
  .more:hover:not(:disabled) { color: var(--text-1); background: var(--surface-2); }
  .more :global(.rot) { transform: rotate(90deg) !important; }
  .more .faint { font-size: var(--fs-xs); }
  section { padding: var(--s4); margin-bottom: var(--s4); transition: background var(--d-base) var(--ease-out); }

  .field { transition: background var(--d-fast); border-radius: var(--r-sm); }
  .field:hover { background: color-mix(in srgb, var(--surface-2) 60%, transparent); }
  .hint { color: var(--text-2); font-size: var(--fs-sm); margin: var(--s1) 0 var(--s3); }
  .hint2 { font-size: var(--fs-sm); }
  /* 标题 / 开关 / 字段名后面跟说明图标 */
  .tipped { display: flex; align-items: center; gap: var(--s1); }
  .tipped.extra { margin-top: var(--s3); }
  .tipped > .switch { min-width: 0; }
  .tipped > h2 { margin-bottom: 0; }
  .with-tip { display: inline-flex; align-items: center; gap: var(--s1); }
  .tip-links { display: flex; flex-wrap: wrap; gap: var(--s1) var(--s4); margin-left: auto; color: var(--text-2); font-size: var(--fs-sm); }
  .card > .tip-links { margin-top: var(--s3); }
  .row { display: flex; align-items: center; gap: var(--s3); margin: var(--s2) 0; }
  .row.top { align-items: flex-start; }
  textarea.input { resize: vertical; min-height: 64px; padding-top: 6px; line-height: 1.5; }
  .label { min-width: 150px; color: var(--text-2); font-size: var(--fs-sm); }
  .grow { flex: 1; min-width: 0; }
  .root { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .num { width: 96px; }
  .switch { display: flex; align-items: center; gap: var(--s2); min-width: 150px; font-weight: 600; }
  .switch.extra { font-weight: 500; margin-top: var(--s3); }
  .field { display: flex; align-items: center; gap: var(--s4); padding: var(--s3) 0; border-top: 1px solid var(--line); }
  .field-name { width: 110px; flex-shrink: 0; font-weight: 600; }
  .sub { display: block; font-weight: 400; font-size: var(--fs-xs); color: var(--text-3); }
  .grid { display: grid; grid-template-columns: 110px 1fr 1fr; align-items: start; }
  .head { padding: 0 0 var(--s1); border-top: 0; color: var(--text-3); font-size: var(--fs-xs); font-weight: 500; }
  .switch.inline { min-width: 0; font-weight: 500; }
  .cookie { margin: 0 0 var(--s2) var(--s5); padding-left: var(--s4); border-left: 2px solid var(--line); }
  .cookie .label { min-width: 126px; }
  .state { font-size: var(--fs-xs); font-weight: 600; padding: 2px 8px; border-radius: var(--r-pill); background: var(--surface-3); color: var(--text-3); white-space: nowrap; transition: background var(--d-base), color var(--d-base); }
  .state.ok { background: var(--ok-soft); color: var(--ok); animation: pop 300ms var(--ease-back); }
  .badge-layout { display: flex; gap: var(--s5); align-items: flex-start; }
  .range { accent-color: var(--accent); }
  .num-out { width: 40px; text-align: right; font-size: var(--fs-sm); color: var(--text-2); }
  .badge-preview {
    --bh: 15px;
    position: relative; flex-shrink: 0; width: 132px; height: 187px; border-radius: var(--r-sm); overflow: hidden;
    background: linear-gradient(160deg, #6b4fa0, #c4547e 55%, #e8a87c); box-shadow: inset 0 0 0 1px var(--line);
    transition: opacity var(--d-base), filter var(--d-base);
  }
  .badge-preview.off { opacity: 0.45; filter: grayscale(1); }
  /* 与 badge.rs 一致:边距 = 高/8,间距 = 高/10(最小 4px 按原图比例缩小) */
  .stack { position: absolute; display: flex; flex-direction: column; gap: max(2px, calc(var(--bh) / 10)); --pad: max(2px, calc(var(--bh) / 8)); }
  .stack.top_left { top: var(--pad); left: var(--pad); align-items: flex-start; }
  .stack.top_right { top: var(--pad); right: var(--pad); align-items: flex-end; }
  .stack.bottom_left { bottom: var(--pad); left: var(--pad); flex-direction: column-reverse; align-items: flex-start; }
  .stack.bottom_right { bottom: var(--pad); right: var(--pad); flex-direction: column-reverse; align-items: flex-end; }
  .badge-preview .mark { height: var(--bh); line-height: var(--bh); font-size: calc(var(--bh) * 0.58); padding: 0 calc(var(--bh) * 0.45); }
  /* 颜色取自内置角标图(amane) */
  .mark { display: inline-block; padding: 1px 8px; border-radius: 999px; color: #fff; font-size: 11px; font-weight: 700; line-height: 1.6; white-space: nowrap; animation: pop 260ms var(--ease-back); }
  .mark.subtitle { background: #3dbd4a; }
  .mark.uncensored { background: #6c5ce0; }
  .mark.cracked { background: #d81b1b; }
  .mark.leaked { background: #d4b000; }
  .mark.definition { background: #ffd400; color: #111; border-radius: 2px; }
  .badge-preview .mark.definition { padding: 0 calc(var(--bh) * 0.15); }
  .rules { list-style: none; margin: 0 0 var(--s2); padding: 0; display: flex; flex-direction: column; gap: var(--s2); font-size: var(--fs-sm); }
  .rules li { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
  .rules li .mono { padding: 0 6px; border-radius: 4px; background: var(--surface-2); }
  .rules li .mark { width: 44px; text-align: center; margin-right: var(--s1); }
  .stamp-files { display: grid; grid-template-columns: max-content 1fr; gap: var(--s2) var(--s4); margin: var(--s3) 0 0; font-size: var(--fs-sm); }
  .stamp-files dt { padding: 0 6px; border-radius: 4px; background: var(--surface-2); justify-self: start; }
  .stamp-files dd { margin: 0; color: var(--text-2); }
  .plex-err { color: var(--danger); font-size: var(--fs-sm); margin-top: var(--s2); animation: rise 240ms var(--ease-out) both; }
  .sections { list-style: none; margin: var(--s3) 0 0; padding: 0; display: flex; flex-direction: column; gap: var(--s1); }
  .sections li {
    display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3); border-radius: var(--r-sm); background: var(--surface-2); font-size: var(--fs-sm);
    animation: rise 300ms var(--ease-out) both;
  }
  .sections li:nth-child(2) { animation-delay: 50ms; }
  .sections li:nth-child(3) { animation-delay: 100ms; }
  .sections li.hit { background: var(--ok-soft); }
  .sec-title { font-weight: 600; }
  .loc { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tag { color: var(--ok); font-size: var(--fs-xs); font-weight: 600; }
  .error {
    display: flex; align-items: center; gap: var(--s2); margin-bottom: var(--s4); padding: var(--s3) var(--s4);
    background: var(--danger-soft); color: var(--danger); border-radius: var(--r-sm);
  }
  .help { margin: var(--s3) 0; border-top: 1px solid var(--line); padding-top: var(--s3); }
  .help summary { cursor: pointer; color: var(--text-2); font-size: var(--fs-sm); font-weight: 600; }
  .help[open] summary { margin-bottom: var(--s3); color: var(--text-1); }
  .help.card { padding: var(--s4); margin: 0 0 var(--s4); }
  .file-path { overflow-wrap: anywhere; }
  #settings-routes { scroll-margin-top: 100px; }
  .source-entry { padding: var(--s3) 0; border-top: 1px solid var(--line); }
  .source-entry:first-of-type { border-top: 0; }
  @media (min-width: 761px) and (max-width: 1150px) {
    .row { flex-wrap: wrap; }
    .row > .label { flex-basis: 100%; }
    .row > .grow.input { flex-basis: 65%; }
    .badge-layout { flex-direction: column-reverse; }
  }
  /* 窄屏:分类变成顶部横向滚动的胶囊 */
  @media (max-width: 760px) {
    .layout { grid-template-columns: 1fr; gap: var(--s4); }
    .cats { position: static; flex-direction: row; overflow-x: auto; scrollbar-width: none; gap: var(--s1); }
    .cat { flex-shrink: 0; border-radius: var(--r-pill); }
    .cat.active { background: var(--accent-soft); }
    .cat-label { flex: none; }
  }
  @media (max-width: 720px) {
    .page { padding: var(--s4) var(--s4) 96px; }
    header { flex-wrap: wrap; }
    .field, .row { flex-wrap: wrap; gap: var(--s2); }
    .grid { grid-template-columns: 1fr; }
    .head { display: none; }
    .label { min-width: 0; width: 100%; }
    .badge-layout { flex-direction: column-reverse; }
  }
</style>
