// 导入流程状态:选目录 → 扫描 → 刮削 → 计划 → 执行。
// 放在模块级单例里,切到别的页面后任务仍在后台跑,回来能看到最新进度。

import { api } from './api'
import { listen } from './events'
import { app } from './state.svelte'
import { cancelPendingReads } from './pending'
import { baseName } from './format'
import { isVideoPath } from './types'
import { applyNotice, notifyFinish, scrapeNotice } from './notify'
import type { ApplyDone, ApplyEvent, ApplyProgress, LogLineEvent, OrganizeMode, PlanItem, Preview, ScanDone, ScanItem, ScrapeDone, ScrapeStatus } from './types'

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

class ImportFlow {
  step = $state<Step>('source')
  dirs = $state<string[]>([])
  error = $state<string | null>(null)

  items = $state<ScanItem[]>([])
  scanRunning = $state(false)
  scanSummary = $state<ScanDone | null>(null)

  statuses = $state<Record<string, ScrapeStatus>>({})
  /** 已找到的结果摘要,按找到的先后排列 */
  found = $state<{ code: string; preview: Preview }[]>([])
  /** 附加提示(如 AI 汉化失败),按番号 */
  notes = $state<Record<string, string>>({})
  scrape = $state({ done: 0, total: 0, running: false })
  scrapeSummary = $state<ScrapeDone | null>(null)
  eta = $state<number | null>(null)

  planning = $state(false)
  plans = $state<PlanItem[]>([])
  picked = $state<Record<number, boolean>>({})

  apply = $state({ done: 0, total: 0, running: false })
  applyProgress = $state<ApplyProgress | null>(null)
  /** 本批执行时的文件处理方式，离开页面或修改设置后仍显示实际操作。 */
  applyMode = $state<OrganizeMode>('move')
  applyLog = $state<ApplyEvent[]>([])
  /** 整理任务的运行日志(log:line),最多保留 LOG_MAX 行 */
  logLines = $state<LogLineEvent[]>([])
  applyDone = $state<ApplyDone | null>(null)

  recognized = $derived(this.items.filter((i) => i.code))
  unrecognized = $derived(this.items.filter((i) => !i.code))
  /** 去重后的番号,保持扫描顺序 */
  uniqueCodes = $derived([...new Set(this.recognized.map((i) => i.code!.id))])
  retryCodes = $derived(this.uniqueCodes.filter((code) => this.statuses[code]?.kind !== 'found'))
  foundIds = $derived(this.recognized.filter((i) => this.statuses[i.code!.id]?.kind === 'found').map((i) => i.id))
  selectedCount = $derived(this.plans.filter((p) => this.picked[p.itemId] && !p.conflict).length)
  conflictCount = $derived(this.plans.filter((p) => p.conflict).length)
  busy = $derived(this.scanRunning || this.scrape.running || this.apply.running || this.planning)
  /** 只在选择来源步骤且没有任务时接收拖入,避免打断进行中的一批 */
  canAcceptDrop = $derived(!this.busy && this.step === 'source')

  private expecting: Kind | null = null
  private jobId: number | null = null
  private started = false
  private retiredJobId = 0
  private scanAttempt: { cancelled: boolean; dispatched: boolean } | null = null
  private planningAttempt: { requestId: string; cancelled: boolean; jobId: number | null } | null = null

  /** 事件可能先于 invoke 的返回值到达,所以第一个事件带来的 jobId 视为当前任务 */
  private accept(kind: Kind, jobId: number): boolean {
    if (this.expecting !== kind || jobId <= this.retiredJobId) return false
    if (this.jobId === null) this.jobId = jobId
    return this.jobId === jobId
  }

