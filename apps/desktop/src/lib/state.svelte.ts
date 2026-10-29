import {RecoveryStore} from './recovery.svelte'
import { api } from './api'
import { listen } from './events'
import { notifyFinish, rebuildNotice } from './notify'
import type { MediaDirs, ProgressEvent, RebuiltEvent, Settings } from './types'

export interface ConfirmOptions {
  title: string
  body: string
  confirmLabel?: string
  cancelLabel?: string
}

export type View = 'library' | 'import' | 'history' | 'settings'

export interface Toast {
  id: number
  kind: 'info' | 'ok' | 'error'
  text: string
  action?: { label: string; run: () => void }
}

interface RebuildTask {
  root: string
  jobId: number | null
  early: Map<number, { progress?: ProgressEvent; complete?: RebuiltEvent }>
}

class AppStore {
  recovery=new RecoveryStore(()=>this.settings?.library_root??null)
  view = $state<View>('library')
  commandsOpen = $state(false)
  settings = $state<Settings | null>(null)
  ready = $state(false)
  /** 媒体库有变化时 +1,列表据此重新加载 */
  libraryVersion = $state(0)
  toasts = $state<Toast[]>([])
  rebuild = $state({ running: false, done: 0, total: 0 })
  /** 当前显示的确认框(同一时间只有一个) */
  dialog = $state<(ConfirmOptions & { resolve: (ok: boolean) => void }) | null>(null)

  /** 系统影片/下载目录,用作默认位置与对话框起点 */
  media = $state<MediaDirs | null>(null)

  needsOnboarding = $derived(this.ready && !this.settings?.library_root)

  private nextToast = 1
  /**
   * 离开当前页前的检查(设置页的未保存提醒):返回 true 直接离开、false 留下;
   * 需要用户决定时返回 Promise(此时必须有确认框在显示)。
   */
  beforeNavigate: (() => boolean | Promise<boolean>) | null = null
  /** 正在等离开确认时用户又点了别的页:只记住最后一次点击,确认后去那里 */
  private pendingView: View | null = null
  private rebuildTask: RebuildTask | null = null

  async init() {
    if (this.ready) return
    const unlisten: (() => void)[] = []
    try {
      this.settings = await api.getSettings()
      this.media = await api.mediaDirs().catch(() => null)
      unlisten.push(await listen('library:changed', (e) => {
        if (e.root !== undefined && e.root !== this.settings?.library_root) return
        this.libraryVersion++
      }))
      unlisten.push(await listen('library:progress', (e) => this.onRebuildEvent(e)))
      for (const event of ['plex:refreshed', 'server:refreshed'] as const) unlisten.push(await listen(event, (e) => { if (e.root === this.settings?.library_root) this.toast(e.ok ? 'ok' : 'error', e.message, e.ok ? 3200 : 6000) }))
      unlisten.push(await listen('library:rebuilt', (e) => this.onRebuildEvent(e)))
      this.ready = true
      void this.recovery.refresh()
    } catch (e) {
      unlisten.forEach((off) => off())
      throw e
    }
  }

