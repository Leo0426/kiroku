import { defaultImportRules, projectImportRules, checkImportName } from './import-rules'
// 导入流程状态:选目录 → 扫描 → 刮削 → 计划 → 执行。
// 放在模块级单例里,切到别的页面后任务仍在后台跑,回来能看到最新进度。

import { api } from './api'
import { listen } from './events'
import { app } from './state.svelte'
import { cancelPendingReads } from './pending'
import { baseName } from './format'
import { applyNotice, notifyFinish, scrapeNotice } from './notify'
import type { ApplyDone, ApplyEvent, ApplyProgress, DroppedPath, ImportRules, LocalImportEntry, LogLineEvent, OrganizeMode, PlanItem, Preview, RollbackResult, ScanDone, ScanItem, ScrapeDone, ScrapeStatus } from './types'

const LOG_MAX = 3000
/** 生成计划只做计算和少量文件检查,正常远低于这个时间;超时则报错而不是一直卡在「正在检查目标」 */
const PLAN_TIMEOUT_MS = 60_000

function withTimeout<T>(p: Promise<T>, ms: number, message: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(message), ms)
    p.then(resolve, reject).finally(() => clearTimeout(timer))
  })
}

export type Step = 'source' | 'scan' | 'scrape' | 'plan' | 'apply' | 'done'
type Kind = 'scan' | 'scrape' | 'plan' | 'apply'
type TaskAttempt = { cancelled: boolean; cancelSent: boolean }
type ImportSource = { path: string; kind: 'video' | 'dir' }

class ImportFlow {
  step = $state<Step>('source')
  // 未编辑时展示设置中的来源；显式清空也是一次有效选择。
  private sourceDraft = $state<ImportSource[] | null>(null)
  sources = $derived(this.sourceDraft ?? (app.settings?.source_dirs ?? []).map((path): ImportSource => ({ path, kind: 'dir' })))
  readingSources = $state(0)
  error = $state<string | null>(null)

  items = $state<ScanItem[]>([])
  scanRunning = $state(false)
  scanSummary = $state<ScanDone | null>(null)
  localImports = $state<LocalImportEntry[]>([])
  correctingCode = $state(false)

  statuses = $state<Record<string, ScrapeStatus>>({})
  /** 已找到的结果摘要,按找到的先后排列 */
  found = $state<{ code: string; preview: Preview }[]>([])
  foundByCode = $derived(Object.fromEntries(this.found.map(entry => [entry.code, entry.preview])))
  /** 附加提示(如 AI 汉化失败),按番号 */
  notes = $state<Record<string, string>>({})
  scrape = $state({ done: 0, total: 0, running: false })
  scrapeSummary = $state<ScrapeDone | null>(null)
  eta = $state<number | null>(null)

  rules = $state<ImportRules | null>(null)
  effectiveRules = $derived(this.rules ?? (app.settings ? defaultImportRules(app.settings) : null))
  renames = $state<Record<number, string>>({})
  skipped = $state<Record<number, boolean>>({})
  planToken = $state<string | undefined>()
  editingRules = $state(false)
  savingRules = $state(false)
  planning = $state(false)
  plans = $state<PlanItem[]>([])
  picked = $state<Record<number, boolean>>({})
  private planOrigin: 'scan' | 'scrape' = 'scrape'

  apply = $state({ done: 0, total: 0, running: false })
  applyProgress = $state<ApplyProgress | null>(null)
  /** 本批执行时的文件处理方式，离开页面或修改设置后仍显示实际操作。 */
  applyMode = $state<OrganizeMode>('move')
  applyLog = $state<ApplyEvent[]>([])
  /** 整理任务的运行日志(log:line),最多保留 LOG_MAX 行 */
  logLines = $state<LogLineEvent[]>([])
  applyDone = $state<ApplyDone | null>(null)
  applyRoot = $state<string | null>(null)
  batchInCurrentLibrary = $derived(!!this.applyDone && !!this.applyRoot && this.applyRoot === app.settings?.library_root)
  batchRollback = $state<RollbackResult | null>(null)
  batchUndone = $derived(this.batchRollback?.status === 'rolledBack' && this.batchRollback.failures.length === 0)
  undoing = $state<{ root: string; id: string } | null>(null)

