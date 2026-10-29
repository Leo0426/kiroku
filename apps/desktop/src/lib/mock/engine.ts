import { normalizeAi, selectedAiProvider, validateAi, validateAiProvider } from '../ai'
import { assetUrl } from '../asset-url'
import { csvCell } from '../library-browser'
// 浏览器预览用的后端模拟:实现与 Tauri 相同的 Api 接口,并通过同一条事件总线发事件。
// 目的是不启动桌面壳也能走通完整闭环(扫描 → 刮削 → 计划 → 执行 → 浏览 → 回滚)。

import {projectNames,validateNames,mergeTags,EMPTY_RULES,NAME_FIELDS} from '../names'
// 与核心共用同一份内置标签译名
import tagPreset from '../../../../../crates/kiroku-core/assets/tag_preset.json'
import type { Api } from '../api'
import { FIELDS, EDITABLE_FIELDS, SOURCE_SUPPORT, isVideoPath, type FacetKind, type ContentType, type Metadata, type RevisionPreview } from '../types'
import { emitMock, listen } from '../events'
import type {
  ApplyEvent,
  Facets,
  FileInfo,
  JournalSummary,
  MovieCard,
  MovieDetail,
  Page,
  PendingList, PendingSummary,
  PlanItem,
  Query,
  ScanItem,
  LogLevel,
  ScrapeStatus,
  SkippedFile,
  SkipReason,
  Settings,
} from '../types'
import { genMovie, posterSvg, type MockMovie } from './data'

const KEY = 'kiroku-mock-settings-v2'

const DEFAULTS: Settings = {
  name_rules: structuredClone(EMPTY_RULES),
  sources: [
    { id: 'javdb', enabled: true, base: 'https://javdb.com' },
    { id: 'javbus', enabled: true, base: 'https://www.javbus.com' },
    { id: 'jav321', enabled: true, base: 'https://www.jav321.com' },
    { id: 'dmm', enabled: false, base: 'https://www.dmm.co.jp', api_base: 'https://api.video.dmm.co.jp/graphql' },
    { id: 'mgstage', enabled: false, base: 'https://www.mgstage.com' },
    { id: 'fc2', enabled: false, base: 'https://adult.contents.fc2.com' },
    { id: 'fc2ppvdb', enabled: false, base: 'https://fc2ppvdb.com' },
    { id: 'avsox', enabled: false, base: 'https://avsox.click' },
    { id: 'javlibrary', enabled: false, base: 'https://www.javlibrary.com' },
    { id: 'xcity', enabled: false, base: 'https://xcity.jp' },
    { id: 'avbase', enabled: false, base: 'https://www.avbase.net' },
  ],
  routes: {
    censored: ['javdb', 'javbus', 'jav321'],
    amateur: ['javdb', 'javbus', 'jav321'],
    uncensored: ['javdb', 'javbus'],
    fc2: ['javdb'],
    chinese: ['javdb'],
    western: ['javdb'],
    hentai: ['javdb'],
  },
  field_priority: { title: ['jav321'], original_title: ['jav321'], plot: ['jav321'], runtime_min: ['jav321'] },
  field_blacklist: {},
  min_interval_secs: 5,
  proxy: null,
  dir_template: '{actor}/{code}',
  nfo_language: 'current',
  file_template: '{code}[-CD{cd?}][-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}]',
  download_extrafanart: false,
  retention: {historyDays:90,logDays:90},
  organize_mode: 'move',
  cleanup_source: true,
  plex: { enabled: false, url: 'http://127.0.0.1:32400', token: '', naming: false },
  media_server: { enabled: false, kind: 'jellyfin', url: 'http://127.0.0.1:8096', api_key: '', local_prefix: '', server_prefix: '' },
  log: { console: true, app: true, file: false, verbose: false },
  scan: { extra_pattern: '(?i)sample|trailer|preview|预告|預告|样片|樣本|样本', blacklist: [], min_video_mb: 0 },
  badges: {
    enabled: false, scale: 0.08, poster: true, fanart: false, external_subtitle: true,
    corners: { subtitle: 'top_left', uncensored: 'top_left', cracked: 'top_left', leaked: 'top_left', definition: 'top_left' },
  },
  ai: normalizeAi(),
  library_root: null,
  source_dirs: [],
  notify_on_finish: true,
  solver: { url: '', timeout_secs: 60 },
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
const clone = <T,>(v: T): T => structuredClone(v)

const publicUrl=(value:string)=>{try{const url=new URL(value);url.username='';url.password='';return url.toString()}catch{return value}}
const identity=async(base:string,cookie:string|undefined,proxy:string|null,apiBase='')=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(JSON.stringify(apiBase?[base,cookie??'',proxy,apiBase]:[base,cookie??'',proxy]))))).map(v=>v.toString(16).padStart(2,'0')).join('')
const effectiveApi=(cfg:Settings['sources'][number])=>cfg.id==='dmm'?(cfg.api_base?.trim()||'https://api.video.dmm.co.jp/graphql'):(cfg.api_base?.trim()||'')
// Mirror Source::supports for factory-built sources; route membership alone is insufficient.
const sourceSupports=(id:string,code:string,contentType:string)=>{
  if(id==='javdb')return true
  if(id==='javbus')return contentType!=='fc2'
  if(id==='jav321')return contentType!=='fc2'&&!/^\d{6}[-_]\d{2,4}$/.test(code)
  if(!SOURCE_SUPPORT[id]?.includes(contentType as ContentType))return false
  return id!=='dmm'||/^[a-z]+[-_]?\d+$/i.test(code)
}
const sourceRoute=(code:string,contentType:string)=>(settings.routes[contentType]??[]).filter(id=>settings.sources.some(s=>s.id===id&&s.enabled)&&sourceSupports(id,code,contentType))
const sourceSnapshot=async(code:string,contentType:string,cfg:Settings['sources'][number],proxy:string|null,fields:Partial<Metadata>)=>({code,content_type:contentType,base:publicUrl(cfg.base),cache_identity:await identity(cfg.base,cfg.cookie,proxy,effectiveApi(cfg)),fetched_at:Math.floor(Date.now()/1000),fields:{...clone(fields),source_url:`${publicUrl(cfg.base).replace(/\/$/,'')}/movie/${code}`}})

// ---- 状态 -----------------------------------------------------------------------------------

let settings: Settings = (() => {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return clone(DEFAULTS)
    const saved = JSON.parse(raw) as Partial<Settings>
    // 与 Rust 端 serde(default) 一致:旧设置缺的角标字段补默认值
    const badges = { ...clone(DEFAULTS.badges), ...saved.badges }
    badges.corners = { ...DEFAULTS.badges.corners, ...saved.badges?.corners }
    const sources = saved.sources ? clone(saved.sources) : clone(DEFAULTS.sources)
    for (const source of DEFAULTS.sources) if (!sources.some(s => s.id === source.id)) sources.push(clone(source))
    return { ...clone(DEFAULTS), ...saved, sources, badges, ai: normalizeAi(saved.ai), name_rules: {...clone(DEFAULTS.name_rules),...saved.name_rules}, media_server: { ...clone(DEFAULTS.media_server), ...saved.media_server }, scan: { ...clone(DEFAULTS.scan), ...saved.scan }, log: { ...clone(DEFAULTS.log), ...saved.log } }
  } catch {
    return clone(DEFAULTS)
  }
})()

// 假装已有一个 600 部的媒体库;ABQ-100 固定存在,用来演示「目标已存在」冲突
const movies: MockMovie[] = (() => {
  const list = Array.from({ length: 600 }, (_, i) => genMovie(i))
  list[0] = { ...list[0], code: 'ABQ-100', missingPoster: true, title: '既存の作品', actors: ['演员壬'] }
  return list.filter((m, i, a) => a.findIndex((x) => x.code === m.code) === i)
})()

interface HistoryRec extends JournalSummary { appended?:{code:string;before:number;after:number}; added: string[]; revision?: { root: string; code: string; before: Metadata; after: Metadata; movie: MockMovie; imageBefore?:string; imageAfter?:string }; repaired?: { code: string; poster: boolean; fanart: boolean } }
const historyReads=new Map<string,{cancelled:boolean}>();const stoppedHistory=new Set<string>();let historyGeneration=0;
function stopHistory(id:string){if(!id||id.length>100)return false;const active=historyReads.get(id);if(active){active.cancelled=true;return true}stoppedHistory.add(id);if(stoppedHistory.size>128)stoppedHistory.delete(stoppedHistory.values().next().value!);return false;}
function invalidateHistory(nextRoot:Settings['library_root']){historyGeneration++;for(const r of historyReads.values())r.cancelled=true;historiesByRoot.set(settings.library_root,history.splice(0));history.push(...(historiesByRoot.get(nextRoot)??[]));}
const historiesByRoot=new Map<Settings['library_root'],HistoryRec[]>();
const history: HistoryRec[] = []
const edited = new Map<string, Metadata>()
const originalDirs = new Map<string, string>()
let catalogPending: import('../types').CatalogPreview | null = null
let catalogSettings=''
const foundMetadata=new Map<string,Metadata>()
let revision: RevisionPreview | null = null
let revisionSettings: Settings | null = null
const artworkImages=new Map<string,string>()
const artworkOriginals=new Map<string,number[]>()
let openedArtwork: {token:string;root:string;code:string;before:Metadata;settings:Settings;image:number[]}|null=null
let croppedArtwork:{token:string;image:string;original:number[]}|null=null
function canvasBytes(canvas:HTMLCanvasElement){return Array.from(atob(canvas.toDataURL('image/png').split(',')[1]),c=>c.charCodeAt(0))}
function bytesUrl(bytes:number[],type='image/png'){let binary='';for(const byte of bytes)binary+=String.fromCharCode(byte);return `data:${type};base64,${btoa(binary)}`}


let nextJob = 1
let job: { id: number; cancelled: boolean } | null = null

function begin(requestId?:string): number {
  if (job) throw '已有任务在运行'
  job = { id: nextJob++, cancelled: false }
  if(requestId!==undefined)emitMock('job:started',{jobId:job.id,requestId})
  return job.id
}
const end = () => {
  job = null
}

async function openMedia(kind:'play'|'reveal',code:string,part?:number,requestId?:string){
 const id=begin(requestId);try {
  if(job?.cancelled)throw '媒体校验已取消';
  const detail=await mockApi.libraryGet(code);if(!detail)throw '影片不在媒体库中';
  const total=detail.meta.placements?.some(p=>p.mode==='symlink')?1048576:0;
  emitMock('media:progress',{jobId:id,root:settings.library_root??'',code,kind,doneBytes:0,totalBytes:total});
  for(let step=1;step<=4;step++){await sleep(75);if(job?.cancelled)throw '媒体校验已取消';emitMock('media:progress',{jobId:id,root:settings.library_root??'',code,kind,doneBytes:Math.round(total*step/4),totalBytes:total});}
  console.info('[mock]',kind,code,part);
 }finally{end()}
}

// ---- 扫描素材 -------------------------------------------------------------------------------

const info = (number: string | null, over: Partial<FileInfo> = {}): FileInfo => ({
  number,
  content_type: 'censored',
  prefix: number?.split('-')[0] ?? '',
  cd: null,
  has_subtitle: false,
  mosaic: 'censored',
  definition: null,
  matched: number !== null,
  ...over,
})

