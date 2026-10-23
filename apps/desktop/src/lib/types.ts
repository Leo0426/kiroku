// 与 Rust 端序列化格式一一对应。
// 约定:kiroku-core 里的 Settings / FileInfo / Code / Metadata 是 snake_case;
// pipeline 与 library 的传输结构(ScanItem、PlanItem、MovieCard…)是 camelCase。

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

export interface SourceCfg {
  id: string
  enabled: boolean
  base: string
  /** 可选 Cookie；空时使用公开匿名会话 */
  cookie?: string
  /** DMM 公开数字影片 GraphQL 完整端点 */
  api_base?: string
  /** 该站代理;省略时跟随全局代理 */
  proxy?: SourceProxy
  /** 文本请求改走用户的求解器(ADR-0007);默认关闭,关闭时省略 */
  solver?: boolean
}
export type SourceProxy = { mode: 'global' } | { mode: 'direct' } | { mode: 'custom'; url: string }
export type ContentType = 'censored' | 'uncensored' | 'chinese' | 'western' | 'fc2' | 'amateur' | 'hentai'

export interface NameRule { canonical: string; aliases: string[] }
export type NameField = 'actors' | 'tags' | 'studio' | 'publisher' | 'series'
/** strip_actor_names_from_title: ADR-0011,关闭时 core 不序列化该键。 */
export type NameRules = Record<NameField, NameRule[]> & { strip_actor_names_from_title?: boolean }
/** 内置标签译名并入名称规则的结果 */
export interface PresetMerge { rules: NameRules; added_rules: number; added_aliases: number }
export type OrganizeMode = 'move' | 'copy' | 'hardlink' | 'symlink'

export type NfoLanguage = 'current' | 'zh-Hans' | 'original'
export interface TextVersion { title?: string | null; plot?: string | null; provenance?: Record<string,string> }
export interface TextVersions { original: TextVersion; localized: Record<string,TextVersion> }
export interface TemplatePreview { directory:string; file:string; warnings:string[]; error:string|null }
export interface Settings {
  name_rules: NameRules

  organize_mode: OrganizeMode

  nfo_language: NfoLanguage
  sources: SourceCfg[]
  routes: Record<string, string[]>
  field_priority: Record<string, string[]>
  field_blacklist: Record<string, string[]>
  min_interval_secs: number
  proxy: string | null
  dir_template: string
  /** 视频文件名模板(不含扩展名);Plex 友好命名开启时不生效 */
  file_template: string
  download_extrafanart: boolean
  retention: RetentionPolicy
  cleanup_source: boolean
  plex: PlexCfg
  media_server: MediaServerCfg
  log: LogCfg
  scan: ScanCfg
  badges: BadgeCfg
  ai: AiCfg
  library_root: string | null
  source_dirs: string[]
  /** 长任务结束时窗口不在前台就发系统通知 */
  notify_on_finish: boolean
  /** 用户自己运行的 FlareSolverr;地址为空时不可用 */
  solver: SolverCfg
}

export interface SolverCfg { url: string; timeout_secs: number }

export interface AiCfg { enabled: boolean; api_key: string; model: string; base_url: string; effort: string; title: boolean; plot: boolean }

/** 运行日志输出位置;verbose 额外记录每个网络请求 */
export interface LogCfg { console: boolean; app: boolean; file: boolean; verbose: boolean }
export type LogLevel = 'error' | 'warn' | 'info' | 'debug'
/** 一行运行日志(log:line 事件);time 为本地 HH:MM:SS.mmm */
export interface LogLineEvent { jobId: number; time: string; level: LogLevel; target: string; message: string }

/** 扫描时哪些视频不算正片;正则按 Rust regex 语法,只匹配文件名 */
export interface ScanCfg { extra_pattern: string; blacklist: string[]; min_video_mb: number }
export type SkipReason = 'extra' | 'blacklisted' | 'tooSmall'
export interface SkippedFile { path: string; size: number; reason: SkipReason }