  private finish() {
    this.retiredJobId = Math.max(this.retiredJobId, this.jobId ?? 0)
    this.expecting = null
    this.jobId = null
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
        this.scrape.running = false
        this.scrapeSummary = e
        this.eta = null
        this.finish()
        notifyFinish(scrapeNotice(e))
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
        // jobId 0 是不属于任何任务的日志(如媒体库页的后台检查);不能让它抢先认领整理任务的编号
        if (e.jobId === 0 || !this.accept('apply', e.jobId)) return
        this.logLines.push(e)
        if (this.logLines.length > LOG_MAX) this.logLines.splice(0, this.logLines.length - LOG_MAX)
      }))
      unlisten.push(await listen('apply:done', (e) => {
        if (!this.accept('apply', e.jobId)) return
        this.apply.running = false
        this.applyDone = e
        this.step = 'done'
        this.finish()
        notifyFinish(applyNotice(e))
        app.libraryVersion++
      }))
      this.started = true
    } catch (e) {
      for (const stop of unlisten) stop()
      throw e
    }
  }

  reset() {
    if (this.busy) return
    this.step = 'source'
    this.items = []
    this.scanSummary = null
    this.statuses = {}
    this.found = []
    this.notes = {}
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
    this.error = null
  }

  /**
   * 把文件夹并入待整理列表,末尾分隔符归一后去重。
   * 待整理目录不能在媒体库里面:库内的文件扫描时会被跳过,选了只会得到空结果。
   */
  private addDirs(dirs: string[]): { inLibrary: string[] } {
    const lib = app.settings?.library_root
    const norm = (p: string) => p.replace(/(?<=[^:\\/])[\\/]+$/, '')
    const inLibrary: string[] = []
    for (const raw of dirs) {
      const dir = norm(raw)
      if (lib && (dir === norm(lib) || dir.startsWith(norm(lib) + '/') || dir.startsWith(norm(lib) + '\\'))) inLibrary.push(dir)
      else if (!this.dirs.includes(dir)) this.dirs.push(dir)
    }
    return { inLibrary }
  }

  removeDir(dir: string) {
    this.dirs = this.dirs.filter((d) => d !== dir)
  }

  /** 选择来源步骤的列表为空时,先放入已记住的待整理文件夹 */
  seedDirs() {
    if (this.dirs.length === 0 && this.step === 'source') this.dirs = [...(app.settings?.source_dirs ?? [])]
  }

  /** 拖入窗口的路径:文件夹和单个视频文件并入待整理列表 */
  async dropPaths(paths: string[]) {
    const classified = await api.classifyPaths(paths)
    // 归类期间可能已开始扫描,此时不再改动列表
    if (!this.canAcceptDrop) return
    this.seedDirs()
    const accepted = (k: string) => k === 'dir' || k === 'video'
    const { inLibrary } = this.addDirs(classified.filter((p) => accepted(p.kind)).map((p) => p.path))
    const rejected = classified.filter((p) => !accepted(p.kind)).map((p) => baseName(p.path))
    const reasons = [
      rejected.length ? `${listNames(rejected)} 不是文件夹或视频，请拖入文件夹或视频文件` : '',
      inLibrary.length ? `${listNames(inLibrary.map(baseName))} 在媒体库里面，库内的影片已经整理过了` : '',
    ].filter(Boolean)
    if (reasons.length) app.toast('error', `未添加：${reasons.join('；')}。`, 6000)
  }

  async pickDir() {
    const d = await api.pickDirectory('选择待整理的文件夹', app.media?.downloads)
    if (d && this.addDirs([d]).inLibrary.length) {
      app.toast('error', '这个文件夹在媒体库里面。请选择媒体库之外的文件夹(库内的影片已经整理过了)。', 5000)
    }
  }

  /** 从「待整理」页直接开始:用已记住的文件夹重新走一遍流程 */
  async startFromSources() {
    if (this.busy) return
    this.reset()
    this.dirs = [...(app.settings?.source_dirs ?? [])]
    if (this.dirs.length) await this.startScan()
  }

  async startScan() {
    const attempt = { cancelled: false, dispatched: false }
    this.scanAttempt = attempt
    this.error = null
    this.items = []
    this.scanSummary = null
    this.statuses = {}
    this.plans = []
    this.step = 'scan'
    this.scanRunning = true
    this.expecting = 'scan'
    this.jobId = null
    try {
      const dirs = $state.snapshot(this.dirs)
      await cancelPendingReads()
      if (attempt.cancelled || this.scanAttempt !== attempt) return
      attempt.dispatched = true
      const id = await api.scanStart(dirs)
      if (attempt.cancelled) await api.jobCancel(id)
      // scanStart 会记住来源,同步门面状态供「待整理」再次开始时使用。
      if (app.settings) app.settings = { ...app.settings, source_dirs: dirs.filter((d) => !isVideoPath(d)) }
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
    try {
      const updated = await api.setItemCode(itemId, text)
      const i = this.items.findIndex((x) => x.id === itemId)
      if (i >= 0) this.items[i] = updated
      delete this.codeErrors[itemId]
      return true
    } catch (e) {
      this.codeErrors[itemId] = String(e)
      return false
    }
  }

  async startScrape(retry = false) {
    if (this.busy) return
    retry ||= Object.keys(this.statuses).length > 0
    this.error = null
    const codes = retry ? this.retryCodes : this.uniqueCodes
    const ids = this.recognized.filter((i) => codes.includes(i.code!.id)).map((i) => i.id)
    if (!ids.length) { this.step = 'scrape'; return }
    if (!retry) { this.statuses = {}; this.found = []; this.notes = {} }
    for (const code of codes) { this.statuses[code] = { kind: 'pending' }; delete this.notes[code] }
    this.scrape = { done: 0, total: codes.length, running: true }
    this.scrapeSummary = null
    this.eta = null
    this.step = 'scrape'
    this.expecting = 'scrape'
    this.jobId = null
    try {
      const id = await api.scrapeStart(ids)
      if (this.jobId === null) this.jobId = id
    } catch (e) {
      this.scrape.running = false
      this.finish()
      this.error = String(e)
      this.step = retry ? 'scrape' : 'scan'
    }
  }

  /** 用某站详情页重刮一部影片(搜索没找到或找错时);结果走同样的刮削事件 */
  async scrapeUrl(code: string, url: string) {
    if (this.busy || !url.trim()) return
    const ids = this.recognized.filter((i) => i.code!.id === code).map((i) => i.id)
    if (!ids.length) return
    this.error = null
    this.statuses[code] = { kind: 'pending' }
    delete this.notes[code]
    this.scrape = { done: 0, total: 1, running: true }
    this.scrapeSummary = null
    this.eta = null
    this.step = 'scrape'
    this.expecting = 'scrape'
    this.jobId = null
    try {
      const id = await api.scrapeUrlStart(ids, url.trim())
      if (this.jobId === null) this.jobId = id
    } catch (e) {
      this.scrape.running = false
      this.finish()
      this.error = String(e)
    }
  }

  async buildPlan() {
    if (this.busy) return
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
      request = api.planBuild([...this.foundIds], attempt.requestId)
      void request.then(() => { settled = true }, () => { settled = true })
      const plans = await withTimeout(request, PLAN_TIMEOUT_MS, '生成整理计划超时,请重试;多次出现请打开「设置 → 日志」查看原因')
      if (attempt.cancelled) throw '计划已取消'
      this.plans = plans
      this.picked = Object.fromEntries(this.plans.map((p) => [p.itemId, !p.conflict && (this.picked[p.itemId] ?? p.selected)]))
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

  setAll(on: boolean) {
    for (const p of this.plans) if (!p.conflict) this.picked[p.itemId] = on
  }

  async startApply() {
    this.error = null
    this.applyMode = app.settings?.organize_mode ?? 'move'
    const ids = this.plans.filter((p) => this.picked[p.itemId] && !p.conflict).map((p) => p.itemId)
    this.applyLog = []
    this.applyProgress = null
    this.logLines = []
    this.applyDone = null
    this.apply = { done: 0, total: ids.length, running: true }
    this.step = 'apply'
    this.expecting = 'apply'
    this.jobId = null
    try {
      const id = await api.applyStart(ids)
      if (this.jobId === null) this.jobId = id
    } catch (e) {
      this.apply.running = false
      this.finish()
      this.error = String(e)
      this.step = 'plan'
    }
  }

  async cancel() {
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
    await api.jobCancel(this.jobId ?? undefined)
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