const GB = 1024 ** 3
const SCAN_FILES: { name: string; size: number; number: string | null; over?: Partial<FileInfo> }[] = [
  { name: 'SMPL-421.mp4', size: 2.1 * GB, number: 'SMPL-421' },
  { name: 'SMPL-421 (1).mp4', size: 1.9 * GB, number: 'SMPL-421' }, // 与上一个同码 → 批内重名
  { name: 'MOVX-679-C.mp4', size: 3.4 * GB, number: 'MOVX-679', over: { has_subtitle: true } },
  { name: 'QRS-912-CD1.mp4', size: 2.2 * GB, number: 'QRS-912', over: { cd: 1 } },
  { name: 'QRS-912-CD2.mp4', size: 2.0 * GB, number: 'QRS-912', over: { cd: 2 } },
  { name: 'FC2-PPV-3012345.mp4', size: 1.4 * GB, number: 'FC2-3012345', over: { content_type: 'fc2', mosaic: null } },
  { name: 'HEYZO-2744.mkv', size: 1.1 * GB, number: 'HEYZO-2744', over: { content_type: 'uncensored', mosaic: 'uncensored' } },
  { name: '259JKL-1503.mp4', size: 1.6 * GB, number: '259JKL-1503', over: { content_type: 'amateur', mosaic: null } },
  { name: 'PQRST-808-4K.mp4', size: 6.8 * GB, number: 'PQRST-808', over: { definition: '4K' } },
  { name: 'PRVW-455.mp4', size: 2.5 * GB, number: 'PRVW-455' }, // 站点没有 → 未找到
  { name: 'WXYZ-377.mp4', size: 2.7 * GB, number: 'WXYZ-377' }, // 请求被拦 → 失败
  { name: 'ABQ-100.mp4', size: 2.3 * GB, number: 'ABQ-100' }, // 库里已有 → 目标已存在
  { name: 'random video.mp4', size: 0.8 * GB, number: null },
  { name: 'IMG_2231.mov', size: 0.3 * GB, number: null },
  // 默认规则下跳过(样片/预告片),不进入待整理列表
  { name: 'SMPL-421-trailer.mp4', size: 0.04 * GB, number: 'SMPL-421' },
  { name: 'MOVX-679 sample.mp4', size: 0.02 * GB, number: 'MOVX-679' },
]

/** Rust regex 的 `(?i)` 前缀在 JS 里换成 i 旗标;mock 只支持这一种内联旗标 */
function toRegExp(p: string): RegExp | null {
  if (!p.trim()) return null
  const ci = p.startsWith('(?i)')
  try {
    return new RegExp(ci ? p.slice(4) : p, ci ? 'i' : '')
  } catch {
    return null
  }
}

/** 与 kiroku-core video_filter 一致:黑名单 > 样片/预告 > 体积 */
function skipReason(name: string, size: number): SkipReason | null {
  const { extra_pattern, blacklist, min_video_mb } = settings.scan
  if (blacklist.some((p) => toRegExp(p)?.test(name))) return 'blacklisted'
  if (toRegExp(extra_pattern)?.test(name)) return 'extra'
  if (min_video_mb > 0 && size < min_video_mb * 1024 * 1024) return 'tooSmall'
  return null
}

/** 按当前设置拆分:正片(id 为其在列表中的序号)与跳过的文件 */
function splitScan(root: string): { kept: ScanItem[]; skipped: SkippedFile[] } {
  const kept: ScanItem[] = []
  const skipped: SkippedFile[] = []
  for (const f of SCAN_FILES) {
    const path = `${root}/${f.name}`
    const size = Math.round(f.size)
    const reason = skipReason(f.name, size)
    if (reason) {
      skipped.push({ path, size, reason })
      continue
    }
    const fi = info(f.number, f.over)
    kept.push({
      id: kept.length,
      path,
      size,
      info: fi,
      code: f.number
        ? { kind: f.number.startsWith('FC2') ? 'Fc2' : 'Standard', id: f.number, content_type: fi.content_type, part: fi.cd, subtitled: fi.has_subtitle, mosaic: fi.mosaic, definition: fi.definition }
        : null,
    })
  }
  return { kept, skipped }
}

/** 与后端一致:有真实详情页样本的来源才支持按地址刮削 */
const URL_SOURCES = ['javdb', 'javbus', 'jav321', 'fc2', 'xcity', 'javlibrary', 'avbase']
const hostOf = (u: string) => { try { const x = new URL(u); return /^https?:$/.test(x.protocol) ? x.hostname.replace(/^www\./, '').toLowerCase() : null } catch { return null } }
function urlSource(url: string, route: string[]): string {
  const host = hostOf(url)
  if (!host) throw '请输入 http(s) 开头的详情页地址'
  const cfg = settings.sources.find((c) => hostOf(c.base) === host)
  if (!cfg) throw `${host} 不是已配置的数据源域名`
  if (!cfg.enabled) throw `数据源 ${cfg.id} 未启用`
  if (!route.includes(cfg.id)) throw `数据源 ${cfg.id} 不在该类型影片的路线中,请先在设置里加入路线`
  if (!URL_SOURCES.includes(cfg.id)) throw `数据源 ${cfg.id} 不支持按详情页地址刮削`
  return cfg.id
}
/** 按地址刮削时本次视为命中;页面不含番号时附带提示 */
const pinnedUrl = new Map<string, string>()

/** 与后端 facets_of 对应;样例角标按色相确定性分配 */
function facetValues(m: MockMovie): Record<FacetKind, string[]> {
  const badges = [m.hue % 3 === 0 ? 'subtitle' : '', m.contentType === 'uncensored' ? 'uncensored' : '', m.hue % 5 === 0 ? '4k' : ''].filter(Boolean)
  return { tag: [...new Set(m.tags)], actor: [...new Set(m.actors)], studio: m.studio ? [m.studio] : [], series: m.series ? [m.series] : [], badge: badges, favorite: m.favorite ? ['yes'] : [] }
}

const OUTCOME: Record<string, ScrapeStatus> = {
  'PRVW-455': { kind: 'notFound' },
  'WXYZ-377': { kind: 'failed', message: 'javdb: blocked by anti-bot (status 403)' },
}

let scan: ScanItem[] = []
const found = new Map<string, MockMovie>()
const sourceText=new Map<string,import('../types').TextVersions>()
let plans: (PlanItem & { _movie?: MockMovie })[] = []

// ---- Api 实现 -------------------------------------------------------------------------------

async function imageVersion(bytes: string | null): Promise<string | null> {
  if (!bytes) return null
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(bytes))
  return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, '0')).join('')
}
const toCard = async (m: MockMovie): Promise<MovieCard> => {
  const poster = artworkImages.get(m.code) ?? (m.missingPoster ? null : posterSvg(m.code, m.hue))
  return ({
  code: m.code,
  title: m.title,
  year: m.year,
  actors: m.actors,
  poster,
  posterVersion: await imageVersion(poster),
  contentType: m.contentType,
  score: m.score,
  addedAt: m.addedAt,
  parts: m.parts,
  favorite: !!m.favorite,
})
}

function newMovie(code: string, contentType: string, parts: number): MockMovie {
  const seed = [...code].reduce((a, c) => a * 31 + c.charCodeAt(0), 7)
  const m = genMovie(seed % 100000)
  return { ...m, code, contentType, parts, year: 2025, releaseDate: '2025-03-14', addedAt: Math.floor(Date.now() / 1000) }
}

let maintenanceToken='';let maintenanceSettings='';let maintenanceCleaned=false;
let pendingAppend: import('../types').AppendPreview|null=null;let appendSettings='';
function movieMetadata(m:MockMovie):Metadata {return {code:m.code,title:m.title,original_title:m.originalTitle,plot:m.plot,release_date:m.releaseDate,runtime_min:m.runtime,studio:m.studio,publisher:m.publisher,series:m.series,actors:m.actors.map(name=>({name,gender:'female'})),tags:[...m.tags],directors:[],scores:[{site:'javdb',score:m.score}],source_urls:{javdb:'https://javdb.com/'},provenance:{title:'javdb',actors:'javbus'},locked_fields:[],favorite:!!m.favorite}}
function invalidateText(meta:Metadata,field:string){
 if(field!=='title'&&field!=='plot')return
 meta.text_versions??={original:{},localized:{}};meta.text_versions.original[field]=null
 for(const version of Object.values(meta.text_versions.localized))version[field]=null
 if(field==='title')meta.original_title=null
}
function recoverText(meta:Metadata){
 meta.text_versions??={original:{},localized:{}}
 for(const field of ['title','plot'] as const){if(meta.text_versions.original[field]!=null)continue
  if(field==='title'&&meta.original_title!=null){meta.text_versions.original.title=meta.original_title;continue}
  const source=meta.provenance?.[field],snapshot=source&&meta.raw_sources?.[source]
  if(snapshot&&snapshot.code===meta.code)meta.text_versions.original[field]=(field==='title'?snapshot.fields.original_title??snapshot.fields.title:snapshot.fields.plot)??null
 }
}
function projectText(input:Metadata){
 const meta=clone(input),warnings:string[]=[];recoverText(meta)
 if(!meta.original_title)meta.original_title=meta.text_versions!.original.title??null
 for(const field of ['title','plot'] as const){if(meta.locked_fields?.includes(field)||!meta.nfo_language||meta.nfo_language==='current')continue
  const version=meta.nfo_language==='original'?meta.text_versions!.original:Object.entries(meta.text_versions!.localized).find(([tag])=>tag.toLowerCase()==='zh-hans')?.[1]
  if(version?.[field]!=null)meta[field]=version[field]??null
  else warnings.push(`${meta.nfo_language==='original'?'原文':'简体中文'}${field==='title'?'标题':'简介'}未保存，回退当前值`)
 }return {meta,warnings}
}
let languageBatch:import('../types').LanguageBatchPreview|null=null,languageSettings:Settings|null=null
let editBatch:import('../types').LanguageBatchPreview|null=null,editSettings:Settings|null=null
const pendingCache = new Map<string, { key: string; at: number; value: PendingList | PendingSummary }>()
let pendingRequest: { requestId: string; id: number } | null = null
void listen('library:changed', () => pendingCache.clear())
async function readPending(preview: boolean, requestId: string = crypto.randomUUID(), force = false): Promise<PendingList | PendingSummary> {
  // With no sources there is no disk work or job to schedule.
  if (!settings.source_dirs.length) {
    const summary: PendingSummary = { dirs: [], total: 0, skippedTotal: 0, visited: 0, errors: [], errorsTotal: 0,
      cancelled: false, truncated: false, complete: true, checkedAt: Date.now(), cached: false }
    return preview ? { ...summary, items: [], codes: 0, unrecognized: 0 } : summary
  }
  const key = JSON.stringify([settings.source_dirs, settings.library_root, settings.scan])
  const kind = preview ? 'preview' : 'summary'
  const cached = pendingCache.get(kind)
  if (!force && cached?.key === key && Date.now() - cached.at < 30_000) return { ...clone(cached.value), cached: true }
  const id = begin()
  pendingRequest = { requestId, id }
  try {
    await sleep(20)
    const dirs = [...settings.source_dirs]
    const { kept, skipped } = dirs.length ? splitScan(dirs[0].replace(/\/$/, '')) : { kept: [], skipped: [] }
    const cancelled = !!job?.cancelled
    const summary: PendingSummary = { dirs, total: cancelled ? 0 : kept.length, skippedTotal: skipped.length,
      visited: cancelled ? 0 : kept.length + skipped.length, errors: [], errorsTotal: 0, cancelled, truncated: false,
      complete: !cancelled, checkedAt: Date.now(), cached: false }
    const items = cancelled ? [] : kept.slice(0, 2000)
    const value = preview ? { ...summary, items, codes: new Set(items.filter(i => i.code).map(i => i.code!.id)).size,
      unrecognized: items.filter(i => !i.code).length } : summary
    if (!cancelled && key === JSON.stringify([settings.source_dirs, settings.library_root, settings.scan])) pendingCache.set(kind, { key, at: Date.now(), value: clone(value) })
    return value
  } finally { if (pendingRequest?.id === id) pendingRequest = null; end() }
}

function matchingMovies(q: Query) {
    const words = (q.text ?? '').toLowerCase().split(/\s+/).filter(Boolean)
    return movies.filter((m) => {
      if (q.contentType && m.contentType !== q.contentType) return false
      if (q.year && m.year !== q.year) return false
      if (q.minScore !== undefined && (m.score == null || m.score < q.minScore)) return false
      if (q.missing === 'poster' && !m.missingPoster) return false
      if (q.missing === 'score' && m.score != null) return false
      if (q.missing === 'year' && m.year != null) return false
      const own = facetValues(m)
      if (!(q.filters ?? []).every((f) => own[f.kind].includes(f.value))) return false
      const hay = `${m.code} ${m.title} ${m.actors.join(' ')} ${m.studio} ${m.series ?? ''}`.toLowerCase()
      return words.every((w) => hay.includes(w))
    })
}

