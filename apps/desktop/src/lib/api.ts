import { inTauri } from './events'
import { assetUrl } from './asset-url'
import type {
  HistoryRequestId, HistoryPage, HistoryPageRequest,
  RecoveryReport,
  CatalogInventory, CatalogPreview, CatalogResult,
  AppendPreview, AppendResult, SourceCheck, HealthReport, MaintenancePreview, MaintenanceResult,
  RevisionPreview, ArtworkOriginal, ArtworkPreview, CropBox,
  MetadataUpdate, TemplatePreview, NfoLanguage, LanguageBatchPreview, LanguageBatchResult,
  SourceCandidate,
  RevisionResult,
  AiCfg, NameRules, PresetMerge,
  DirInfo,
  DroppedPath,
  Facets,
  JournalSummary,
  MediaDirs,
  MovieDetail,
  Page,
  PendingList, PendingSummary,
  PlanItem,
  PlexTest, MediaServerCfg, ServerTest,
  Query,
  RollbackResult,
  ScanItem,
  Settings,
  TrashResult, LibraryPruned,
} from './types'

/** 后端能力的统一接口。Tauri 里走 invoke,浏览器里走 mock 引擎(便于不启动桌面壳就预览/验证界面)。 */
export interface Api {
  recoveryInspect():Promise<RecoveryReport>
  catalogInventory(requestId?:string):Promise<CatalogInventory>
  catalogPreview(dirs:string[],requestId?:string):Promise<CatalogPreview>
  catalogApply(token:string,requestId?:string):Promise<CatalogResult>
  maintenancePreview(includeRecent?:boolean,requestId?:string):Promise<MaintenancePreview>
  maintenanceApply(token:string,selected:string[],requestId?:string):Promise<MaintenanceResult>
  libraryHealth(requestId?:string):Promise<HealthReport>
  revealLibraryPath(path:string):Promise<void>
  sourcesCheck(ids?:string[], draft?:Settings, requestId?:string):Promise<SourceCheck[]>
  pickMediaFiles():Promise<string[]>
  appendTargetVideos(code:string,target:string,requestId?:string):Promise<string[]>
  appendTargets(code:string,requestId?:string):Promise<string[]>
  appendPreview(code:string,inputs:string[],target?:string,video?:string,requestId?:string):Promise<AppendPreview>
  appendApply(token:string,requestId?:string):Promise<AppendResult>
  getSettings(): Promise<Settings>
  defaultSettings(): Promise<Settings>
  settingsPath(): Promise<string>
  saveSettings(settings: Settings): Promise<void>
  /** create 为真时目录不存在就创建(用于默认位置) */
  setLibraryRoot(path: string, create?: boolean): Promise<Settings>
  /** start:对话框初始打开的目录 */
  pickDirectory(title?: string, start?: string | null): Promise<string | null>
  mediaDirs(): Promise<MediaDirs>
  /** 测试 Plex 连接:列出分区并找出包含媒体库目录的那个 */
  plexTest(url: string, token: string): Promise<PlexTest>
  /** 测试 Jellyfin / Emby 连接(未保存的设置) */
  mediaServerTest(cfg: MediaServerCfg): Promise<ServerTest>
  /** 用给定的 AI 设置试译一句 */
  /** 使用所选 provider 的未保存配置试译；与全局标题/简介开关无关。 */
  aiTest(cfg: AiCfg): Promise<{ title: string | null; plot: string | null }>
  inspectDirectory(path: string): Promise<DirInfo>
  /** 清空 AI 汉化译文缓存 */
  clearTranslationCache(): Promise<void>
  /** 长任务结束的系统通知;设置关闭或窗口在前台时不发,返回是否发出 */
  notifyFinish(title: string, body: string): Promise<boolean>
  /** 把内置标签译名并入(尚未保存的)名称规则,不改动已有规则 */
  tagPresetMerge(rules: NameRules): Promise<PresetMerge>
  /** 只读归类拖入的路径;不是扫描授权 */
  classifyPaths(paths: string[]): Promise<DroppedPath[]>

  /** 已记住的待整理文件夹里还没整理的视频 */
  pendingList(requestId?: string, force?: boolean): Promise<PendingList>
  pendingSummary(requestId?: string, force?: boolean): Promise<PendingSummary>
  pendingCancel(requestId: string): Promise<boolean>
  setSourceDirs(dirs: string[]): Promise<Settings>
  scanStart(dirs: string[], rememberSources?: boolean): Promise<number>
  /** 手动指定/修正某个文件的番号;无法识别会抛错 */
  setItemCode(itemId: number, code: string): Promise<ScanItem>
  scrapeStart(itemIds: number[]): Promise<number>
  /** 预览冲突包含视频、字幕和全部辅助产物的具体路径。 */
  planBuild(itemIds: number[], requestId?: string): Promise<PlanItem[]>
  applyStart(itemIds: number[]): Promise<number>
  /** 用某站详情页重刮一部影片;结果经 scrape:event / scrape:done 返回 */
  scrapeUrlStart(itemIds: number[], url: string): Promise<number>
  // Preparation progress is delivered through EventMap['apply:progress'] before
  // apply:event, and may arrive before applyStart resolves (same jobId contract).
  jobCancel(jobId?:number): Promise<boolean>