export type Corner = 'top_left' | 'top_right' | 'bottom_left' | 'bottom_right'
export type BadgeKind = 'subtitle' | 'uncensored' | 'cracked' | 'leaked' | 'definition'
/** 封面角标;corners 某类为 null 表示该类不贴 */
export interface BadgeCfg { enabled: boolean; scale: number; poster: boolean; fanart: boolean; corners: Record<BadgeKind, Corner | null>; /** 随迁的外挂中文字幕也算中字 */ external_subtitle: boolean }

export interface PlexCfg { enabled: boolean; url: string; token: string; naming: boolean }
export interface PlexSection { key: string; title: string; kind: string; locations: string[] }
export interface PlexTest { sections: PlexSection[]; matched: string | null }
/** ADR-0010 Jellyfin / Emby 联动 */
export interface MediaServerCfg { enabled: boolean; kind: 'jellyfin' | 'emby'; url: string; api_key: string; local_prefix: string; server_prefix: string }
export interface ServerTest { serverName: string; version: string; libraries: { name: string; locations: string[] }[]; matched: string | null }

/** 与 kiroku-core 的 aggregate::FIELDS 保持一致 */
export const FIELDS: { key: string; label: string }[] = [
  { key: 'title', label: '标题' },
  { key: 'original_title', label: '原标题' },
  { key: 'plot', label: '简介' },
  { key: 'release_date', label: '发行日期' },
  { key: 'runtime_min', label: '时长' },
  { key: 'studio', label: '片商' },
  { key: 'publisher', label: '发行商' },
  { key: 'series', label: '系列' },
  { key: 'actors', label: '演员' },
  { key: 'tags', label: '类别' },
  { key: 'directors', label: '导演' },
  { key: 'poster_urls', label: '竖版封面' },
  { key: 'thumb_urls', label: '横版封面' },
  { key: 'trailer_urls', label: '预告片' },
  { key: 'extrafanart', label: '剧照' },
  { key: 'score', label: '评分' },
]

/** 内容类型由番号解析得出,决定查哪些站、什么顺序 */
export const CONTENT_TYPES: { key: ContentType; label: string; hint: string }[] = [
  { key: 'censored', label: '有码', hint: 'ABC-123' },
  { key: 'uncensored', label: '无码', hint: 'HEYZO、六位日期号' },
  { key: 'amateur', label: '素人', hint: '259LUXU-1456' },
  { key: 'fc2', label: 'FC2', hint: 'FC2-1234567' },
  { key: 'chinese', label: '国产', hint: 'MD0165' },
  { key: 'western', label: '欧美', hint: 'Vixen.23.04.15' },
  { key: 'hentai', label: '里番', hint: '路径含「里番」' },
]

export const CONTENT_TYPE_LABEL: Record<string, string> = Object.fromEntries(CONTENT_TYPES.map((c) => [c.key, c.label]))

export const SOURCE_NAMES: Record<string, string> = {
  javdb: 'JavDB',
  javbus: 'JavBus',
  jav321: 'Jav321',
  dmm: 'DMM / FANZA 数字影片',
  mgstage: 'MGStage',
  fc2: 'FC2 官方',
  fc2ppvdb: 'FC2PPVDB',
  avsox: 'Avsox',
  javlibrary: 'JavLibrary',
  xcity: 'XCity',
  avbase: 'AVBase',
}