export const mockApi: Api = {
  async recoveryInspect(){const id=begin();try{if(!settings.library_root)throw '媒体库未设置';const list=history.map(({added:_a,...h})=>({...h}));return {root:settings.library_root,entries:list.filter(j=>j.error||j.status==='partial'||j.pendingOperations).map(j=>({id:j.id,kind:j.error?'corrupt' as const:j.pendingOperations?'pending' as const:'partial' as const,codes:j.codes,pendingOperations:j.pendingOperations,message:j.error?`日志无法安全读取：${j.error}`:j.pendingOperations?'有中断操作待核对':'上次操作部分完成',canUndo:!j.error&&/^\d+$/.test(j.id)}))}}finally{end()}},

  async catalogInventory(requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});emitMock('library:catalog',{jobId:id,phase:'inventory',done:0,total:0,code:''});if(!settings.library_root)throw '请先设置媒体库';const names:Record<string,Record<string,number>>={},list:import('../types').CatalogMovie[]=[];for(const m of movies){if(job?.cancelled)break;const d=await mockApi.libraryGet(m.code);if(!d)continue;const fields={actors:d.meta.actors.map(a=>a.name),tags:d.meta.tags,studio:d.meta.studio?[d.meta.studio]:[],publisher:d.meta.publisher?[d.meta.publisher]:[],series:d.meta.series?[d.meta.series]:[]};list.push({code:m.code,dir:d.dir,names:fields});for(const f of NAME_FIELDS)for(const name of new Set(fields[f])){names[f]??={};names[f][name]=(names[f][name]??0)+1}}return {movies:list,names,errors:[],cancelled:!!job?.cancelled}}finally{end()}},
  async catalogPreview(dirs,requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});catalogPending=null;emitMock('library:catalog',{jobId:id,phase:'preview',done:0,total:dirs.length,code:''});const items:import('../types').CatalogPreview['items']=[],warnings:string[]=[];for(const dir of new Set(dirs)){if(job?.cancelled)throw '预览已取消';const m=movies.find(m=>(originalDirs.get(m.code)??`${settings.library_root}/${m.actors[0]}/${m.code}`)===dir);if(!m)throw '影片目录不存在';const d=(await mockApi.libraryGet(m.code))!;if(job?.cancelled)throw '预览已取消';const p=projectNames(d.meta,settings.name_rules);warnings.push(...p.warnings);const changes=[...NAME_FIELDS,'title' as const].filter(f=>JSON.stringify(d.meta[f])!==JSON.stringify(p.metadata[f])).map(f=>({field:f,before:clone(d.meta[f]),after:clone(p.metadata[f])}));items.push({root:settings.library_root??'',dir,code:m.code,before:d.meta,after:p.metadata,changes})}catalogSettings=JSON.stringify(settings);catalogPending={token:String(id),root:settings.library_root??'',fingerprint:JSON.stringify(settings.name_rules),items,warnings};return clone(catalogPending)}finally{end()}},
  async catalogApply(token,requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});emitMock('library:catalog',{jobId:id,phase:'apply',done:0,total:0,code:''});const p=catalogPending;catalogPending=null;if(!p||p.token!==token||catalogSettings!==JSON.stringify(settings))throw '名称规则或设置已变化，请重新预览';const items:import('../types').CatalogResult['items']=[];for(const item of p.items){await sleep(15);if(job?.cancelled)break;const d=await mockApi.libraryGet(item.code);let error:string|null=null,journalId:string|null=null;if(!d||JSON.stringify(d.meta)!==JSON.stringify(item.before))error='预览后磁盘内容已变化';else if(JSON.stringify(item.before)!==JSON.stringify(item.after)){const m=movies.find(m=>m.code===item.code)!;journalId=`catalog-${Date.now()}-${id}-${items.length}`;history.unshift({id:journalId,ts:Date.now(),status:'applied',itemCount:1,opCount:2,codes:[item.code],trashed:0,pendingOperations:0,error:null,added:[],revision:{root:p.root,code:item.code,before:clone(item.before),after:clone(item.after),movie:clone(m)}});originalDirs.set(item.code,item.dir);edited.set(item.code,clone(item.after));Object.assign(m,{actors:item.after.actors.map(a=>a.name),tags:[...item.after.tags],studio:item.after.studio??'',publisher:item.after.publisher??'',series:item.after.series})}items.push({code:item.code,dir:item.dir,journalId,error});emitMock('library:catalog',{jobId:id,phase:'apply',done:items.length,total:p.items.length,code:item.code})}emitMock('library:changed',{jobId:id});return {jobId:id,items,cancelled:!!job?.cancelled,indexError:null}}finally{end()}},

  async maintenancePreview(includeRecent=false,requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});await sleep(20);if(job?.cancelled)throw '预览已取消';maintenanceToken=String(id);maintenanceSettings=JSON.stringify(settings);return {token:maintenanceToken,root:settings.library_root??'',logDir:'/mock/logs',policy:includeRecent?{historyDays:0,logDays:0}:clone(settings.retention),scannedAt:Math.floor(Date.now()/1000),candidateBytes:maintenanceCleaned?0:2048,protected:['现有影片原图与有效撤销记录受到保护'],candidates:maintenanceCleaned?[]:[{id:'original:unused',kind:'original',paths:[`${settings.library_root}/.kiroku-original-unused.png`],bytes:2048,description:'无引用的旧原图（模拟）'}]}}finally{end()}},
  async maintenanceApply(token,selected,requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});await sleep(20);if(token!==maintenanceToken||maintenanceSettings!==JSON.stringify(settings))throw '预览已过期或设置变化';maintenanceToken='';if(selected.some(s=>s!=='original:unused'))throw '未知候选';if(job?.cancelled)return {removed:0,skipped:[],failed:[],cancelled:true};maintenanceCleaned=!!selected.length;return {removed:selected.length,skipped:[],failed:[],cancelled:false}}finally{end()}},
  async libraryHealth(requestId){const id=begin();try{if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId});await sleep(250);return {root:settings.library_root??'',scannedAt:Math.floor(Date.now()/1000),scanned:movies.length,cancelled:!!job?.cancelled,issues:movies.filter(m=>m.missingPoster).map(m=>({kind:'missingArtwork',severity:'warning',dir:`${settings.library_root}/${m.code}`,code:m.code,path:`${settings.library_root}/${m.code}/poster.jpg`,message:'缺少 poster 图片',action:'repairImages'}))}}finally{end()}},
  async revealLibraryPath(path){if(!path.startsWith(settings.library_root??'!'))throw '路径不在库内';console.info('[mock] reveal',path)},
  async sourcesCheck(ids, draft, requestId) {
    const config = clone(draft ?? settings)
    const id = begin()
    try {
      if(requestId!==undefined)emitMock('job:started',{jobId:id,requestId})
      const sources = ids ?? [...new Set(Object.values(config.routes).flat())].filter(id => config.sources.some(s => s.id === id && s.enabled))
      await sleep(250)
      return [...new Set(sources)].map(sourceId => {
        const cancelled = !!job?.cancelled
        const disabled = !config.sources.some(s => s.id === sourceId && s.enabled)
        return { sourceId, outcome: { status: cancelled || disabled ? 'skipped' : 'ok', reason: cancelled ? 'cancelled' : disabled ? 'disabledOrUnknown' : sourceId === 'javdb' ? config.sources.find(s=>s.id===sourceId)?.cookie?.trim() ? 'authUnknown' : 'unauthenticated' : 'ok', httpStatus: cancelled || disabled ? null : 200 }, elapsedMs: 250, checkedAt: Math.floor(Date.now() / 1000) }
      })
    } finally { end() }
  },
  async pickMediaFiles(){const input=window.prompt('追加文件路径（浏览器模拟）');return input?[input]:[]},
  async appendTargetVideos(code,target,requestId){
    begin(requestId)
    try{const d=await mockApi.libraryGet(code);if(job?.cancelled)throw '已取消';if(!d||d.dir!==target)throw '目标目录不匹配';return d.files.map(f=>f.path)}finally{end()}
  },
  async appendTargets(code,requestId){
    begin(requestId)
    try{const d=await mockApi.libraryGet(code);if(job?.cancelled)throw '已取消';return d?[d.dir]:[]}finally{end()}
  },
  async appendPreview(code,inputs,target,_video,requestId){
    const id=begin(requestId);pendingAppend=null
    try{
      const detail=await mockApi.libraryGet(code);if(job?.cancelled)throw '已取消';if(!detail)throw '影片不存在';if(target&&target!==detail.dir)throw '目标不匹配'
      pendingAppend={token:String(id),root:settings.library_root??'',code,targetDir:detail.dir,ops:inputs.map(from=>({[settings.organize_mode === 'move' ? 'MoveFile' : settings.organize_mode === 'copy' ? 'CopyFile' : settings.organize_mode === 'hardlink' ? 'HardlinkFile' : 'SymlinkFile']:{from,to:`${detail.dir}/${from.split('/').pop()}`}})),skipped:[],conflicts:[]}
      appendSettings=JSON.stringify(settings);return clone(pendingAppend)
    }finally{end()}
  },
  async appendApply(token,requestId){
    const id=begin(requestId)
    try{
      const p=pendingAppend;pendingAppend=null;if(!p||p.token!==token||appendSettings!==JSON.stringify(settings))throw '追加预览已过期或设置已变化'
      await sleep(40)
      if(job?.cancelled)return {journalId:'',completed:0,skipped:[],failed:[],cancelled:true}
      const m=movies.find(m=>m.code===p.code)!;const before=m.parts;m.parts+=p.ops.filter(o=>/\.(mp4|mkv)$/i.test(Object.values(o)[0]!.to)).length
      const journalId=String(Date.now());history.unshift({id:journalId,ts:Date.now(),status:'applied',itemCount:1,opCount:p.ops.length,trashed:0,pendingOperations:0,codes:[p.code],added:[],appended:{code:p.code,before,after:m.parts},error:null})
      emitMock('library:changed',{jobId:id});return {journalId,completed:p.ops.length,skipped:[],failed:[],cancelled:false}
    }finally{end()}
  },
  async getSettings() {
    return clone(settings)
  },
  async defaultSettings() {
    return { ...clone(DEFAULTS), library_root: settings.library_root, source_dirs: settings.source_dirs }
  },
  async settingsPath() {
    return '(浏览器预览:localStorage)'
  },
  async saveSettings(s) {
    validateNames(s.name_rules)
    validateAi(s.ai)
    if (s.min_interval_secs < 1) throw '请求间隔不能为 0'
    const badProxy = s.sources.find((c) => c.proxy?.mode === 'custom' && !/^(https?|socks5h?):\/\/\S+$/.test(c.proxy.url.trim()))
    if (badProxy) throw `数据源 ${badProxy.id} 的自定义代理地址无效`
    const solver = s.solver.url.trim()
    if (solver && !/^https?:\/\/\S+$/.test(solver)) throw '求解器地址必须是 http(s) 地址'
    if (solver && !(s.solver.timeout_secs > 0)) throw '求解器超时必须大于 0'
    const {validateTemplate}=await import('../template-preview');if(s.dir_template!==settings.dir_template)validateTemplate(s.dir_template,false);if(s.file_template!==settings.file_template)validateTemplate(s.file_template,true)
    const rootChanged = settings.library_root !== s.library_root
    if (rootChanged && job) throw '有任务正在运行,请等待完成后再更换媒体库'
    const updated = clone(s)
    localStorage.setItem(KEY, JSON.stringify(updated))
    if(JSON.stringify(settings)!==JSON.stringify(s)){editBatch=null;editSettings=null;catalogPending=null;pendingAppend=null;maintenanceToken='';plans=[]}
    if (rootChanged) { invalidateHistory(updated.library_root); editBatch=null;editSettings=null;catalogPending=null; pendingAppend=null;maintenanceToken=''; plans = []; revision = null; openedArtwork=null; croppedArtwork=null }
    settings = updated
  },
  async setLibraryRoot(path, _create) {
    if (!path.trim()) throw '目录不存在'
    const rootChanged = settings.library_root !== path
    if (job) throw '有任务正在运行,请等待完成后再更换媒体库'
    const updated = { ...settings, library_root: path }
    localStorage.setItem(KEY, JSON.stringify(updated))
    if (rootChanged) { invalidateHistory(updated.library_root); editBatch=null;editSettings=null;catalogPending=null; pendingAppend=null;maintenanceToken=''; plans = []; revision = null; openedArtwork=null; croppedArtwork=null }
    settings = updated
    emitMock('library:changed', {jobId:0})
    return clone(settings)
  },
  async mediaDirs() {
    return { videos: '/Users/demo/Movies', downloads: '/Users/demo/Downloads', suggestedLibrary: '/Users/demo/Movies/kiroku' }
  },
  async aiTest(cfg) {
    await sleep(800)
    validateAiProvider(selectedAiProvider(cfg))
    return { title: '在港町度过的三天。', plot: null }
  },
  async mediaServerTest(cfg) {
    await sleep(300)
    const name = cfg.kind === 'emby' ? 'Emby' : 'Jellyfin'
    if (!cfg.api_key.trim()) throw `${name} API Key 无效(401)`
    if (!/^https?:\/\//.test(cfg.url)) throw `连不上 ${name}(${cfg.url})`
    const root = settings.library_root ?? ''
    const local = cfg.local_prefix.trim(), server = cfg.server_prefix.trim().replace(/[\/\\]+$/, '')
    const mapped = local && (root === local || root.startsWith(local + '/')) ? server + root.slice(local.length) : root
    const libraries = [{ name: '电影', locations: ['/media/movies'] }, { name: '剧集', locations: [mapped || '/media/series'] }]
    return { serverName: '家里的服务器', version: cfg.kind === 'emby' ? '4.9.1' : '10.11.0', libraries, matched: mapped ? '剧集' : null }
  },
  async plexTest(url, token) {
    await sleep(500)
    if (!token.trim()) throw 'Plex Token 无效(401)'
    if (!/^https?:\/\//.test(url)) throw `连不上 Plex(${url})`
    const root = settings.library_root ?? ''
    const sections = [
      { key: '1', title: '电影', kind: 'movie', locations: ['/Users/demo/Movies'] },
      { key: '5', title: 'kiroku', kind: 'movie', locations: [root || '/Users/demo/Movies/kiroku'] },
      { key: '2', title: '剧集', kind: 'show', locations: ['/Users/demo/TV'] },
    ]
    return { sections, matched: root ? 'kiroku' : null }
  },
  async pickDirectory(title, start) {
    // 浏览器里没有原生对话框,用 prompt 代替
    const v = window.prompt(title ?? '输入文件夹路径', start ?? settings.source_dirs[0] ?? '/Volumes/Downloads/新片')
    return v && v.trim() ? v.trim() : null
  },

  async inspectDirectory(path) {
    // 浏览器预览:路径里带 Downloads/新片 的当作「有视频没 NFO」,带 Existing 的当作已有媒体库
    if (/existing/i.test(path)) return { videos: 120, nfos: 118, truncated: false }
    if (/downloads|新片|待整理/i.test(path)) return { videos: 14, nfos: 0, truncated: false }
    return { videos: 0, nfos: 0, truncated: false }
  },

  async clearTranslationCache() {},
  async tagPresetMerge(rules) { return mergeTags(rules, tagPreset) },
  async notifyFinish(title, body) {
    if (!settings.notify_on_finish || document.hasFocus()) return false
    console.info(`[通知] ${title}：${body}`)
    return true
  },

  async classifyPaths(paths) {
    // 浏览器预览:带 missing 的当作不存在,带扩展名的当作文件,其余当作文件夹
    return paths.map((path) => ({ path, kind: /missing/i.test(path) ? 'missing' : isVideoPath(path) ? 'video' : /\.[a-z0-9]{2,4}$/i.test(path) ? 'file' : 'dir' }))
  },

  async pendingList(requestId, force): Promise<PendingList> { return await readPending(true, requestId, force) as PendingList },
  async pendingSummary(requestId, force): Promise<PendingSummary> { return await readPending(false, requestId, force) as PendingSummary },
  async pendingCancel(requestId) {
    if (!pendingRequest || pendingRequest.requestId !== requestId || job?.id !== pendingRequest.id) return false
    job.cancelled = true; return true
  },
  async setSourceDirs(dirs) {
    const lib = settings.library_root
    const bad = lib && dirs.find((d) => d.startsWith(lib))
    if (bad) throw `「${bad}」在媒体库目录里面。请选择媒体库之外的文件夹。`
    if(JSON.stringify(settings.source_dirs)!==JSON.stringify(dirs)){editBatch=null;editSettings=null}
    settings = { ...settings, source_dirs: dirs }
    localStorage.setItem(KEY, JSON.stringify(settings))
    return clone(settings)
  },

  async scanStart(dirs, rememberSources = true) {
    const lib = settings.library_root
    const bad = lib && dirs.find((d) => d.startsWith(lib))
    if (bad) throw `「${bad}」在媒体库目录里面。请选择媒体库之外的文件夹(库内的影片已经整理过了)。`
    const id = begin()
    // 单个视频文件只用于本次导入,不记为待整理文件夹
    if (rememberSources) settings = { ...settings, source_dirs: dirs.filter((d) => !isVideoPath(d)) }
    localStorage.setItem(KEY, JSON.stringify(settings))
    const folder = dirs.find((d) => !isVideoPath(d))
    const split = folder ? splitScan(folder.replace(/\/$/, '')) : { kept: [] as ScanItem[], skipped: [] as SkippedFile[] }
    // 文件根:只扫这一个文件(按文件名匹配样例文件)
    for (const file of dirs.filter(isVideoPath)) {
      const name = file.split('/').pop()!
      const sample = splitScan(file.slice(0, -name.length - 1)).kept.find((k) => k.path.endsWith('/' + name))
      if (sample) split.kept.push({ ...sample, id: split.kept.length, path: file })
    }
    const { kept, skipped } = split
    ;(async () => {
      scan = []
      for (let i = 0; i < kept.length; i += 4) {
        await sleep(260)
        if (job?.cancelled) break
        const batch = kept.slice(i, i + 4)
        scan.push(...batch)
        emitMock('scan:items', { jobId: id, items: batch })
      }
      found.clear()
      plans = []
      const codes = new Set(scan.filter((s) => s.code).map((s) => s.code!.id))
      const cancelled = !!job?.cancelled
      end()
      emitMock('scan:done', {
        jobId: id,
        total: scan.length,
        codes: codes.size,
        unrecognized: scan.filter((s) => !s.code).length,
        skipped,
        skippedTotal: skipped.length,
        errors: [], errorsTotal: 0, complete: !cancelled,
        cancelled,
      })
    })()
    return id
  },

  async setItemCode(itemId, text) {
    if (job) throw '有任务正在运行,请稍后再试'
    const it = scan[itemId]
    if (!it) throw '找不到该文件'
    const t = text.trim()
    const fc2 = /^fc2[-_ ]?(?:ppv)?[-_ ]?(\d{5,})$/i.exec(t)
    const std = /^([A-Za-z]{2,6})[-_ ]?(\d{2,5})$/.exec(t)
    if (!fc2 && !std) throw `无法识别「${t}」这种番号格式,试试 ABC-123 或 FC2-PPV-1234567`
    const id = fc2 ? `FC2-${fc2[1]}` : `${std![1].toUpperCase()}-${std![2].padStart(3, '0')}`
    const ct = fc2 ? 'fc2' : 'censored'
    scan[itemId] = {
      ...it,
      info: { ...it.info, number: id, content_type: ct, matched: true, prefix: id.split('-')[0] },
      code: { kind: fc2 ? 'Fc2' : 'Standard', id, content_type: ct, part: it.info.cd, subtitled: it.info.has_subtitle, mosaic: it.info.mosaic === 'cracked' || it.info.mosaic === 'leaked' ? it.info.mosaic : fc2 ? null : 'censored', definition: it.info.definition },
    }
    found.clear()
    plans = []
    return scan[itemId]
  },

  async scrapeStart(itemIds) {
    const id = begin()
    const codes = [...new Map(itemIds.map((i) => scan[i]).filter((s) => s?.code && !found.has(s.code!.id)).map((s) => [s.code!.id, s.code!])).values()]
    ;(async () => {
      let nFound = 0, nMissing = 0, nFailed = 0
      const t0 = Date.now()
      for (let i = 0; i < codes.length; i++) {
        const c = codes[i]
        const route = sourceRoute(c.id,c.content_type)
        const ev = (status: ScrapeStatus, done: number) =>
          emitMock('scrape:event', { jobId: id, code: c.id, status, done, total: codes.length, preview: null, note: null })
        // 与后端一致:同一部影片的路线站点并发查询
        ev({ kind: 'scraping', site: route.join(',') || '无可用来源' }, i)
        await sleep(520)
        if (job?.cancelled) { ev({ kind: 'cancelled' }, i); break }
        await sleep(380)
        if (job?.cancelled) { ev({ kind: 'cancelled' }, i); break }
        // 与后端一致:JavDB 的 FC2 详情页要求登录,未填 Cookie 时失败并提示
        const needLogin = c.id.startsWith('FC2-') && route.length===1 && route[0]==='javdb' && !settings.sources.find((x) => x.id === 'javdb')?.cookie
        const outcome: ScrapeStatus = !route.length
          ? { kind: 'failed', message: '该内容类型没有可用的数据源' }
          : needLogin
          ? { kind: 'failed', message: 'javdb: 该影片需要登录 JavDB 才能查看,请在 设置 → 数据源 填写 JavDB Cookie' }
          : pinnedUrl.has(c.id) ? { kind: 'found' as const }
          : OUTCOME[c.id] ?? { kind: 'found' as const }
        let preview = null
        if (outcome.kind === 'found') {
          nFound++
          const m = newMovie(c.id, c.content_type, 1)
          const rawMeta=movieMetadata(m)
          const texts:import('../types').TextVersions={original:{title:m.originalTitle??m.title,plot:m.plot,provenance:{title:route[0],plot:route[0]}},localized:{}}
          sourceText.set(c.id,texts)
          if (settings.ai.enabled && selectedAiProvider(settings.ai)?.api_key) {
            ev({ kind: 'translating' }, i)
            await sleep(450)
            m.originalTitle = texts.original.title??m.title
            texts.localized['zh-Hans']={}
            if(settings.ai.title){m.title='【译】'+m.title;texts.localized['zh-Hans'].title=m.title}
            if(settings.ai.plot){m.plot='【译】'+m.plot;texts.localized['zh-Hans'].plot=m.plot}
          }
          const metadata=movieMetadata(m)
          metadata.text_versions=clone(texts);metadata.nfo_language=settings.nfo_language
          metadata.raw_sources={}
          for(const cfg of settings.sources.filter(s=>route.includes(s.id)))metadata.raw_sources[cfg.id]=await sourceSnapshot(c.id,c.content_type,cfg,settings.proxy,rawMeta)
          const projected=projectNames(metadata,settings.name_rules).metadata
          foundMetadata.set(c.id,projected);m.actors=projected.actors.map(a=>a.name);m.tags=projected.tags;m.studio=projected.studio??'';m.publisher=projected.publisher??'';m.series=projected.series
          found.set(c.id, m)
          preview = {
            title: m.title, originalTitle: m.originalTitle, cover: posterSvg(m.code, m.hue), coverIsLandscape: false,
            releaseDate: m.releaseDate, runtimeMin: m.runtime, studio: m.studio, actors: m.actors, score: m.score, sources: route,
          }
        } else if (outcome.kind === 'notFound') nMissing++
        else nFailed++
        const url = pinnedUrl.get(c.id); pinnedUrl.delete(c.id)
        const note = url !== undefined && !url.toLowerCase().replace(/[-_]/g, '').includes(c.id.toLowerCase().replace(/[-_]/g, '')) ? `详情页里没有找到番号 ${c.id},请确认是同一部影片` : null
        emitMock('scrape:event', { jobId: id, code: c.id, status: outcome, done: i + 1, total: codes.length, preview, note })
        const per = (Date.now() - t0) / (i + 1) / 1000
        if (i + 1 < codes.length) emitMock('job:eta', { jobId: id, remainingSecs: Math.round(per * (codes.length - i - 1)) })
      }
      plans = []
      const cancelled = !!job?.cancelled
      end()
      emitMock('scrape:done', { jobId: id, found: nFound, notFound: nMissing, failed: nFailed, cancelled })
    })()
    return id
  },

  async scrapeUrlStart(itemIds, url) {
    const codes = [...new Map(itemIds.map((i) => scan[i]).filter((s) => s?.code).map((s) => [s.code!.id, s.code!])).values()]
    if (codes.length !== 1) throw '请为一部影片指定详情页'
    const c = codes[0]
    try { urlSource(url.trim(), sourceRoute(c.id, c.content_type)) } catch (message) {
      const id = begin()
      queueMicrotask(() => {
        emitMock('scrape:event', { jobId: id, code: c.id, status: { kind: 'failed', message: String(message) }, done: 1, total: 1, preview: null, note: null })
        end(); emitMock('scrape:done', { jobId: id, found: 0, notFound: 0, failed: 1, cancelled: false })
      })
      return id
    }
    found.delete(c.id); pinnedUrl.set(c.id, url.trim())
    return mockApi.scrapeStart(itemIds)
  },

  async planBuild(itemIds, requestId) {
    const id = begin()
    const config = clone(settings)
    emitMock('plan:started', { jobId: id, requestId: requestId ?? null })
    try {
    const root = config.library_root
    if (!root) throw '请先设置媒体库目录'
    const {previewTemplate}=await import('../template-preview')
    await sleep(0)
    if (job?.cancelled) throw '计划已取消'
    if (JSON.stringify(settings) !== JSON.stringify(config)) throw '设置或媒体库已变化，请重新生成整理计划'
    const taken = new Set<string>()
    const metaDone = new Set<string>()
    const lib = new Set(movies.map((m) => m.code))
    const items = itemIds.map((i) => scan[i]).filter((s) => s?.code && found.has(s.code.id))
    const builtPlans = items.map((it) => {
      const m = found.get(it.code!.id)!
      const meta=foundMetadata.get(m.code);if(meta){const projected=projectNames(meta,config.name_rules).metadata;foundMetadata.set(m.code,projected);m.actors=projected.actors.map(a=>a.name);m.tags=projected.tags;m.studio=projected.studio??'';m.publisher=projected.publisher??'';m.series=projected.series}
      const part = it.code!.part
      const c=it.code!,ext=it.path.split('.').pop()??'mp4',names=[...new Set(m.actors)]
      const context:Record<string,string>={code:m.code,number:m.code,title:m.title,original_title:m.originalTitle??'',actor:m.actors[0]??'',actress:m.actors[0]??'',actors:names.join('、'),actresses:names.join('、'),year:m.releaseDate.slice(0,4),release:m.releaseDate,runtime:String(m.runtime),score:String(m.score),studio:m.studio,publisher:m.publisher,series:m.series??'',directors:'',prefix:m.code.split('-').slice(0,-1).join('-'),suffix:m.code.split('-').pop()??'',ext,content_type:c.content_type,raw_name:it.path.split('/').pop()!.replace(/\.[^.]+$/,''),raw_dir:it.path.split('/').slice(-2,-1)[0]??'',cd:part?String(part):'',sub:c.subtitled?'C':'',mosaic:c.mosaic??'',def:c.definition??''}
      const fileTpl=config.plex.naming?'{code}[-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}][ {title}][ - cd{cd?}]':config.file_template
      const preview=previewTemplate(config.dir_template,fileTpl,context)
      const to = `${root}/${preview.directory}/${preview.file}`
      const carrier = !metaDone.has(m.code)
      metaDone.add(m.code)
      const stem = preview.file.slice(0, -(ext.length + 1))
      const extras = carrier ? [`${stem}.nfo`, 'kiroku.json', `${stem}-poster.jpg`, `${stem}-fanart.jpg`] : []
      if (carrier && config.download_extrafanart) extras.push('extrafanart/fanart1.jpg', 'extrafanart/fanart2.jpg', 'extrafanart/fanart3.jpg')
      const outputs = [to, ...extras.map((name) => to.slice(0, to.lastIndexOf('/') + 1) + name)]
      const conflicts: PlanItem['conflicts'] = preview.error?[{kind:'invalidTemplate',path:it.path}]:[]
      for (const path of outputs) {
        const kind = taken.has(path) ? 'duplicateInBatch' : lib.has(m.code) ? 'targetExists' : null
        if (kind) conflicts.push({ kind, path })
        taken.add(path)
      }
      // ADR-0008:库里已有该番号、文件带中字/清晰度/破解标记 → 作为新版本并存,默认不选
      const label=[c.definition,c.subtitled?'中字':null,c.mosaic==='cracked'?'破解':c.mosaic==='leaked'?'流出':null].filter(Boolean).join(' ')
      if(lib.has(m.code)&&label&&!preview.error){const dir=originalDirs.get(m.code)??`${root}/${m.actors[0]}/${m.code}`;const target=`${dir}/${m.code} - ${label}.${ext}`;return {itemId:it.id,code:m.code,title:m.title,from:it.path,to:target,extras:[],conflict:null,conflicts:[],selected:false,newVersion:true,notices:[`库中已有该分片的其他版本，将作为新版本并存：${m.code} - ${label}.${ext}；确认后请手动勾选`,'追加到已有影片，保留元数据和海报'],_movie:m} as PlanItem & { _movie: MockMovie }}
      const conflict = conflicts[0]?.kind ?? null
      return {
        itemId: it.id,
        code: m.code,
        title: m.title,
        from: it.path,
        to,
        extras,
        conflict,
        conflicts,
        selected: conflict === null,
        notices:[...(preview.error?[preview.error]:preview.warnings),...Object.values(foundMetadata.get(m.code)?.name_projection??{}).flatMap(r=>r.warnings??[]),...(config.organize_mode==='move'?[]:[`${config.organize_mode}：保留源文件，不清理原文件夹`])],
        _movie: m,
      } as PlanItem & { _movie: MockMovie }
    })
    for (const p of builtPlans) {
      const donor = builtPlans.find((other) => other.code === p.code && other.extras.length)
      for (const c of donor?.conflicts ?? []) {
        if (!c.path.match(/\.(mp4|mkv|mov)$/i) && !p.conflicts.some((x) => x.path === c.path)) p.conflicts.push({ ...c })
      }
      p.conflict = p.conflicts[0]?.kind ?? null
      p.selected = !p.conflict
    }
    if (job?.cancelled) throw '计划已取消'
    if (JSON.stringify(settings) !== JSON.stringify(config)) throw '设置或媒体库已变化，请重新生成整理计划'
    plans = builtPlans
    return plans.map(({ _movie, ...p }) => p)
    } finally { end() }
  },

  async applyStart(itemIds) {
    const chosen = plans.filter((p) => itemIds.includes(p.itemId) && !p.conflict).map((p) => ({ ...p }))
    if (!chosen.length) throw '没有可执行的项目,请重新生成整理计划'
    const carriers = new Set(chosen.filter((p) => p.extras.length).map((p) => p.code))
    for (const p of chosen) {
      if (!carriers.has(p.code)) {
        p.extras = [...(plans.find((other) => other.code === p.code && other.extras.length)?.extras ?? [])]
        carriers.add(p.code)
      }
    }
    const id = begin()
    /** 与 kiroku-core 的整理日志同格式;按设置决定是否在 App 内显示 */
    const log = (level: LogLevel, message: string, target = 'organize') => {
      if (!settings.log.app || (level === 'debug' && !settings.log.verbose)) return
      const time = new Date().toTimeString().slice(0, 8) + '.' + String(Date.now() % 1000).padStart(3, '0')
      emitMock('log:line', { jobId: id, time, level, target, message })
    }
    ;(async () => {
      const added: string[] = []
      let done = 0
      let cancelled = false
      log('info', `开始整理 ${chosen.length} 项 → ${settings.library_root}${settings.cleanup_source ? ',完成后清理原文件夹' : ''}`)
      const imageTasks = [...new Map(chosen.flatMap(p => p.extras.filter(x => /\.(jpg|png)$/.test(x)).map(file => [`${p.code}/${file}`, { code: p.code, file }] as const))).values()]
      let downloaded = 0
      emitMock('apply:progress', { jobId: id, phase: 'images', done: 0, total: imageTasks.length, failed: 0, code: null })
      log('info', `下载图片:共 ${imageTasks.length} 张`)
      for (const task of imageTasks) {
        if (job?.cancelled) { cancelled = true; break }
        emitMock('apply:progress', { jobId: id, phase: 'images', done: downloaded, total: imageTasks.length, failed: 0, code: task.code })
        await sleep(100)
        if (job?.cancelled) { cancelled = true; break }
        downloaded++
        emitMock('apply:progress', { jobId: id, phase: 'images', done: downloaded, total: imageTasks.length, failed: 0, code: task.code })
      }
      if (!cancelled) emitMock('apply:progress', { jobId: id, phase: 'organize', done: 0, total: chosen.length, failed: 0, code: null })
      for (const p of chosen) {
        if (cancelled) break
        await sleep(380)
        if (job?.cancelled) { cancelled = true; log('info', '  已取消'); break }
        log('info', `[${done + 1}/${chosen.length}] ${p.code}`)
        log('info', `  移动 ${p.from} → ${p.to}`)
        const dir = p.to.slice(0, p.to.lastIndexOf('/'))
        for (const x of p.extras) log('info', x.endsWith('.jpg') ? `  保存 ${dir}/${x}${x.endsWith('-poster.jpg') ? '(裁剪海报)' : ''}` : `  写入 ${dir}/${x}`)
        log('debug', `GET https://c0.jdbstatic.com/covers/mock/${p.code}.jpg → 200 (183 KB, 412 ms)`, 'net')
        done++
        const m = p._movie!
        const existing = movies.find((x) => x.code === m.code)
        if (existing) existing.parts += 1
        else {
          movies.unshift({ ...m, addedAt: Math.floor(Date.now() / 1000) }); added.push(m.code)
          const meta=foundMetadata.get(m.code);if(meta)edited.set(m.code,clone(meta))
          originalDirs.set(m.code,p.to.slice(0,p.to.lastIndexOf('/')))
          // Snapshots were captured before AI/name projections during scraping.
          // Never regenerate them from normalized import metadata.
          const imported=clone(meta ?? (await mockApi.libraryGet(m.code))!.meta)
          imported.asset_stem = p.extras.find(x => x.endsWith('.nfo'))?.slice(0, -4) ?? p.to.split('/').pop()!.replace(/\.[^.]+$/, '')
          imported.text_versions=clone(sourceText.get(m.code)??imported.text_versions??{original:{},localized:{}});imported.nfo_language=settings.nfo_language
          edited.set(m.code,imported);originalDirs.set(m.code,dir)
        }
        const ev: ApplyEvent = { jobId: id, code: p.code, itemId: p.itemId, done, total: chosen.length, ok: true, error: null, warnings: [] }
        emitMock('apply:event', ev)
        emitMock('library:changed', {jobId:job?.id??0})
      }
      const journalId = String(Date.now())
      if (done > 0) {
        history.unshift({
          id: journalId, ts: Date.now(), status: cancelled ? 'partial' : 'applied',
          itemCount: done, opCount: done * 4, trashed: 0, pendingOperations: 0, error: null, codes: [...new Set(chosen.slice(0, done).map((c) => c.code))].slice(0, 6), added,
        })
      }
      // 模拟:整理掉的文件所在子文件夹视为已清理
      const cleaned = settings.cleanup_source && settings.organize_mode === 'move' && done > 0 ? [`${settings.source_dirs[0] ?? '/Volumes/Downloads/新片'}/QRS-912`] : []
      if (history[0]?.id === journalId) history[0].trashed = cleaned.length
      for (const d of cleaned) log('info', `原文件夹已移到废纸篓: ${d}`)
      log('info', `整理结束:成功 ${done} 项,失败 0 项${cancelled ? ',已取消' : ''}`)
      emitMock('apply:progress', { jobId: id, phase: 'index', done: 0, total: done, failed: 0, code: null })
      log('info', `媒体库索引已更新(${done} 个目录)`)
      end()
      emitMock('apply:done', { indexError: null, jobId: id, applied: done, failed: [], cancelled, journalId, cleaned })
    })()
    return id
  },

  async jobCancel(jobId) {
    if (!job || (jobId!==undefined&&jobId!==job.id)) return false
    job.cancelled = true
    return true
  },

  async historyList(requestId=crypto.randomUUID(), page={}) {
    if(!requestId||requestId.length>100)throw '无效的读取请求';if(historyReads.has(requestId)||historyReads.size>=16)throw '历史读取繁忙，请稍后重试';
    const read={cancelled:stoppedHistory.delete(requestId)};historyReads.set(requestId,read);const root=settings.library_root,generation=historyGeneration;
    try {
      await sleep(20)
      if(read.cancelled)throw '历史读取已取消'
      if(root!==settings.library_root||generation!==historyGeneration)throw '媒体库已变化，请重新读取'
      const f=page.filter??{},words=(f.text??'').trim().toLowerCase().split(/\s+/).filter(Boolean)
      if((f.text?.length??0)>512 || (f.from!=null && f.until!=null && f.from>=f.until))throw '历史筛选范围无效'
      const rows=history.filter(h=>(!f.status||h.status===f.status)&&(f.from==null||h.ts>=f.from)&&(f.until==null||h.ts<f.until)&&words.every(w=>h.id.includes(w)||h.codes.join(' ').toLowerCase().includes(w))).sort((a,b)=>b.ts-a.ts||b.id.localeCompare(a.id))
      const cursor=page.before?history.find(h=>h.id===page.before):null
      const limit=Math.min(Math.max(page.limit??50,1),1000),after=page.before?rows.filter(h=>cursor?(h.ts<cursor.ts||h.ts===cursor.ts&&h.id<cursor.id):false):rows,items=after.slice(0,limit).map(h=>({id:h.id,ts:h.ts,status:h.status,itemCount:h.itemCount,opCount:h.opCount,codes:h.codes.slice(0,6),trashed:h.trashed,pendingOperations:h.pendingOperations,error:h.error}))
      return {items,total:rows.length,next:after.length>items.length&&items.length?items[items.length-1].id:null}
    } finally {historyReads.delete(requestId)}
  },
  async historyDetail(requestId,journalId,offset=0) {
    if(!requestId||requestId.length>100||historyReads.has(requestId)||historyReads.size>=16)throw '无效的读取请求'
    const read={cancelled:stoppedHistory.delete(requestId)};historyReads.set(requestId,read);const root=settings.library_root,generation=historyGeneration
    try {
      await sleep(30)
      if(read.cancelled)throw '历史读取已取消'
      if(root!==settings.library_root||generation!==historyGeneration)throw '媒体库已变化，请重新读取'
      const h=history.find(h=>h.id===journalId);if(!h)throw '日志不存在';if(h.error)throw h.error
      const total=h.opCount+(h.pendingOperations??0)+h.trashed
      const operations=Array.from({length:Math.max(0,Math.min(100,total-offset))},(_,i)=>({index:offset+i,description:`示例操作 ${offset+i+1} · ${h.codes[(offset+i)%Math.max(1,h.codes.length)]??journalId}`,state:h.status==='rolledBack'?'undone' as const:offset+i<h.opCount?'recorded' as const:offset+i<h.opCount+(h.pendingOperations??0)?'pending' as const:'trashed' as const}))
      return {id:journalId,status:h.status,operations,total,next:offset+operations.length<total?offset+operations.length:null}
    }finally{historyReads.delete(requestId)}
  },
  async historyCancel(requestId){return stopHistory(requestId)},
  async revealJournal(id) { console.info('[mock] reveal journal', id) },
  async rollback(journalId) {
    const h = history.find((x) => x.id === journalId)
    if (!h) throw '日志不存在'
    if (job) throw '已有任务在运行'
    if (h.status === 'rolledBack') return { indexError: null, failures: [], status: 'rolledBack' }
    if(h.appended){const m=movies.find(m=>m.code===h.appended!.code);if(!m||m.parts!==h.appended.after)return {indexError:null,failures:['影片已变化，拒绝撤销'],status:'partial'};m.parts=h.appended.before}
    if (h.revision) {
      const r = h.revision
      if (settings.library_root !== r.root || JSON.stringify(edited.get(r.code)) !== JSON.stringify(r.after)) return { indexError: null, failures: ['元数据发生外部变动，已拒绝撤销'], status: 'partial' }
      if(r.imageAfter && artworkImages.get(r.code)!==r.imageAfter)return {indexError:null,failures:['海报发生外部变动，已拒绝撤销'],status:'partial'}
      if(r.imageBefore)artworkImages.set(r.code,r.imageBefore);else artworkImages.delete(r.code)
      if(!r.before.artwork)artworkOriginals.delete(r.code)
      edited.set(r.code, clone(r.before))
      const movie = movies.find(m => m.code === r.code); if (movie) Object.assign(movie, clone(r.movie))
    }
    for (const code of h.added) {
      const i = movies.findIndex((m) => m.code === code)
      if (i >= 0) movies.splice(i, 1)
      edited.delete(code);originalDirs.delete(code)
    }
    if (h.repaired) {
      const m = movies.find((x) => x.code === h.repaired!.code)
      if (m) { if (h.repaired.poster) m.missingPoster = true; if (h.repaired.fanart) m.missingFanart = true }
    }
    h.status = 'rolledBack'
    emitMock('library:changed', {jobId:0})
    return { indexError: null, failures: [], status: 'rolledBack' }
  },

  async libraryOverview(query) {
    const list = clone(matchingMovies(query))
    const scores = list.flatMap(m => m.score == null ? [] : [m.score])
    const result = { movies:list.length, files:list.reduce((n,m) => n + m.parts,0), bytes:list.reduce((n,m) => n + Array.from({length:m.parts},(_,i) => Math.round((1.6 + i * 0.4) * GB)).reduce((a,b) => a+b,0),0), favorites:list.filter(m => facetValues(m).favorite.length > 0).length, averageScore:scores.length ? scores.reduce((a,b) => a+b,0)/scores.length : null, missingPoster:list.filter(m => m.missingPoster).length, missingScore:list.filter(m => m.score == null).length, missingYear:list.filter(m => m.year == null).length }
    await sleep(40)
    return result
  },
  async libraryList(q: Query): Promise<Page> {
    let list = matchingMovies(q)
    const sort = q.sort ?? 'added'
    const bytes = new Map<string, Uint8Array>()
    const binary = (a: string, b: string) => {
      if (!bytes.has(a)) bytes.set(a, new TextEncoder().encode(a))
      if (!bytes.has(b)) bytes.set(b, new TextEncoder().encode(b))
      const x = bytes.get(a)!, y = bytes.get(b)!
      for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) return x[i] - y[i]
      return x.length - y.length
    }
    const nocase = (text: string) => text.replace(/[A-Z]/g, c => c.toLowerCase())
    list = [...list].sort((a, b) => {
      const av = sort === 'year' ? a.year : sort === 'score' ? a.score : 0
      const bv = sort === 'year' ? b.year : sort === 'score' ? b.score : 0
      if (av == null || bv == null) {
        if (av != bv) return av == null ? 1 : -1
      }
      const compare = sort === 'title' ? binary(nocase(a.title), nocase(b.title)) : sort === 'year' || sort === 'score' ? (bv ?? 0) - (av ?? 0) : b.addedAt - a.addedAt
      return (compare || binary(a.code, b.code)) * (q.reverse ? -1 : 1)
    })
    const off = q.offset ?? 0
    const lim = q.limit || 60
    const selected = list.slice(off, off + lim).map(m => clone(m))
    await sleep(40) // 模拟 IPC 延迟,让占位/骨架可见
    return { total: list.length, items: await Promise.all(selected.map(toCard)) }
  },

  async libraryExportCsv(query, requestId) {
    const jobId = begin(requestId)
    try {
      // Capture before yielding so later mock mutations cannot mix snapshots.
      const pending = mockApi.libraryList({ ...query, offset: 0, limit: Math.max(1, movies.length) })
      const page = await pending
      const chunks = ['\ufeff编号,标题,年份,演员,评分,分片数,收藏\r\n']
      for (let i = 0; i < page.items.length; i++) {
        if (i % 500 === 0) {
          emitMock('library:export', { jobId, root: settings.library_root ?? '', done: i, total: page.total })
          await sleep(0)
        }
        if (job?.cancelled) throw '导出已停止'
        const m = page.items[i]
        chunks.push([m.code, m.title ?? '', String(m.year ?? ''), m.actors.join(' / '), m.score === null ? '' : (m.score / 10).toFixed(1), String(m.parts), m.favorite ? '是' : '否'].map(csvCell).join(',') + '\r\n')
      }
      if (job?.cancelled) throw '导出已停止'
      emitMock('library:export', { jobId, root: settings.library_root ?? '', done: page.total, total: page.total })
      const url = URL.createObjectURL(new Blob(chunks, { type: 'text/csv;charset=utf-8' }))
      const a = document.createElement('a'); a.href = url; a.download = 'kiroku-library.csv'; a.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
      return page.total
    } finally { end() }
  },

  async libraryGet(code): Promise<MovieDetail | null> {
    const m = movies.find((x) => x.code === code)
    if (!m) return null
    const dir = originalDirs.get(code) ?? `${settings.library_root ?? '/Library'}/${m.actors[0]}/${m.code}`
    return {
      ...await toCard(m),
      meta: edited.has(code) ? clone(edited.get(code)!) : {
        code: m.code, title: m.title, original_title: m.originalTitle, plot: m.plot, release_date: m.releaseDate,
        runtime_min: m.runtime, studio: m.studio, publisher: m.publisher, series: m.series,
        actors: m.actors.map((name) => ({ name, gender: 'female' as const })),
        tags: m.tags, directors: [], scores: [{ site: 'javdb', score: m.score }], source_urls: { javdb: 'https://javdb.com/' }, provenance: { title: 'javdb', actors: 'javbus' }, locked_fields: [],
      },
      dir,
      files: Array.from({ length: m.parts }, (_, i) => ({
        path: `${dir}/${m.code}${m.parts > 1 ? `-CD${i + 1}` : ''}.mp4`, part: m.parts > 1 ? i + 1 : null, size: Math.round((1.6 + i * 0.4) * GB),
      })),
      fanart: m.missingFanart ? null : posterSvg(m.code, (m.hue + 20) % 360),
      fanartVersion: await imageVersion(m.missingFanart ? null : posterSvg(m.code, (m.hue + 20) % 360)),
    }
  },

  async artworkOpen(code,requestId) {
    const id=begin(requestId)
    try {
      const detail=await mockApi.libraryGet(code);if(!detail)throw '影片不在媒体库中'
      let bytes=artworkOriginals.get(code)
      if(!bytes){const canvas=document.createElement('canvas');canvas.width=800;canvas.height=600;const ctx=canvas.getContext('2d')!;const gradient=ctx.createLinearGradient(0,0,800,600);gradient.addColorStop(0,'#725ccb');gradient.addColorStop(1,'#efbe88');ctx.fillStyle=gradient;ctx.fillRect(0,0,800,600);ctx.fillStyle='white';ctx.font='48px sans-serif';ctx.fillText(code,80,250);bytes=canvasBytes(canvas)}
      const token=String(id),root=settings.library_root??'/Library'
      if(job?.cancelled)throw '已取消'
      openedArtwork={token,root,code,before:clone(detail.meta),settings:clone(settings),image:bytes}
      return {token,root,code,width:800,height:600,image:bytes,crop:detail.meta.artwork?.crop??null,source:detail.meta.artwork?.source_url??'浏览器预览示例原图'}
    } finally {end()}
  },
  async artworkPreview(token,root,code,crop,requestId) {
    const id=begin(requestId)
    try {
      const o=openedArtwork
      if(!o||o.token!==token||o.root!==root||o.code!==code||root!==settings.library_root||JSON.stringify(o.settings)!==JSON.stringify(settings))throw '原图媒体库或设置已变化'
      if(Object.values(crop).some(v=>!Number.isInteger(v)||v<0)||!crop.width||!crop.height||crop.x+crop.width>800||crop.y+crop.height>600)throw '裁剪框超出原图范围'
      const current=await mockApi.libraryGet(code);if(!current||JSON.stringify(current.meta)!==JSON.stringify(o.before))throw '磁盘内容已变化'
      const img=new Image();img.src=bytesUrl(o.image);await img.decode();const canvas=document.createElement('canvas');canvas.width=crop.width;canvas.height=crop.height;canvas.getContext('2d')!.drawImage(img,crop.x,crop.y,crop.width,crop.height,0,0,crop.width,crop.height)
      const after=clone(o.before);after.artwork={original:`.kiroku-original-${code}.png`,digest:`mock-${code}`,source_url:'浏览器预览示例原图',crop:clone(crop),phase:{subtitle:false,uncensored:false,cracked:false,leaked:false,definition:null}};after.locked_fields=[...new Set([...(after.locked_fields??[]),'poster_urls'])];after.provenance={...after.provenance,poster_urls:'manual'}
      if(job?.cancelled)throw '已取消'
      revision={token:String(id),root,code,dir:current.dir,before:clone(o.before),after,changes:[]};revisionSettings=clone(settings)
      croppedArtwork={token:String(id),image:canvas.toDataURL('image/jpeg',.92),original:o.image}
      return {revision:clone(revision),image:Array.from(atob(croppedArtwork.image.split(',')[1]),c=>c.charCodeAt(0))}
    } finally {end()}
  },
  async revisionPreview(code, patch) {
    const id = begin()
    try {
      const detail = await mockApi.libraryGet(code)
      if (!detail) throw '影片不在媒体库中'
      const after = clone(detail.meta), changes: RevisionPreview['changes'] = []
      for (const [field, supplied] of Object.entries(patch)) {
        const value = field === 'actors' && Array.isArray(supplied) ? supplied.map(v => v && typeof v === 'object' ? { ...v, gender: v.gender ?? 'unknown' } : v) : supplied
        if (!EDITABLE_FIELDS.some(f => f.key === field)) throw '不可人工编辑字段'
        if (['actors', 'tags', 'directors'].includes(field)) {
          if (!Array.isArray(value) || value.some(v => field === 'actors' ? !v || typeof v.name !== 'string' || !['female','male','unknown'].includes(v.gender ?? 'unknown') : typeof v !== 'string')) throw '列表字段格式无效'
        } else if (field === 'runtime_min') { if (value !== null && (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 4294967295)) throw '片长格式无效' }
        else if (value !== null && typeof value !== 'string') throw '文本字段格式无效'
        const old = after[field as keyof Metadata]
        if (JSON.stringify(old) === JSON.stringify(value)) continue
        changes.push({ field, before: clone(old), after: clone(value) })
        Object.assign(after, { [field]: clone(value) })
        if(after.name_projection)delete after.name_projection[field]
        after.locked_fields = [...new Set([...(after.locked_fields ?? []), field])]
        after.provenance = { ...after.provenance, [field]: 'manual' }
      }
      revisionSettings = null
      revision = { token: String(id), root: settings.library_root ?? '/Library', dir: detail.dir, code, before: clone(detail.meta), after, changes }
      return clone(revision)
    } finally { end() }
  },
  async nfoExportPreview(codes,language,requestId){
    const id=begin(requestId)
    try{
      if(!codes.length)throw '请选择影片'
      const captured=clone(settings),items:RevisionPreview[]=[],root=settings.library_root??'/Library',token=String(id)
      for(const code of [...new Set(codes)]){
        if(job?.cancelled)throw '已取消'
        const detail=await mockApi.libraryGet(code);if(!detail)throw '影片不在媒体库中'
        const after=clone(detail.meta);after.nfo_language=language;recoverText(after)
        const changes:RevisionPreview['changes']=[]
        for(const field of EDITABLE_FIELDS.map(f=>f.key)){
          const before=projectText(detail.meta).meta[field as keyof Metadata],value=projectText(after).meta[field as keyof Metadata]
          if(JSON.stringify(before)!==JSON.stringify(value))changes.push({field,before:clone(before),after:clone(value)})
        }
        items.push({token,root,dir:detail.dir,code,before:clone(detail.meta),after,changes,warnings:projectText(after).warnings})
      }
      if(job?.cancelled)throw '已取消'
      if(JSON.stringify(settings)!==JSON.stringify(captured))throw '设置已变化，请重新预览'
      languageSettings=captured;languageBatch={token,root,items};return clone(languageBatch)
    }finally{end()}
  },
  async nfoExportApply(token,root,requestId){
    const jobId=begin(requestId)
    const p=languageBatch,captured=languageSettings;languageBatch=null;languageSettings=null
    try{
     if(!p||p.token!==token||p.root!==root||JSON.stringify(captured)!==JSON.stringify(settings))throw '预览已过期，请重新预览'
     const results:import('../types').RevisionResult[]=[],failed:string[]=[]
     for(const preview of p.items){await sleep(40);if(job?.cancelled)break
      try{const detail=await mockApi.libraryGet(preview.code);if(!detail||detail.dir!==preview.dir||JSON.stringify(detail.meta)!==JSON.stringify(preview.before))throw '预览后磁盘内容已变化'
       if(JSON.stringify(captured)!==JSON.stringify(settings))throw '设置已变化，请重新预览'
       const m=movies.find(m=>m.code===preview.code)!,journalId=`language-${Date.now()}-${jobId}-${results.length}`
       history.unshift({id:journalId,ts:Date.now(),status:'applied',itemCount:1,opCount:2,codes:[preview.code],trashed:0,pendingOperations:0,error:null,added:[],revision:{root,code:preview.code,before:clone(preview.before),after:clone(preview.after),movie:clone(m)}})
       originalDirs.set(preview.code,detail.dir);edited.set(preview.code,clone(preview.after));results.push({jobId,root,code:preview.code,journalId,error:null,indexError:null})
       emitMock('library:changed',{jobId});emitMock('nfo:export',{jobId,completed:results.length,failed:failed.length})
      }catch(e){failed.push(`${preview.code}：${e}`)}
     }
     return {jobId,results,failed,cancelled:job?.cancelled??false}
    }finally{end()}
  },
  async batchEditPreview(codes,edit,requestId) {
    const id=begin(requestId)
    editBatch=null;editSettings=null
    try {
      if(!codes.length||codes.length>200)throw '请选择 1–200 部影片'
      const captured=clone(settings),items:RevisionPreview[]=[],root=settings.library_root??'/Library',token=String(id)
      let add:string[]=[],remove:string[]=[]
      if(edit.kind==='tags'){
        const clean=(values:string[])=>{if(values.length>50||values.some(v=>!v.trim()||new TextEncoder().encode(v.trim()).length>200||/[\x00-\x1f\x7f]/.test(v.trim())))throw '标签格式无效';return [...new Set(values.map(v=>v.trim()))]}
        add=clean(edit.add);remove=clean(edit.remove)
        if(!add.length&&!remove.length||add.some(v=>remove.includes(v)))throw '请填写有效且不重叠的增删标签'
      }
      for(const code of [...new Set(codes)]) {
        if(job?.cancelled)throw '已取消'
        const detail=await mockApi.libraryGet(code);if(!detail)throw '影片不在媒体库中'
        const after=clone(detail.meta),changes:RevisionPreview['changes']=[]
        if(edit.kind==='favorite') {if(!!after.favorite!==edit.favorite){changes.push({field:'favorite',before:!!after.favorite,after:edit.favorite});after.favorite=edit.favorite}}
        else {
          const tags=after.tags.filter(v=>!remove.includes(v));for(const value of add)if(!tags.includes(value))tags.push(value)
          if(JSON.stringify(tags)!==JSON.stringify(after.tags)){changes.push({field:'tags',before:[...after.tags],after:tags});after.tags=tags;after.locked_fields=[...new Set([...(after.locked_fields??[]),'tags'])];after.provenance={...after.provenance,tags:'manual'};if(after.name_projection)delete after.name_projection.tags}
        }
        items.push({token,root,dir:detail.dir,code,before:clone(detail.meta),after,changes})
      }
      if(job?.cancelled)throw '已取消'
      if(JSON.stringify(settings)!==JSON.stringify(captured))throw '设置已变化，请重新预览'
      editSettings=captured;editBatch={token,root,items};return {token,root,items:items.map(({code,changes,warnings})=>({code,changes:clone(changes),warnings:clone(warnings)}))}
    }finally{end()}
  },
  async batchEditApply(token,root,requestId){
    const jobId=begin(requestId)
    const p=editBatch?.token===token?editBatch:null,captured=editSettings
    if(p){editBatch=null;editSettings=null}
    try{
     if(!p||p.token!==token||p.root!==root||JSON.stringify(captured)!==JSON.stringify(settings))throw '预览已过期，请重新预览'
     const results:import('../types').RevisionResult[]=[],failed:string[]=[],skipped:string[]=[]
     for(const preview of p.items){await sleep(40);if(job?.cancelled)break
      if(JSON.stringify(preview.before)===JSON.stringify(preview.after)){skipped.push(preview.code);continue}
      try{const detail=await mockApi.libraryGet(preview.code);if(!detail||detail.dir!==preview.dir||JSON.stringify(detail.meta)!==JSON.stringify(preview.before))throw '预览后磁盘内容已变化'
       if(JSON.stringify(captured)!==JSON.stringify(settings))throw '设置已变化，请重新预览'
       const m=movies.find(m=>m.code===preview.code)!,journalId=`batch-${Date.now()}-${jobId}-${results.length}`
       history.unshift({id:journalId,ts:Date.now(),status:'applied',itemCount:1,opCount:2,codes:[preview.code],trashed:0,pendingOperations:0,error:null,added:[],revision:{root,code:preview.code,before:clone(preview.before),after:clone(preview.after),movie:clone(m)}})
       originalDirs.set(preview.code,detail.dir);edited.set(preview.code,clone(preview.after));m.tags=[...preview.after.tags];m.favorite=!!preview.after.favorite;results.push({jobId,root,code:preview.code,journalId,error:null,indexError:null})
       emitMock('library:changed',{jobId});emitMock('batch:edit',{jobId,completed:results.length,failed:failed.length})
      }catch(e){failed.push(`${preview.code}：${e}`)}
     }
     return {jobId,results,failed,skipped,cancelled:job?.cancelled??false}
    }finally{end()}
  },
  async templatePreview(directory,file) {
    const {previewTemplate}=await import('../template-preview')
    return previewTemplate(directory,file)
  },
  async batchEditDiscard(token) {
    if(editBatch?.token===token){editBatch=null;editSettings=null}
  },
  async metadataSourceCandidates(code) {
    const detail=await mockApi.libraryGet(code)
    if(!detail)throw '影片不在媒体库中'
    const contentType=(detail.contentType??'censored') as keyof Settings['routes']
    const route=sourceRoute(code,contentType)
    const candidates: Record<string, import('../types').SourceCandidate[]>=Object.fromEntries(EDITABLE_FIELDS.map(f=>[f.key,[]]))
    for(const cfg of settings.sources.filter(s=>s.enabled&&route.includes(s.id))) {
      const snapshot=detail.meta.raw_sources?.[cfg.id]
      if(!snapshot||snapshot.code!==code||snapshot.content_type!==contentType||snapshot.cache_identity!==await identity(cfg.base,cfg.cookie,settings.proxy,effectiveApi(cfg)))continue
      for(const field of EDITABLE_FIELDS) {
        const value=snapshot.fields[field.key as keyof Metadata]
        if(value==null||Array.isArray(value)&&!value.length||typeof value==='string'&&!value.trim())continue
        candidates[field.key].push({source:cfg.id,value:clone(value),fetchedAt:snapshot.fetched_at})
      }
    }
    return candidates
  },
  async metadataUpdatePreview(code, update,requestId) {
    const id=begin(requestId)
    try {
      const detail=await mockApi.libraryGet(code)
      if (!detail) throw '影片不在媒体库中'
      const after=clone(detail.meta), changes: RevisionPreview['changes']=[]
      const capturedSettings=clone(settings)
      const contentType=(detail.contentType??'censored') as keyof Settings['routes']
      const route=sourceRoute(code,contentType)
      if (update.kind==='refresh' && update.url?.trim()) urlSource(update.url.trim(), route)
      const nonempty=(v: unknown) => v!=null && (Array.isArray(v) ? v.length>0 : typeof v==='string' ? v.trim().length>0 : true)
      if(update.kind==='textVersion'){
        if(!/^[a-zA-Z]{2,8}(?:-[a-zA-Z0-9]{1,8})*$/.test(update.language))throw '语言标签须为 BCP 47，如 zh-Hans、en 或 ja'
        recoverText(after);const canonical=update.language.split('-').map((p,i)=>i===0?p.toLowerCase():p.length===4?p[0].toUpperCase()+p.slice(1).toLowerCase():p.length===2?p.toUpperCase():p.toLowerCase()).join('-');const version=after.text_versions!.localized[canonical]??={}
        for(const field of ['title','plot'] as const){const value=update.text[field];if(value!=null){version[field]=value;version.provenance={...version.provenance,[field]:'manual'};if(update.select&&!after.locked_fields?.includes(field)){after[field]=value;after.provenance={...after.provenance,[field]:`localized:${canonical}`}}}}
      }else if (update.kind==='nfoLanguage') {
        after.nfo_language=update.language
        recoverText(after)
      }else if (update.kind==='revalidateLinks') {
        if (!after.placements?.some(p=>p.mode==='symlink')) throw '没有登记的媒体链接';
        const before=clone(after.placements); after.placements=after.placements.map(p=>({...p,source_proof:'mock-revalidated'}));changes.push({field:'placements',before,after:clone(after.placements)});
      } else if (update.kind==='favorite') {
        if (!!after.favorite !== update.favorite) changes.push({ field: 'favorite', before: !!after.favorite, after: update.favorite })
        after.favorite = update.favorite
      } else if (update.kind==='lock') {
        if (!FIELDS.some(f=>f.key===update.field)) throw '未知锁定字段'
        after.locked_fields=[...new Set([...(after.locked_fields??[]).filter(f=>f!==update.field), ...(update.locked?[update.field]:[])])]
      } else if (update.kind==='source') {
        if (!EDITABLE_FIELDS.some(f=>f.key===update.field)) throw '该字段不支持来源重选'
        const snapshot=after.raw_sources?.[update.source]
        const cfg=settings.sources.find(s=>s.id===update.source && s.enabled)
        if (!snapshot || !cfg || !(await mockApi.metadataSourceCandidates(code))[update.field]?.some(candidate=>candidate.source===update.source)) throw '该字段没有这个来源的有效非空候选，请更新缓存'
        const value=snapshot.fields[update.field as keyof Metadata]
        if (!nonempty(value)) throw '该来源没有这个字段的非空候选'
        Object.assign(after,{[update.field]:clone(value)});invalidateText(after,update.field)
        after.provenance={...after.provenance,[update.field]:update.source};recoverText(after)
        if(snapshot.fields.source_url)after.source_urls={...after.source_urls,[update.source]:publicUrl(snapshot.fields.source_url)}
        else delete after.source_urls[update.source]
        if(after.name_projection)delete after.name_projection[update.field]
        after.locked_fields=(after.locked_fields??[]).filter(f=>f!==update.field)
        Object.assign(after,projectNames(after,settings.name_rules,[update.field]).metadata)
        after.locked_fields=[...new Set([...(after.locked_fields??[]),update.field])]
      } else {
        if (update.fields.some(f=>!EDITABLE_FIELDS.some(e=>e.key===f))) throw '不可更新字段'
        if (!route.length) throw '该内容类型没有可用的数据源'
        const movie=movies.find(m=>m.code===code)!
        // Deterministic fixture source values, independent from manual library edits.
        const {raw_sources: _previousSnapshots, ...baseFields}=clone(detail.meta)
        const raw={...baseFields, title:`${code} 原站标题`, plot:`${code} 原站简介`, studio:movie.studio}
        const snapshots={...after.raw_sources}
        for (const cfg of settings.sources.filter(s=>route.includes(s.id))) {
          if (update.force || !snapshots[cfg.id] || snapshots[cfg.id].cache_identity!==await identity(cfg.base,cfg.cookie,settings.proxy,effectiveApi(cfg))) snapshots[cfg.id]=await sourceSnapshot(code,contentType,cfg,settings.proxy,raw)
        }
        after.raw_sources=snapshots
        const adopted:string[]=[]
        for (const field of update.fields) {
          const chain=[...new Set([...(settings.field_priority[field]??[]).filter(s=>route.includes(s)),...route])].filter(s=>!settings.field_blacklist[field]?.includes(s))
          const site=chain.find(s=>nonempty(snapshots[s]?.fields[field as keyof Metadata]))
          if (!site) continue
          const value=snapshots[site].fields[field as keyof Metadata], old=after[field as keyof Metadata]
          if (after.locked_fields?.includes(field) || !nonempty(value) || (!update.replace && nonempty(old))) continue
          adopted.push(field)
          if(after.name_projection)delete after.name_projection[field]
          Object.assign(after,{[field]:clone(value)});invalidateText(after,field); after.provenance={...after.provenance,[field]:site};recoverText(after)
          if(snapshots[site].fields.source_url)after.source_urls={...after.source_urls,[site]:publicUrl(snapshots[site].fields.source_url!)}
          else delete after.source_urls[site]
        }
        Object.assign(after,projectNames(after,settings.name_rules,adopted).metadata)
      }
      for (const field of EDITABLE_FIELDS.map(f=>f.key)) {
        const old=projectText(detail.meta).meta[field as keyof Metadata], value=projectText(after).meta[field as keyof Metadata]
        if (JSON.stringify(old)!==JSON.stringify(value)) changes.push({field,before:clone(old),after:clone(value)})
      }
      if (job?.cancelled) throw '已取消'
      if (JSON.stringify(settings)!==JSON.stringify(capturedSettings)) throw '媒体库或设置已变化，请重新预览'
      revisionSettings=clone(capturedSettings)
      revision={token:String(id),root:settings.library_root??'/Library',dir:detail.dir,code,before:clone(detail.meta),after,changes,warnings:projectText(after).warnings}
      return clone(revision)
    } finally {end()}
  },
  async revisionSave(token, root, code) {
    const jobId = begin()
    try {
      const p = revision, expectedSettings=revisionSettings; revision = null;revisionSettings=null
      if(expectedSettings&&JSON.stringify(expectedSettings)!==JSON.stringify(settings))throw '设置已变化，预览已过期，请重新预览'
      if (!p || p.token !== token || p.root !== root || p.code !== code || root !== settings.library_root) throw '预览已过期，请重新预览'
      const detail = await mockApi.libraryGet(code)
      if (!detail || JSON.stringify(detail.meta) !== JSON.stringify(p.before)) throw '预览后磁盘内容已变化'
      const m = movies.find(m => m.code === code)!
      const journalId = `revision-${Date.now()}-${jobId}`
      history.unshift({ id: journalId, ts:Date.now(), status: 'applied', itemCount: 1, opCount: 2, codes: [code], trashed: 0, pendingOperations: 0, error: null, added: [], revision: { root, code, before: clone(p.before), after: clone(p.after), movie: clone(m), imageBefore:artworkImages.get(code), imageAfter:croppedArtwork?.token===token?croppedArtwork.image:undefined } })
      if(croppedArtwork?.token===token){artworkImages.set(code,croppedArtwork.image);artworkOriginals.set(code,croppedArtwork.original);croppedArtwork=null}
      originalDirs.set(code, detail.dir); edited.set(code, clone(p.after))
      m.title = p.after.title ?? ''; m.plot = p.after.plot ?? ''; m.actors = p.after.actors.map(a => a.name); m.tags = [...p.after.tags]; m.studio = p.after.studio ?? ''; m.publisher = p.after.publisher ?? ''; m.series = p.after.series; m.releaseDate = p.after.release_date ?? ''; m.runtime = p.after.runtime_min ?? 0; m.favorite = !!p.after.favorite
      emitMock('library:changed', {jobId:job?.id??0})
      return { jobId, root, code, journalId, error: null, indexError: null }
    } finally { end() }
  },
  async openSourceUrl(url) {
    if (!['http:', 'https:'].includes(new URL(url).protocol)) throw '来源链接仅支持 HTTP/HTTPS'
    window.open(url, '_blank', 'noopener,noreferrer')
  },

  async repairImagesStart(code) {
    const id = begin()
    const root = settings.library_root ?? '/Library'
    ;(async () => {
      await sleep(600)
      const cancelled = !!job?.cancelled
      const m = movies.find((x) => x.code === code)
      const poster = !!m?.missingPoster, fanart = !!m?.missingFanart
      const written = cancelled ? 0 : Number(poster) + Number(fanart)
      let journalId: string | null = null
      if (m && written) {
        m.missingPoster = false; m.missingFanart = false
        journalId = `repair-${Date.now()}-${id}`
        history.unshift({ id: journalId, ts:Date.now(), status: 'applied', itemCount: 1, opCount: written, codes: [code], trashed: 0, pendingOperations: 0, error: null, added: [], repaired: { code, poster, fanart } })
        emitMock('library:changed', {jobId:job?.id??0})
      }
      end()
      emitMock('repair:done', { jobId: id, root, code, written, failed: m ? [] : [[code, '影片不在媒体库中']], cancelled, journalId, indexError: null, warnings: m && !written && !cancelled ? ['没有可补的缺失图片'] : [] })
    })()
    return id
  },

  async libraryFacets(): Promise<Facets> {
    const years = new Map<number, number>()
    const types = new Map<string, number>()
    for (const m of movies) {
      years.set(m.year, (years.get(m.year) ?? 0) + 1)
      types.set(m.contentType, (types.get(m.contentType) ?? 0) + 1)
    }
    const count = (kind: FacetKind): [string, number][] => {
      const n = new Map<string, number>()
      for (const m of movies) for (const v of facetValues(m)[kind]) n.set(v, (n.get(v) ?? 0) + 1)
      return [...n].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 300)
    }
    return {
      total: movies.length,
      years: [...years].sort((a, b) => b[0] - a[0]),
      contentTypes: [...types].sort((a, b) => b[1] - a[1]),
      tags: count('tag'), actors: count('actor'), studios: count('studio'), series: count('series'), badges: count('badge'),
      favorites: movies.filter((m) => m.favorite).length,
    }
  },

  async libraryPruneMissing(root, automatic, requestId) {
    if (root !== settings.library_root) throw '媒体库已变化，请重新刷新'
    if (job && automatic) return { root, removed: 0, skipped: true }
    begin(requestId)
    try {
      // Preview has no filesystem; all synthetic movie files remain present.
      if (job?.cancelled) throw '刷新已取消'
      return { root, removed: 0, skipped: false }
    } finally { end() }
  },

  async libraryRebuild() {
    const id = begin()
    ;(async () => {
      const total = 40
      for (let i = 1; i <= total; i++) {
        await sleep(40)
        if (job?.cancelled) break
        emitMock('library:progress', { root: settings.library_root ?? '', jobId: id, done: i, total })
      }
      const cancelled = !!job?.cancelled
      end()
      emitMock('library:rebuilt', { root: settings.library_root ?? '', error: null, jobId: id, count: movies.length, cancelled })
      emitMock('library:changed', {jobId:job?.id??0})
    })()
    return id
  },

  async libraryTrash(code, keepMedia) {
    const id = begin()
    try {
      await sleep(120)
      const i = movies.findIndex((m) => m.code === code)
      if (i < 0) throw '影片不在媒体库中'
      movies.splice(i, 1)
      console.info('[mock] trash', code, keepMedia ? 'metadata' : 'all')
      emitMock('library:changed', { jobId: id })
      return { missing: false, trashed: keepMedia ? 3 : 1 }
    } finally { end() }
  },

  async play(code, part, requestId) {
    await openMedia('play',code,part,requestId)
  },
  async openLogDir() {
    console.info('[mock] open log dir')
  },

  async reveal(code, part, requestId) {
    await openMedia('reveal',code,part,requestId)
  },

  posterUrl: (path, revision) => assetUrl(path, revision),
}