  /** A zero duration keeps an actionable notice until dismissed or resolved. */
  toast(kind: Toast['kind'], text: string, ms = 3200, action?: Toast['action']): number {
    const id = this.nextToast++
    this.toasts = [...this.toasts, { id, kind, text, action }]
    if (ms > 0) setTimeout(() => this.dismiss(id), ms)
    return id
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id)
  }

  /** 应用内确认框;用户点确认返回 true */
  confirm(opts: ConfirmOptions): Promise<boolean> {
    if (this.dialog) return Promise.resolve(false)
    return new Promise((resolve) => {
      this.dialog = {
        ...opts,
        resolve: (ok) => {
          this.dialog = null
          resolve(ok)
        },
      }
    })
  }

  /**
   * 切换页面。点击永远不会被吞掉:无需确认时同步切换;等待确认期间的点击更新目标(以最后一次为准);
   * 等待中却没有确认框在显示(异常情况)时不再干等,直接放行这次点击。
   */
  navigate(v: View) {
    if (this.pendingView !== null) {
      if (this.dialog) {
        this.pendingView = v
        return
      }
      this.pendingView = null
    }
    if (this.view === v) return
    const verdict = this.beforeNavigate ? this.beforeNavigate() : true
    if (typeof verdict === 'boolean') {
      if (verdict) this.view = v
      return
    }
    this.pendingView = v
    verdict
      .then((ok) => {
        if (ok && this.pendingView !== null) this.view = this.pendingView
      })
      .catch(() => {})
      .finally(() => (this.pendingView = null))
  }

  async refreshSettings() {
    this.settings = await api.getSettings()
    await this.recovery.refresh()
  }

  async setLibraryRoot(path: string, create = false): Promise<boolean> {
    try {
      this.settings = await api.setLibraryRoot(path, create)
      this.libraryVersion++
      await this.recovery.refresh()
      return true
    } catch (e) {
      this.toast('error', String(e), 5000)
      return false
    }
  }

  /**
   * 选择/更换媒体库文件夹的统一流程:选目录 → 体检 → 必要时提醒 → 设置 →(已有影片时)重建索引。
   * 返回是否真的换了。
   */
  async chooseLibraryRoot(): Promise<boolean> {
    // 对话框从系统影片目录打开
    const dir = await api.pickDirectory('选择媒体库文件夹', this.media?.videos)
    return dir ? this.adoptLibraryRoot(dir, false) : false
  }

  /** 直接使用默认位置(影片目录下的 kiroku),不存在就创建 */
  async useDefaultLibraryRoot(): Promise<boolean> {
    const dir = this.media?.suggestedLibrary
    if (!dir) {
      this.toast('error', '没有找到系统的影片目录,请手动选择文件夹', 5000)
      return false
    }
    return this.adoptLibraryRoot(dir, true)
  }

  /** 体检 → 必要时提醒 → 设置 →(已有影片时)重建索引 */
  private async adoptLibraryRoot(dir: string, create: boolean): Promise<boolean> {
    const info = await api.inspectDirectory(dir).catch(() => null)
    if (info && info.videos > 0 && info.nfos === 0) {
      const ok = await this.confirm({
        title: '这个文件夹里有还没整理的视频',
        body: `里面有 ${info.videos}${info.truncated ? '+' : ''} 个视频,但没有任何 NFO。媒体库只显示 kiroku 整理过(或已带 NFO)的影片,这些视频不会出现在海报墙,而且「导入整理」也不能再把它们当作待整理的来源。\n\n建议另选一个专门放整理结果的空文件夹。`,
        confirmLabel: '仍然使用它',
        cancelLabel: '重新选择',
      })
      if (!ok) return false
    }
    if (!(await this.setLibraryRoot(dir, create))) return false
    if (info && info.nfos > 0) {
      this.toast('info', `检测到已有 ${info.nfos} 部影片的 NFO,正在建立索引…`, 4500)
      await this.rebuildIndex()
    }
    return true
  }

  /** 把用户目录显示成 ~ 开头,更短更好认 */
  prettyPath(p: string | null | undefined): string {
    if (!p) return ''
    const home = this.media?.videos?.replace(/[\\/][^\\/]+$/, '')
    return home && p.startsWith(home) ? '~' + p.slice(home.length) : p
  }

  private onRebuildEvent(e: ProgressEvent | RebuiltEvent) {
    const task = this.rebuildTask
    if (!task || e.root !== task.root) return
    if (task.jobId === null) {
      // A start response owns the task. Events can arrive first but cannot claim it.
      const early = task.early.get(e.jobId) ?? {}
      if ('count' in e) early.complete ??= e
      else early.progress = e
      task.early.set(e.jobId, early)
      return
    }
    if (e.jobId !== task.jobId) return
    if (!('count' in e)) {
      this.rebuild = { running: true, done: e.done, total: e.total }
      return
    }
    this.rebuildTask = null
    this.rebuild = { running: false, done: 0, total: 0 }
    if (task.root !== this.settings?.library_root) return
    this.toast(e.error ? 'error' : 'ok', e.error ? `索引重建失败：${e.error}` : e.cancelled ? '已取消重建，保留原索引' : `索引已重建:${e.count} 部影片`)
    notifyFinish(rebuildNotice(e))
    this.libraryVersion++
  }

  async rebuildIndex() {
    if (this.rebuildTask) return
    const task: RebuildTask = { root: this.settings?.library_root ?? '', jobId: null, early: new Map() }
    this.rebuildTask = task
    this.rebuild = { running: true, done: 0, total: 0 }
    try {
      const id = await api.libraryRebuild()
      if (this.rebuildTask !== task) return
      task.jobId = id
      const early = task.early.get(id)
      task.early.clear()
      const event = early?.complete ?? early?.progress
      if (event) this.onRebuildEvent(event)
    } catch (e) {
      if (this.rebuildTask !== task) return
      this.rebuildTask = null
      this.rebuild = { running: false, done: 0, total: 0 }
      if (task.root === this.settings?.library_root) this.toast('error', String(e), 5000)
    }
  }
}

export const app = new AppStore()

// 开发时:本模块持有全局单例状态,热替换会造出第二份状态与旧的事件订阅并存(页面卡住、点击无效)。
// 改动它或它依赖的模块时整页刷新。
import.meta.hot?.accept(() => location.reload())