/** Source::supports 对应的可用路线；用户主动加入后生效。 */
export const SOURCE_SUPPORT: Record<string, ContentType[]> = {
  dmm: ['censored'], mgstage: ['amateur'], fc2: ['fc2'], fc2ppvdb: ['fc2'], avsox: ['uncensored'], javlibrary: ['censored'], xcity: ['censored'], avbase: ['censored', 'amateur'],
}
export const SOURCE_HINTS: Record<string, string> = {
  javlibrary: '有码备用源；默认关闭并需手动加入路线。直连会被 Cloudflare 挑战拦住，需要在「网络」里填写求解器地址，并对本站开启「经求解器访问」。可配置镜像域名与 Cookie。',
  avbase: '有码和素人的资料库（标题、简介、演员、类型、剧照、预告片）；默认关闭并需手动加入路线。直连会被 Cloudflare 挑战拦住，需要求解器。同番号有多部作品时优先无厂牌前缀的那部。',
  xcity: '有码备用源；默认关闭并需手动加入路线。逐条核对メーカー品番，缺失字段保持为空；默认带公开年龄 Cookie。',
  dmm: '公开 API 搜索及详情覆盖数字 AV；搜索有目录缺口，网页回退通常需日本出口。TV、租赁及实物目录暂不支持。Jav321 的字段优先级仍需在高级设置中自行调整。',
  mgstage: '素人作品；默认带公开年龄 Cookie adc=1。目前采集出口返回 CloudFront 403，正常页尚未联网验证。',
  fc2: 'FC2 官方公开详情；下架作品会显示未找到，保持原番号前导零。',
  fc2ppvdb: 'FC2 资料库；目前原域名返回 HTTP 526（站点上游 TLS 异常），正常页尚未联网验证。',
  avsox: '无码作品；自动建立匿名 CSRF 会话，无需手动填写 CSRF Cookie。',
}

// ---------------------------------------------------------------------------
// 导入流程(pipeline)
// ---------------------------------------------------------------------------

export type Mosaic = 'censored' | 'uncensored' | 'cracked' | 'leaked'

export interface FileInfo {
  number: string | null
  content_type: ContentType
  prefix: string
  cd: number | null
  has_subtitle: boolean
  mosaic: Mosaic | null
  definition: string | null
  matched: boolean
}

export interface Code {
  kind: 'Standard' | 'Fc2' | 'Dated'
  id: string
  content_type: ContentType
  part: number | null
  subtitled: boolean
  mosaic: Mosaic | null
  definition: string | null
}

export interface ScanItem {
  id: number
  path: string
  size: number
  info: FileInfo
  code: Code | null
}

export type ScrapeStatus =
  | { kind: 'pending' }
  | { kind: 'scraping'; site: string }
  | { kind: 'translating' }
  | { kind: 'found' }
  | { kind: 'notFound' }
  | { kind: 'noRoute' }
  | { kind: 'failed'; message: string }
  | { kind: 'cancelled' }

export type Conflict = 'invalidTemplate' | 'targetExists' | 'duplicateInBatch' | 'invalidLibrary'

export interface PlanItem {
  itemId: number
  code: string
  title: string | null
  from: string
  to: string
  notices?: string[]
  extras: string[]
  conflict: Conflict | null
  conflicts: { kind: Conflict; path: string }[]
  selected: boolean
  /** ADR-0008:作为同分片新版本并存,默认不选 */
  newVersion?: boolean
}

// ---- 事件载荷:都带 jobId ----------------------------------------------------

export interface ScanItemsEvent { jobId: number; items: ScanItem[] }
export interface ScanDone {
  jobId: number; total: number; codes: number; unrecognized: number; cancelled: boolean
  /** 不算正片而跳过的视频(最多 500 条),skippedTotal 为实际总数 */
  skipped: SkippedFile[]; skippedTotal: number
  errors: ScanIssue[]; errorsTotal: number; complete: boolean
}
/** 刮削结果摘要:找到时随事件一起到,界面边刮边展示 */
export interface Preview {
  title: string | null
  originalTitle: string | null
  cover: string | null
  coverIsLandscape: boolean
  releaseDate: string | null
  runtimeMin: number | null
  studio: string | null
  actors: string[]
  score: number | null
  sources: string[]
}
export interface ScrapeEvent { jobId: number; code: string; status: ScrapeStatus; done: number; total: number; preview: Preview | null; note: string | null }
export interface ScrapeDone { jobId: number; found: number; notFound: number; failed: number; cancelled: boolean }
export interface EtaEvent { jobId: number; remainingSecs: number }
export interface ApplyEvent {
  jobId: number
  code: string
  itemId: number
  done: number
  total: number
  ok: boolean
  error: string | null
  warnings: string[]
}
export interface ApplyDone {
  jobId: number
  applied: number
  failed: [string, string][]
  cancelled: boolean
  journalId: string
  /** 移到废纸篓的原文件夹 */
  cleaned: string[]
  indexError: string | null
}
/** done includes failed output tasks; retries and cached candidate URLs do not inflate total. */
export interface ApplyProgress {
  jobId: number
  phase: 'images' | 'organize' | 'index'
  done: number
  total: number
  failed: number
  code: string | null
}
export interface ProgressEvent { root: string; jobId: number; done: number; total: number }
export interface RebuiltEvent { root: string; error: string | null; jobId: number; count: number; cancelled: boolean }