  recognized = $derived(this.items.filter((i) => i.code))
  unrecognized = $derived(this.items.filter((i) => !i.code))
  /** 去重后的番号,保持扫描顺序 */
  uniqueCodes = $derived([...new Set(this.recognized.map((i) => i.code!.id))])
  retryCodes = $derived(this.uniqueCodes.filter((code) => this.statuses[code]?.kind !== 'found'))
  readyCount = $derived(this.uniqueCodes.length - this.retryCodes.length)
  localReady = $derived(this.localImports.filter(entry => entry.reusable))
  localByCode = $derived(Object.fromEntries(this.localImports.map(entry => [entry.code, entry])))
  foundIds = $derived(this.recognized.filter((i) => this.statuses[i.code!.id]?.kind === 'found').map((i) => i.id))
  selectedCount = $derived(this.plans.filter((p) => this.picked[p.itemId] && !p.conflict && p.actions?.length !== 0).length)
  conflictCount = $derived(this.plans.filter((p) => p.conflict).length)
  private taskBusy = $derived(this.savingRules || this.scanRunning || this.correctingCode || this.scrape.running || this.apply.running || this.planning || !!this.undoing)
  busy = $derived(this.taskBusy || this.readingSources > 0)
  /** 只在选择来源步骤且没有任务时接收拖入,避免打断进行中的一批 */
  canAcceptDrop = $derived(!this.taskBusy && this.step === 'source')

  private expecting: Kind | null = null
  private jobId: number | null = null
  private started = false
  private retiredJobId = 0
  private scanAttempt: { cancelled: boolean; dispatched: boolean } | null = null
  private planningAttempt: { requestId: string; cancelled: boolean; jobId: number | null } | null = null
  private taskAttempt: TaskAttempt | null = null
  private autoPlan = false
  private pendingLogLines: LogLineEvent[] = []

  /** 事件可能先于 invoke 的返回值到达,所以第一个事件带来的 jobId 视为当前任务 */
  private accept(kind: Kind, jobId: number): boolean {
    if (this.expecting !== kind || jobId <= this.retiredJobId) return false
    if (this.jobId === null) {
      this.jobId = jobId
      if (kind === 'apply') {
        this.logLines.push(...this.pendingLogLines.filter(line => line.jobId === jobId))
        this.pendingLogLines = []
      }
      if (this.taskAttempt) void this.cancelTask(this.taskAttempt)
    }
    return this.jobId === jobId
  }

  private finish() {
    this.retiredJobId = Math.max(this.retiredJobId, this.jobId ?? 0)
    this.expecting = null
    this.jobId = null
    this.taskAttempt = null
    this.pendingLogLines = []
  }

  /** 启动响应可能晚于完成事件；只允许仍归本次请求所有的响应改变流程。 */
  private async startTask(kind: 'scrape' | 'apply', start: () => Promise<number>, onError: () => void) {
    const attempt: TaskAttempt = { cancelled: false, cancelSent: false }
    this.taskAttempt = attempt
    this.expecting = kind
    this.jobId = null
    try {
      const id = await start()
      if (this.taskAttempt === attempt) this.accept(kind, id)
    } catch (e) {
      if (this.taskAttempt !== attempt) return
      this.error = String(e)
      onError()
      this.finish()
    }
  }

  /** 停止意图可早于任务编号到达，绝不使用未指定编号的全局取消。 */
  private async cancelTask(attempt: TaskAttempt) {
    if (this.taskAttempt !== attempt || !attempt.cancelled || attempt.cancelSent || this.jobId === null) return
    attempt.cancelSent = true
    try {
      await api.jobCancel(this.jobId)
    } catch (e) {
      if (this.taskAttempt !== attempt) return
      attempt.cancelSent = false
      this.error = String(e)
    }
  }