  /** 按 id 从新到旧分页;只读取本页日志 */
  historyDetail(requestId:string, journalId:string, offset?:number): Promise<import('./types').HistoryDetail>
  historyList(requestId?:HistoryRequestId, page?:HistoryPageRequest): Promise<HistoryPage>
  historyCancel(requestId:HistoryRequestId):Promise<boolean>
  revealJournal(journalId: string): Promise<void>
  rollback(journalId: string): Promise<RollbackResult>

  libraryOverview(query: Query): Promise<import('./types').LibraryOverview>
  libraryList(query: Query): Promise<Page>
  libraryExportCsv(query: Query, requestId?: string): Promise<number | null>
  libraryPruneMissing(root: string, automatic: boolean, requestId?: string): Promise<LibraryPruned>
  libraryGet(code: string): Promise<MovieDetail | null>
  /** 移到系统废纸篓并移出索引。keepMedia:只移走资料(NFO/图片等),视频和字幕留下;否则整个影片目录 */
  libraryTrash(code: string, keepMedia: boolean): Promise<TrashResult>
  repairImagesStart(code: string): Promise<number>
  metadataSourceCandidates(code: string): Promise<Record<string, SourceCandidate[]>>
  batchEditPreview(codes:string[],edit:import('./types').BatchEdit,requestId?:string):Promise<import('./types').BatchEditPreview>
  batchEditDiscard(token:string):Promise<void>
  batchEditApply(token:string,root:string,requestId?:string):Promise<import('./types').BatchEditResult>
  nfoExportPreview(codes:string[],language:NfoLanguage,requestId?:string):Promise<LanguageBatchPreview>
  nfoExportApply(token:string,root:string,requestId?:string):Promise<LanguageBatchResult>
  templatePreview(directory:string,file:string): Promise<TemplatePreview>
  metadataUpdatePreview(code: string, update: MetadataUpdate, requestId?:string): Promise<RevisionPreview>
  artworkOpen(code:string,requestId?:string):Promise<ArtworkOriginal>
  artworkPreview(token:string,root:string,code:string,crop:CropBox,requestId?:string):Promise<ArtworkPreview>
  revisionPreview(code: string, patch: Record<string, unknown>): Promise<RevisionPreview>
  revisionSave(token: string, root: string, code: string): Promise<RevisionResult>
  openSourceUrl(url: string): Promise<void>
  libraryFacets(): Promise<Facets>
  libraryRebuild(): Promise<number>

  /** job:started establishes ownership; media:progress reports validation before opening. */
  play(code: string, part?: number, requestId?: string): Promise<void>
  reveal(code: string, part?: number, requestId?: string): Promise<void>
  /** 打开日志文件夹(<设置目录>/logs) */
  openLogDir(): Promise<void>

  /** 把后端给的海报路径转成 <img> 可用的 URL */
  posterUrl(path: string | null, revision?: string | null): string | null
}