export interface RepairDone {
  jobId: number; root: string; code: string; written: number; failed: [string, string][]; cancelled: boolean
  journalId: string | null; indexError: string | null; warnings: string[]
}

export interface EventMap {
  'job:started': { jobId: number; requestId: string }
  /** Progress only; catalog request ownership is established by job:started. */
  'library:catalog': {jobId:number; phase:string; done:number; total:number; code:string}
  'nfo:export': {jobId:number;completed:number;failed:number}
  /** Progress cannot claim a media request; use its matching job:started event. */
  'media:progress': {jobId:number;root:string;code:string;kind:'play'|'reveal';doneBytes:number;totalBytes:number}
  'repair:event': ApplyEvent
  'repair:done': RepairDone
  'scan:items': ScanItemsEvent
  'scan:done': ScanDone
  'scrape:event': ScrapeEvent
  'scrape:done': ScrapeDone
  'plan:started': { jobId: number; requestId: string | null }
  'job:eta': EtaEvent
  'apply:event': ApplyEvent
  'apply:progress': ApplyProgress
  'apply:done': ApplyDone
  'log:line': LogLineEvent
  'library:progress': ProgressEvent
  'library:rebuilt': RebuiltEvent
  'library:changed': {jobId:number}
  'plex:refreshed': { jobId: number; root: string; ok: boolean; message: string }
  'server:refreshed': { jobId: number; root: string; ok: boolean; message: string }
}

// ---------------------------------------------------------------------------
// 历史
// ---------------------------------------------------------------------------

export type JournalStatus = 'applied' | 'partial' | 'rolledBack'

export interface JournalSummary {
  id: string
  ts: number
  status: JournalStatus
  itemCount: number
  opCount: number
  codes: string[]
  trashed: number
  pendingOperations: number
  error: string | null
}

/** 历史分页:next 是下一页游标(本页最后一条 id),没有更多时为 null */
export interface HistoryPage { items: JournalSummary[]; total: number; next: string | null }
export interface HistoryPageRequest { before?: string | null; limit?: number }

export interface RollbackResult { indexError: string | null; failures: string[]; status: JournalStatus }

// ---------------------------------------------------------------------------
// 媒体库
// ---------------------------------------------------------------------------

export type Sort = 'added' | 'title' | 'year' | 'score'

export type FacetKind = 'tag' | 'actor' | 'studio' | 'series' | 'badge' | 'favorite'
export interface FacetFilter { kind: FacetKind; value: string }
export const FACET_LABEL: Record<FacetKind, string> = { tag: '标签', actor: '演员', studio: '片商', series: '系列', badge: '角标', favorite: '收藏' }
/** 角标维度取值的显示名;清晰度按原值大写 */
export const badgeLabel = (v: string) => ({ subtitle: '中字', uncensored: '无码', cracked: '破解', leaked: '流出' } as Record<string, string>)[v] ?? v.toUpperCase()

export interface Query {
  text?: string
  contentType?: string
  year?: number
  sort?: Sort
  offset?: number
  limit?: number
  /** 维度筛选,全部满足 */
  filters?: FacetFilter[]
}