  async init() {
    if (this.started) return
    const unlisten: (() => void)[] = []
    try {
      unlisten.push(await listen('scan:items', (e) => {
        if (!this.accept('scan', e.jobId)) return
        this.items.push(...e.items)
      }))
      unlisten.push(await listen('scan:done', (e) => {
        if (!this.accept('scan', e.jobId)) return
        this.scanRunning = false
        this.scanSummary = e
        if (e.total === 0 && e.errorsTotal > 0) this.items = []
        this.setLocalResults(e.localImports ?? [])
        this.finish()
      }))
      unlisten.push(await listen('scrape:event', (e) => {
        if (!this.accept('scrape', e.jobId)) return
        this.statuses[e.code] = e.status
        if (e.preview) {
          // 按详情页重刮会替换同一番号之前的结果
          const i = this.found.findIndex((f) => f.code === e.code)
          if (i >= 0) this.found[i] = { code: e.code, preview: e.preview }
          else this.found.push({ code: e.code, preview: e.preview })
        }
        if (e.note) this.notes[e.code] = e.note
        this.scrape.done = e.done
        this.scrape.total = e.total
      }))
      unlisten.push(await listen('job:eta', (e) => {
        if (this.expecting === 'scrape' && this.accept('scrape', e.jobId)) this.eta = e.remainingSecs
      }))
      unlisten.push(await listen('scrape:done', (e) => {
        if (!this.accept('scrape', e.jobId)) return
        if (e.cancelled) {
          for (const { code } of this.found) {
            if (['pending', 'scraping', 'translating', 'cancelled'].includes(this.statuses[code]?.kind)) this.statuses[code] = { kind: 'found' }
          }
        }
        this.scrape.running = false
        this.scrapeSummary = e
        this.eta = null
        const advance = this.autoPlan && !e.cancelled && this.retryCodes.length === 0
        this.autoPlan = false
        this.finish()
        if (advance) void this.buildPlan('scan')
        else notifyFinish(scrapeNotice(e))
      }))
      unlisten.push(await listen('plan:started', (e) => {
        const attempt = this.planningAttempt
        if (!attempt || e.requestId !== attempt.requestId || e.jobId <= this.retiredJobId) return
        if (attempt.jobId !== null && attempt.jobId !== e.jobId) return
        attempt.jobId = e.jobId
        this.jobId = e.jobId
        if (attempt.cancelled) void api.jobCancel(e.jobId).catch(error => { this.error = String(error) })
      }))
      unlisten.push(await listen('apply:progress', (e) => {
        if (!this.accept('apply', e.jobId)) return
        this.applyProgress = e
      }))
      unlisten.push(await listen('apply:event', (e) => {
        if (!this.accept('apply', e.jobId)) return
        this.applyLog.push(e)
        this.apply.done = e.done
        this.apply.total = e.total
      }))
      unlisten.push(await listen('log:line', (e) => {
        // 各类任务共用日志总线；只有整理事件或启动响应可以确认归属。
        if (this.expecting !== 'apply' || e.jobId === 0 || e.jobId <= this.retiredJobId) return
        if (this.jobId !== null && e.jobId !== this.jobId) return
        const lines = this.jobId === null ? this.pendingLogLines : this.logLines
        lines.push(e)
        if (lines.length > LOG_MAX) lines.splice(0, lines.length - LOG_MAX)
      }))
      unlisten.push(await listen('apply:done', (e) => {
        if (!this.accept('apply', e.jobId)) return
        this.apply.running = false
        this.applyDone = e
        this.step = 'done'
        this.finish()
        notifyFinish(applyNotice(e, this.applyLog.filter(item => item.warnings.length).length))
        app.libraryVersion++
      }))
      this.started = true
    } catch (e) {
      for (const stop of unlisten) stop()
      throw e
    }
  }