async function tauriApi(): Promise<Api> {
  const { invoke, convertFileSrc } = await import('@tauri-apps/api/core')
  return {
    catalogInventory:(requestId)=>invoke('catalog_inventory',{requestId}),
    catalogPreview:(dirs,requestId)=>invoke('catalog_preview',{dirs,requestId}),
    catalogApply:(token,requestId)=>invoke('catalog_apply',{token,requestId}),
    maintenancePreview:(includeRecent=false,requestId)=>invoke('maintenance_preview',{includeRecent,requestId}),
    maintenanceApply:(token,selected,requestId)=>invoke('maintenance_apply',{token,selected,requestId}),
    libraryHealth:(requestId)=>invoke('library_health',{requestId}),
    revealLibraryPath:(path)=>invoke('reveal_library_path',{path}),
    sourcesCheck:(ids,draft,requestId)=>invoke('sources_check',{ids,draft,requestId}),
    pickMediaFiles:()=>invoke('pick_media_files'),
    appendTargetVideos:(code,target,requestId)=>invoke('append_target_videos',{code,target,requestId}),
    appendTargets:(code,requestId)=>invoke('append_targets',{code,requestId}),
    appendPreview:(code,inputs,target,video,requestId)=>invoke('append_preview',{code,inputs,target,video,requestId}),
    appendApply:(token,requestId)=>invoke('append_apply',{token,requestId}),
    getSettings: () => invoke('get_settings'),
    defaultSettings: () => invoke('default_settings'),
    settingsPath: () => invoke('settings_path'),
    saveSettings: (settings) => invoke('save_settings', { settings }),
    setLibraryRoot: (path, create) => invoke('set_library_root', { path, create }),
    pickDirectory: (title, start) => invoke('pick_directory', { title, start }),
    mediaDirs: () => invoke('media_dirs'),
    plexTest: (url, token) => invoke('plex_test', { url, token }),
    mediaServerTest: (cfg) => invoke('media_server_test', { cfg }),
    aiTest: (cfg) => invoke('ai_test', { cfg }),
    inspectDirectory: (path) => invoke('inspect_directory', { path }),
    clearTranslationCache: () => invoke('translation_cache_clear'),
    notifyFinish: (title, body) => invoke('notify_finish', { title, body }),
    tagPresetMerge: (rules) => invoke('tag_preset_merge', { rules }),
    classifyPaths: (paths) => invoke('classify_paths', { paths }),
    pendingList: (requestId = crypto.randomUUID(), force = false) => invoke('pending_list', { requestId, force }),
    pendingSummary: (requestId = crypto.randomUUID(), force = false) => invoke('pending_summary', { requestId, force }),
    pendingCancel: (requestId) => invoke('pending_cancel', { requestId }),
    setSourceDirs: (dirs) => invoke('set_source_dirs', { dirs }),
    scanStart: (dirs, rememberSources = true) => invoke('scan_start', { dirs, rememberSources }),
    setItemCode: (itemId, code) => invoke('set_item_code', { itemId, code }),
    scrapeStart: (itemIds) => invoke('scrape_start', { itemIds }),
    planBuild: (itemIds, requestId) => invoke('plan_build', { itemIds, requestId }),
    applyStart: (itemIds) => invoke('apply_start', { itemIds }),
    scrapeUrlStart: (itemIds, url) => invoke('scrape_url_start', { itemIds, url }),
    jobCancel: (jobId) => invoke('job_cancel',{jobId}),
    recoveryInspect:()=>invoke('recovery_inspect'),
    historyDetail: (requestId, journalId, offset = 0) => invoke('history_detail', { requestId, journalId, offset }),
    historyList: (requestId, page = {}) => invoke('history_list', {requestId, before: page.before ?? null, limit: page.limit ?? null, filter:page.filter ?? null}),
    historyCancel: (requestId) => invoke('history_cancel', {requestId}),
    revealJournal: (journalId) => invoke('reveal_journal', { journalId }),
    rollback: (journalId) => invoke('rollback', { journalId }),
    libraryOverview: (query) => invoke('library_overview', { query }),
    libraryList: (query) => invoke('library_list', { query }),
    libraryExportCsv: (query, requestId) => invoke('library_export_csv', { query, requestId }),
    libraryPruneMissing: (root, automatic, requestId) => invoke('library_prune_missing', { root, automatic, requestId }),
    libraryGet: (code) => invoke('library_get', { code }),
    libraryTrash: (code, keepMedia) => invoke('library_trash', { code, keepMedia }),
    repairImagesStart: (code) => invoke('repair_images_start', { code }),
    metadataSourceCandidates: (code) => invoke('metadata_source_candidates', { code }),
    batchEditPreview: (codes,edit,requestId) => invoke('batch_edit_preview',{codes,edit,requestId}),
    batchEditApply: (token,root,requestId) => invoke('batch_edit_apply',{token,root,requestId}),
    batchEditDiscard: (token) => invoke('batch_edit_discard',{token}),
    nfoExportPreview:(codes,language,requestId)=>invoke('nfo_export_preview',{codes,language,requestId}),
    nfoExportApply:(token,root,requestId)=>invoke('nfo_export_apply',{token,root,requestId}),
    templatePreview: (directory,file) => invoke('template_preview', {directory,file}),
    metadataUpdatePreview: (code, update,requestId) => invoke('metadata_update_preview', { code, update,requestId}),
    artworkOpen: (code,requestId) => invoke('artwork_open', { code,requestId}),
    artworkPreview: (token,root,code,crop,requestId) => invoke('artwork_preview', {token,root,code,crop,requestId}),
    revisionPreview: (code, patch) => invoke('revision_preview', { code, patch }),
    revisionSave: (token, root, code) => invoke('revision_save', { token, root, code }),
    openSourceUrl: (url) => invoke('open_source_url', { url }),
    libraryFacets: () => invoke('library_facets'),
    libraryRebuild: () => invoke('library_rebuild'),
    play: (code, part, requestId) => invoke('play', { code, part, requestId }),
    reveal: (code, part, requestId) => invoke('reveal', { code, part, requestId }),
    openLogDir: () => invoke('open_log_dir'),
    // A revision replaces bytes at the same path; refreshed cards must request new bytes.
    posterUrl: (path, revision) => assetUrl(path, revision, convertFileSrc),
  }
}

async function load(): Promise<Api> {
  return inTauri ? await tauriApi() : (await import('./mock/engine')).mockApi
}

// 顶层 await:应用启动前选好实现
export const api: Api = await load()