export interface MovieCard {
  code: string
  title: string | null
  year: number | null
  actors: string[]
  /** 海报绝对路径(Tauri)或 data URL(浏览器 mock) */
  poster: string | null
  posterVersion: string | null
  contentType: string | null
  score: number | null
  addedAt: number
  parts: number
  favorite: boolean
}

export interface Actor { name: string; gender: 'female' | 'male' | 'unknown'; /** ADR-0009 头像 URL */ thumb?: string | null }

export interface Metadata {
  asset_stem?: string | null
  name_projection?: Record<string, { original: unknown; applied: unknown; fingerprint: string; warnings?:string[] }>
  placements?: { path: string; mode: OrganizeMode; source: string; source_proof: string; size: number; anchor?: string | null }[]
  text_versions?: TextVersions
  nfo_language?: NfoLanguage
  artwork?: Artwork

  code: string
  title: string | null
  original_title: string | null
  plot: string | null
  release_date: string | null
  runtime_min: number | null
  studio: string | null
  publisher: string | null
  series: string | null
  actors: Actor[]
  tags: string[]
  directors: string[]
  scores: { site: string; score: number }[]
  provenance?: Record<string, string>
  locked_fields?: string[]
  /** 收藏只存 sidecar */
  favorite?: boolean
  raw_sources?: Record<string, SourceSnapshot>
  source_urls: Record<string, string>
}

export interface MovieFile { path: string; part: number | null; size: number }

export interface MovieDetail extends MovieCard {
  meta: Metadata
  dir: string
  files: MovieFile[]
  fanart: string | null
  fanartVersion: string | null
}

export interface Page { total: number; items: MovieCard[] }

export interface Facets {
  total: number
  years: [number, number][]
  contentTypes: [string, number][]
  tags: [string, number][]
  actors: [string, number][]
  studios: [string, number][]
  series: [string, number][]
  badges: [string, number][]
  favorites: number
}

/** 文件夹体检结果(选目录时用来给提醒) */
export interface DirInfo { videos: number; nfos: number; truncated: boolean }

/** 拖入窗口的路径,按跟随软链接后的类型归类 */
export interface DroppedPath { path: string; kind: 'dir' | 'video' | 'file' | 'missing' }

/** 与 kiroku-core 的 VIDEO_EXT 一致 */
export const VIDEO_EXT = ['mp4', 'mkv', 'avi', 'wmv', 'mov', 'ts', 'm2ts', 'flv', 'rmvb']
/** 待整理来源是单个视频文件(而不是文件夹) */
export const isVideoPath = (p: string) => VIDEO_EXT.includes(p.split('.').pop()?.toLowerCase() ?? '') && /\.[^\\/]+$/.test(p)

/** 已记住的待整理文件夹里的视频(媒体库页的「待整理」标签) */
export interface ScanIssue { path: string; message: string }
export interface ScanSummary {
  total: number; skippedTotal: number; visited: number
  errors: ScanIssue[]; errorsTotal: number
  cancelled: boolean; truncated: boolean; complete: boolean
}
export interface PendingSummary extends ScanSummary {
  dirs: string[]; checkedAt: number; cached: boolean
}
export interface PendingList extends PendingSummary {
  items: ScanItem[]; codes: number; unrecognized: number
}

/** 系统媒体目录:默认媒体库位置与选文件夹对话框的起点 */
export interface MediaDirs { videos: string | null; downloads: string | null; suggestedLibrary: string | null }

export const EDITABLE_FIELDS = FIELDS.filter(f => ['title', 'plot', 'actors', 'tags', 'studio', 'publisher', 'series', 'release_date', 'runtime_min', 'directors'].includes(f.key))
export interface RevisionPreview {
  token: string; root: string; dir: string; code: string; before: Metadata; after: Metadata
  warnings?: string[]
  changes: { field: string; before: unknown; after: unknown }[]
}
export interface RevisionResult { jobId: number; root: string; code: string; journalId: string; error: string | null; indexError: string | null }