  reset(reason: 'newBatch' | 'reselectSources' = 'newBatch') {
    if (this.busy) return
    this.step = 'source'
    if (reason === 'newBatch') {
      this.sourceDraft = null
      this.rules = null
      this.cachedSourcesOnly = false
    }
    this.renames = {}
    this.skipped = {}
    this.planToken = undefined
    this.items = []
    this.codeErrors = {}
    this.scanSummary = null
    this.setLocalResults([])
    this.scrape = { done: 0, total: 0, running: false }
    this.scrapeSummary = null
    this.eta = null
    this.plans = []
    this.picked = {}
    this.apply = { done: 0, total: 0, running: false }
    this.applyLog = []
    this.applyProgress = null
    this.logLines = []
    this.applyDone = null
    this.applyRoot = null
    this.batchRollback = null
    this.error = null
  }

  /**
   * 把文件夹或视频并入待整理列表，末尾分隔符归一后去重。
   * 待整理目录不能在媒体库里面:库内的文件扫描时会被跳过,选了只会得到空结果。
   */
  private addSources(entries: ImportSource[]): { inLibrary: string[] } {
    const lib = app.settings?.library_root
    const norm = (p: string) => p.replace(/(?<=[^:\\/])[\\/]+$/, '')
    const inLibrary: string[] = []
    const next = new Map(this.sources.map(source => [source.path, source]))
    for (const entry of entries) {
      const path = norm(entry.path)
      if (lib && (path === norm(lib) || path.startsWith(norm(lib) + '/') || path.startsWith(norm(lib) + '\\'))) inLibrary.push(path)
      else next.set(path, { path, kind: entry.kind })
    }
    if (entries.length > inLibrary.length) this.sourceDraft = [...next.values()]
    return { inLibrary }
  }

  removeSource(path: string) {
    this.sourceDraft = this.sources.filter(source => source.path !== path)
  }

  async addSourcePaths(paths: string[], videosOnly = false) {
    if (!this.canAcceptDrop) return
    let classified
    this.readingSources++
    try {
      classified = await api.classifyPaths(paths)
    } catch (e) {
      app.toast('error', `无法读取所选路径：${e}`, 5000)
      return
    } finally {
      this.readingSources--
    }
    // 归类期间可能已开始扫描,此时不再改动列表
    if (!this.canAcceptDrop) return
    const accepted = (p: DroppedPath): p is ImportSource => p.kind === 'video' || (!videosOnly && p.kind === 'dir')
    const { inLibrary } = this.addSources(classified.filter(accepted))
    const rejected = classified.filter(p => !accepted(p)).map(p => baseName(p.path))
    const reasons = [
      rejected.length ? `${listNames(rejected)} ${videosOnly ? '不是可读取的视频文件' : '不是文件夹或视频'}` : '',
      inLibrary.length ? `${listNames(inLibrary.map(baseName))} 在媒体库里面，库内的影片已经整理过了` : '',
    ].filter(Boolean)
    if (reasons.length) app.toast('error', `未添加：${reasons.join('；')}。`, 6000)
  }

  async pickSources(kind: 'folder' | 'video') {
    try {
      if (kind === 'video') {
        const paths = await api.pickMediaFiles('import')
        if (paths.length && this.canAcceptDrop) await this.addSourcePaths(paths, true)
      } else {
        const dir = await api.pickDirectory('选择待整理的文件夹', app.media?.downloads)
        if (dir && this.canAcceptDrop && this.addSources([{ path: dir, kind: 'dir' }]).inLibrary.length) {
          app.toast('error', '这个文件夹在媒体库里面，请选择库外的文件夹。', 5000)
        }
      }
    } catch (e) {
      app.toast('error', `无法选择待整理来源：${e}`, 5000)
    }
  }

  /** 从「待整理」页直接开始:用已记住的文件夹重新走一遍流程 */
  async startFromSources() {
    if (this.busy) return
    this.reset()
    if (this.sources.length) await this.startScan()
  }

