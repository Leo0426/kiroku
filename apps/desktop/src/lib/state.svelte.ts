import {RecoveryStore} from './recovery.svelte'
import { api } from './api'
import { listen } from './events'
import { notifyFinish, rebuildNotice } from './notify'
import type { SettingLink } from './settings-search'
import type { MediaDirs, ProgressEvent, RebuiltEvent, Settings } from './types'

export interface ConfirmOptions {
  title: string
  body: string
  confirmLabel?: string
  cancelLabel?: string
}

type DialogChoice = 'confirm' | 'alternative' | 'dismiss'

export type View = 'library' | 'import' | 'history' | 'settings'

export interface Toast {
  id: number
  kind: 'info' | 'ok' | 'error'
  text: string
  action?: { label: string; run: () => void }
  title?: string
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
  settingsLink = $state.raw<SettingLink | null>(null)
  settings = $state<Settings | null>(null)
  ready = $state(false)
  /** 媒体库有变化时 +1,列表据此重新加载 */
  libraryVersion = $state(0)
  toasts = $state<Toast[]>([])
  rebuild = $state({ running: false, done: 0, total: 0 })
  private rebuildIssue = $state<{ root: string; message: string } | null>(null)
  rebuildMessage = $derived(this.rebuildIssue && this.rebuildIssue.root === this.settings?.library_root ? this.rebuildIssue.message : '')
  /** 当前显示的确认框(同一时间只有一个) */
  dialog = $state<(ConfirmOptions & { resolve: (choice: DialogChoice) => void }) | null>(null)

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
  private pendingNavigation: { view: View; setting: SettingLink | null } | null = null
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
  toast(kind: Toast['kind'], text: string, ms = 3200, action?: Toast['action'], title?: string): number {
    const id = this.nextToast++
    this.toasts = [...this.toasts, { id, kind, text, action, title }]
    if (ms > 0) setTimeout(() => this.dismiss(id), ms)
    return id
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id)
  }

  /** 应用内确认框;用户点确认返回 true */
  async confirm(opts: ConfirmOptions): Promise<boolean> {
    return await this.dialogChoice(opts) === 'confirm'
  }

  private dialogChoice(opts: ConfirmOptions): Promise<DialogChoice> {
    if (this.dialog) return Promise.resolve('dismiss')
    return new Promise((resolve) => {
      this.dialog = {
        ...opts,
        resolve: (choice) => {
          this.dialog = null
          resolve(choice)
        },
      }
    })
  }

  /**
   * 切换页面。点击永远不会被吞掉:无需确认时同步切换;等待确认期间的点击更新目标(以最后一次为准);
   * 等待中却没有确认框在显示(异常情况)时不再干等,直接放行这次点击。
   */
  navigate(v: View, setting: SettingLink | null = null) {
    if (this.pendingNavigation) {
      if (this.dialog) {
        this.pendingNavigation.view = v
        this.pendingNavigation.setting = setting
        return
      }
      this.pendingNavigation = null
    }
    if (this.view === v) {
      if (setting) this.settingsLink = setting
      return
    }
    const verdict = this.beforeNavigate ? this.beforeNavigate() : true
    if (typeof verdict === 'boolean') {
      if (verdict) {
        this.settingsLink = setting
        this.view = v
      }
      return
    }
    const target = { view: v, setting }
    this.pendingNavigation = target
    verdict
      .then((ok) => {
        if (ok && this.pendingNavigation === target) {
          this.settingsLink = target.setting
          this.view = target.view
        }
      })
      .catch(() => {})
      .finally(() => { if (this.pendingNavigation === target) this.pendingNavigation = null })
  }

  async refreshSettings() {
    this.settings = await api.getSettings()
    void this.recovery.refresh()
  }

  async setLibraryRoot(path: string, create = false, rebuild = false): Promise<boolean> {
    try {
      this.settings = await api.setLibraryRoot(path, create)
      this.rebuildIssue = null
      this.libraryVersion++
      const indexing = rebuild ? this.rebuildIndex() : undefined
      await Promise.all([indexing, this.recovery.refresh()])
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
      const choice = await this.dialogChoice({
        title: '这个文件夹里有还没整理的视频',
        body: `已找到 ${info.videos}${info.truncated ? '+' : ''} 个视频，暂未发现 NFO。把这里用作媒体库后，这些视频将无法作为待整理来源导入。\n\n可以另选一个空文件夹存放整理结果，稍后再导入这里的视频。`,
        confirmLabel: '重新选择',
        cancelLabel: '仍然使用它',
      })
      if (choice === 'confirm') return this.chooseLibraryRoot()
      if (choice === 'dismiss') return false
    }
    return this.setLibraryRoot(dir, create, !!info?.nfos)
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
    const issue = e.error ? `索引重建失败：${e.error}` : e.cancelled ? '已取消重建，保留原索引' : ''
    if (issue) this.rebuildIssue = { root: task.root, message: issue }
    this.toast(e.error ? 'error' : e.cancelled ? 'info' : 'ok', issue || `索引已重建:${e.count} 部影片`)
    notifyFinish(rebuildNotice(e))
    this.libraryVersion++
  }

  async rebuildIndex() {
    if (this.rebuildTask) return
    const task: RebuildTask = { root: this.settings?.library_root ?? '', jobId: null, early: new Map() }
    this.rebuildTask = task
    this.rebuildIssue = null
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
      if (task.root === this.settings?.library_root) {
        const message = `无法启动索引重建：${e}`
        this.rebuildIssue = { root: task.root, message }
        this.toast('error', message, 5000)
      }
    }
  }
}

export const app = new AppStore()

// 开发时:本模块持有全局单例状态,热替换会造出第二份状态与旧的事件订阅并存(页面卡住、点击无效)。
// 改动它或它依赖的模块时整页刷新。
import.meta.hot?.accept(() => location.reload())