export interface SourceSnapshot { code: string; content_type: string; base: string; base_identity?: string; cache_identity?: string; fetched_at: number; fields: Partial<Metadata> & { source_url?: string | null } }
export type MetadataUpdate = {kind:'revalidateLinks'} | { kind: 'favorite'; favorite: boolean } | {kind:'textVersion';language:string;text:TextVersion;select:boolean} | { kind: 'nfoLanguage'; language:NfoLanguage } | { kind: 'lock'; field: string; locked: boolean } | { kind: 'source'; field: string; source: string } | { kind: 'refresh'; fields: string[]; replace: boolean; force: boolean; url?: string }

export interface SourceCandidate { source: string; value: unknown; fetchedAt: number }

export interface CropBox { x:number; y:number; width:number; height:number }
export interface ArtworkOriginal { token:string; root:string; code:string; width:number; height:number; image:number[]; crop:CropBox|null; source:string|null }
export interface ArtworkPreview { revision:RevisionPreview; image:number[] }
export interface Artwork {original:string; digest:string; source_url:string|null; crop:CropBox|null; phase:{subtitle:boolean;uncensored:boolean;cracked:boolean;leaked:boolean;definition:string|null}}

export interface AppendPreview {token:string;root:string;code:string;targetDir:string;ops:({MoveFile?:{from:string;to:string};CopyFile?:{from:string;to:string};HardlinkFile?:{from:string;to:string};SymlinkFile?:{from:string;to:string}})[];skipped:string[];conflicts:string[];newVersions?:string[]}
export interface AppendResult {journalId:string;completed:number;skipped:string[];failed:string[];cancelled:boolean}

/** JavDB reason also reports authenticated / unauthenticated / authUnknown; ok alone means reachability. */
export interface SourceCheck {sourceId:string;outcome:{status:string;reason:string;httpStatus:number|null};elapsedMs:number;checkedAt:number}

export interface HealthIssue {kind:string;severity:string;dir:string;code:string|null;path:string;message:string;action:string}
export interface HealthReport {root:string;scannedAt:number;scanned:number;cancelled:boolean;issues:HealthIssue[]}

export interface RetentionPolicy {historyDays:number;logDays:number}
export interface MaintenancePreview {token:string;root:string;logDir:string;policy:RetentionPolicy;candidates:{id:string;kind:string;paths:string[];bytes:number;description:string}[];protected:string[];candidateBytes:number;resourceBytes?:number;protectedBytes?:number;scannedAt:number}
export interface MaintenanceResult {removed:number;skipped:string[];failed:string[];cancelled:boolean}

export interface CatalogMovie {code:string; dir:string; names:Record<NameField,string[]>}
export interface CatalogInventory {movies:CatalogMovie[]; names:Record<string,Record<string,number>>; errors:string[]; cancelled:boolean}
export interface CatalogPreview {token:string; root:string; fingerprint:string; items:Omit<RevisionPreview,'token'>[]; warnings:string[]}
export interface CatalogResult {jobId:number; items:{code:string;dir:string;journalId:string|null;error:string|null}[];cancelled:boolean;indexError:string|null}
export interface LanguageBatchPreview { token:string; root:string; items:RevisionPreview[] }
export interface LanguageBatchResult { jobId:number; results:RevisionResult[]; failed:string[]; cancelled:boolean }

export interface RecoveryEntry {id:string;kind:'partial'|'pending'|'corrupt';codes:string[];pendingOperations:number;message:string;canUndo:boolean}
export interface RecoveryReport {root:string;entries:RecoveryEntry[]}

/** Unique request ID scopes read cancellation; summaries never authorize undo. */
export type HistoryRequestId = string

/** 删除影片的结果;missing 表示目录早已不在磁盘上,只清了索引 */
export interface TrashResult { missing: boolean; trashed: number }