  async startFromFiles(paths: string[]) {
    if (this.busy || !paths.length) return
    this.reset()
    this.sourceDraft = [...new Set(paths)].map(path => ({ path, kind: 'video' }))
    await this.startScan()
  }

  async startScan() {
    if (this.busy || !this.sources.length) return
    const attempt = { cancelled: false, dispatched: false }
    this.scanAttempt = attempt
    this.error = null
    this.items = []
    this.scanSummary = null
    this.setLocalResults([])
    this.plans = []
    this.picked = {}
    this.step = 'scan'
    this.scanRunning = true
    this.expecting = 'scan'
    this.jobId = null
    try {
      const sources = $state.snapshot(this.sources)
      this.sourceDraft = sources
      const roots = sources.map(source => source.path)
      const folders = sources.filter(source => source.kind === 'dir').map(source => source.path)
      await cancelPendingReads()
      if (attempt.cancelled || this.scanAttempt !== attempt) return
      attempt.dispatched = true
      const id = await api.scanStart(roots, folders.length > 0)
      if (attempt.cancelled) await api.jobCancel(id)
      // scanStart 会记住来源,同步门面状态供「待整理」再次开始时使用。
      if (folders.length && app.settings) app.settings = { ...app.settings, source_dirs: folders }
      if (this.expecting === 'scan' && this.scanAttempt === attempt && this.jobId === null) this.jobId = id
    } catch (e) {
      if (this.scanAttempt !== attempt) return
      this.scanRunning = false
      this.finish()
      this.error = String(e)
      this.step = 'source'
    } finally {
      if (this.scanAttempt === attempt) this.scanAttempt = null
    }
  }

  /** 手动指定番号;失败原因放进 codeErrors 供行内展示 */
  codeErrors = $state<Record<number, string>>({})

  async setCode(itemId: number, text: string): Promise<boolean> {
    if (this.busy) return false
    this.correctingCode = true
    try {
      const { localImports, ...updated } = await api.setItemCode(itemId, text)
      const i = this.items.findIndex((x) => x.id === itemId)
      if (i >= 0) this.items[i] = updated
      this.setLocalResults(localImports ?? [])
      this.picked = {}
      delete this.codeErrors[itemId]
      return true
    } catch (e) {
      this.codeErrors[itemId] = String(e)
      return false
    } finally {
      this.correctingCode = false
    }
  }

  private setLocalResults(entries: LocalImportEntry[]) {
    this.localImports = entries
    this.found = entries.filter(entry => entry.reusable).map(({ code, preview }) => ({ code, preview }))
    this.statuses = Object.fromEntries(this.found.map(({ code }) => [code, { kind: 'found' as const }]))
    this.notes = {}
    this.scrapeSummary = null
  }

  cachedSourcesOnly = $state(false)

  async prepareImport() {
    if (this.busy) return
    if (!this.retryCodes.length) return this.buildPlan('scan')
    return this.startScrape(this.localReady.length > 0)
  }

  async startScrape(advance = false) {
    if (this.busy) return
    const origin = this.step
    this.error = null
    const codes = this.retryCodes
    const ids = this.recognized.filter((i) => codes.includes(i.code!.id)).map((i) => i.id)
    if (!ids.length) return
    for (const code of codes) { this.statuses[code] = { kind: 'pending' }; delete this.notes[code] }
    this.scrape = { done: 0, total: codes.length, running: true }
    this.scrapeSummary = null
    this.eta = null
    this.autoPlan = advance
    this.step = 'scrape'
    await this.startTask('scrape', () => api.scrapeStart(ids, this.cachedSourcesOnly), () => {
      this.scrape.running = false
      this.autoPlan = false
      this.step = origin
    })
  }

  /** 用某站详情页重刮一部影片(搜索没找到或找错时);结果走同样的刮削事件 */
  async scrapeUrl(code: string, url: string) {
    if (!url.trim()) return
    await this.refreshFilms([code], ids => api.scrapeUrlStart(ids, url.trim()))
  }

  async rescrape(code?: string) {
    await this.refreshFilms(code ? [code] : this.localReady.map(entry => entry.code), ids => api.scrapeStart(ids, false, true), this.step === 'scan')
  }

  private async refreshFilms(codes: string[], start: (ids: number[]) => Promise<number>, stayAtScan = false) {
    if (this.busy) return
    const wanted = new Set(codes)
    const ids = this.recognized.filter((i) => wanted.has(i.code!.id)).map((i) => i.id)
    if (!ids.length) return
    const previous = codes.map(code => ({ code, status: this.statuses[code], note: this.notes[code] }))
    this.error = null
    for (const code of codes) { this.statuses[code] = { kind: 'pending' }; delete this.notes[code] }
    this.scrape = { done: 0, total: codes.length, running: true }
    this.scrapeSummary = null
    this.eta = null
    this.autoPlan = false
    if (!stayAtScan) this.step = 'scrape'
    await this.startTask('scrape', () => start(ids), () => {
      this.scrape.running = false
      for (const { code, status, note } of previous) {
        if (status) this.statuses[code] = status
        else delete this.statuses[code]
        if (note) this.notes[code] = note
      }
    })
  }

  async updateRules(rules: ImportRules, saveDefault: boolean) {
    if (!app.settings || this.busy) return false
    this.error = null
    try {
      projectImportRules(app.settings, rules)
      if (saveDefault) {
        this.savingRules = true
        await api.saveImportDefaults(rules)
        await app.refreshSettings()
        app.toast('ok', '已保存为下次导入的默认规则')
      }
      this.rules = { ...rules }
    } catch (error) {
      this.error = String(error)
      return false
    } finally { this.savingRules = false }
    if (this.step === 'plan') await this.buildPlan()
    return !this.error
  }

  async renameItem(itemId: number, suffix: string) {
    if (this.busy) return
    try { checkImportName(suffix) } catch (error) { this.error = String(error); return }
    this.renames[itemId] = suffix
    this.skipped[itemId] = false
    await this.buildPlan()
  }

  async buildPlan(origin: 'scan' | 'scrape' = this.step === 'plan' ? this.planOrigin : this.step === 'scan' ? 'scan' : 'scrape') {
    if (this.busy || !this.foundIds.length) return
    this.planToken = undefined
    this.planOrigin = origin
    this.planning = true
    this.error = null
    const attempt = { requestId: crypto.randomUUID(), cancelled: false, jobId: null as number | null }
    this.planningAttempt = attempt
    this.expecting = 'plan'
    this.jobId = null
    let request: Promise<PlanItem[]> | undefined
    let settled = false
    const finish = () => {
      if (this.planningAttempt !== attempt) return
      this.planningAttempt = null
      this.planning = false
      this.finish()
    }
    try {
      request = api.planBuild([...this.foundIds], attempt.requestId, this.rules ? $state.snapshot(this.rules) : undefined, $state.snapshot(this.renames))
      void request.then(() => { settled = true }, () => { settled = true })
      const plans = await withTimeout(request, PLAN_TIMEOUT_MS, '生成整理计划超时,请重试;多次出现请打开「设置 → 日志」查看原因')
      if (attempt.cancelled) throw '计划已取消'
      this.planToken = attempt.requestId
      const reviewedVersions=new Map(this.plans.filter(p=>p.newVersion).map(p=>[p.itemId,p.to]))
      this.picked=Object.fromEntries(plans.map(p=>{
        const needsVersionReview=p.newVersion && reviewedVersions.get(p.itemId)!==p.to
        const selectable=!p.conflict && p.actions?.length!==0 && !needsVersionReview
        return [p.itemId, selectable && (this.picked[p.itemId]??p.selected)]
      }))
      this.plans = plans
      this.step = 'plan'
    } catch (e) {
      this.error = String(e)
    } finally {
      if (request && !settled) {
        attempt.cancelled = true
        if (attempt.jobId !== null) void api.jobCancel(attempt.jobId).catch(error => { this.error = String(error) })
        void request.then(finish, finish)
      } else finish()
    }
  }

  backFromPlan() { this.step = this.planOrigin }

  setAll(on: boolean) {
    for (const p of this.plans) if (!p.conflict && p.actions?.length !== 0) this.picked[p.itemId] = on
  }

  async startApply() {
    this.error = null
    if (this.busy || !this.planToken) return
    this.applyMode = this.effectiveRules?.mode ?? 'move'
    this.applyRoot = app.settings?.library_root ?? null
    this.batchRollback = null
    const ids = this.plans.filter((p) => this.picked[p.itemId] && !p.conflict && p.actions?.length !== 0).map((p) => p.itemId)
    this.applyLog = []
    this.applyProgress = null
    this.logLines = []
    this.applyDone = null
    this.apply = { done: 0, total: ids.length, running: true }
    this.step = 'apply'
    await this.startTask('apply', () => api.applyStart(ids, this.planToken), () => {
      this.apply.running = false
      this.step = 'plan'
    })
  }

  async undoJournal(root: string | null, id: string) {
    if (!root || root !== app.settings?.library_root || this.undoing) return
    const isBatch = this.applyRoot === root && this.applyDone?.journalId === id
    if (isBatch && this.batchUndone) return
    this.undoing = { root, id }
    const version = app.libraryVersion
    try {
      const result = await api.rollback(id, root)
      const complete = result.status === 'rolledBack' && !result.failures.length
      if (isBatch) this.batchRollback = result
      if (root !== app.settings?.library_root) return
      const issues = result.failures.length > 1
        ? [`共 ${result.failures.length} 项未撤销。首项：${result.failures[0]}`]
        : result.failures.slice(0, 1)
      if (!complete && !issues.length) issues.push('尚有操作未撤销，可检查日志后重试')
      if (result.indexError) issues.push('索引更新失败：' + result.indexError)
      const detail = issues.join('；')
      if (issues.length) app.toast('error', (complete ? '文件已撤销。' : '撤销未完成。') + (detail.length > 180 ? detail.slice(0, 179) + '…' : detail), 6000)
      else app.toast('ok', '已撤销，文件已按操作日志恢复', 5000)
      if (app.libraryVersion === version) app.libraryVersion++
      void app.recovery.refresh()
    } catch (error) {
      if (root === app.settings?.library_root) app.toast('error', String(error), 5000)
    } finally {
      this.undoing = null
    }
  }

  async cancel() {
    if (this.taskAttempt) {
      this.taskAttempt.cancelled = true
      await this.cancelTask(this.taskAttempt)
      return
    }
    if (this.planningAttempt) {
      this.planningAttempt.cancelled = true
      if (this.planningAttempt.jobId !== null) await api.jobCancel(this.planningAttempt.jobId)
      return
    }
    if (this.scanAttempt) {
      this.scanAttempt.cancelled = true
      if (!this.scanAttempt.dispatched) {
        this.scanRunning = false
        this.scanSummary = { jobId: 0, total: 0, codes: 0, unrecognized: 0, skipped: [], skippedTotal: 0,
          errors: [], errorsTotal: 0, complete: false, cancelled: true }
        this.finish()
      } else if (this.jobId !== null) {
        await api.jobCancel(this.jobId)
      }
      return
    }
    if (this.expecting !== null && this.jobId !== null) await api.jobCancel(this.jobId)
  }
}

/** 提示里最多列出 3 个名字 */
function listNames(names: string[]): string {
  return names.length > 3 ? `${names.slice(0, 3).join('、')} 等 ${names.length} 项` : names.join('、')
}

export const flow = new ImportFlow()

// 开发时:本模块持有全局单例状态,热替换会造出第二份状态与旧的事件订阅并存(页面卡住、点击无效)。
// 改动它或它依赖的模块时整页刷新。
import.meta.hot?.accept(() => location.reload())
