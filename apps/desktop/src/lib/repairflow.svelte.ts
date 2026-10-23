import { api } from './api'
import { listen } from './events'
import { app } from './state.svelte'
import type { RepairDone } from './types'

/** App-lifetime task ownership survives moving between health reports and movie details. */
class RepairFlow {
  running = $state(false)
  origin = $state<'health' | 'details'>('details')
  code = $state('')
  root = $state('')
  jobId = $state<number | null>(null)
  result = $state<RepairDone | null>(null)
  error = $state('')
  private ready: Promise<unknown> | null = null
  private buffered: RepairDone[] = []
  private cancelRequested = false
  private cancelSent = false
  private cancelNotice: number | null = null
  private generation = 0

  private init() {
    return this.ready ??= listen('repair:done', e => {
      if (!this.running || e.code !== this.code || e.root !== this.root) return
      if (this.jobId === null) this.buffered.push(e)
      else this.complete(e)
    }).catch(error => {
      this.ready = null
      throw error
    })
  }

  private clearCancelNotice() {
    if (this.cancelNotice !== null) app.dismiss(this.cancelNotice)
    this.cancelNotice = null
  }

  private complete(e: RepairDone) {
    if (!this.running || e.jobId !== this.jobId || e.code !== this.code || e.root !== this.root) return
    this.running = false
    this.result = e
    this.buffered = []
    this.clearCancelNotice()
    if (app.settings?.library_root === e.root) app.libraryVersion++
  }

  async start(code: string, origin: 'health' | 'details' = 'details') {
    if (this.running) throw '已有补图任务正在运行'
    const generation = ++this.generation
    this.origin = origin
    this.code = code
    this.root = app.settings?.library_root ?? ''
    this.result = null
    this.error = ''
    this.running = true
    this.jobId = null
    this.buffered = []
    this.cancelRequested = false
    this.cancelSent = false
    this.clearCancelNotice()
    try {
      await this.init()
      if (this.root !== (app.settings?.library_root ?? '')) {
        this.running = false
        app.toast('info', '媒体库已变化，请重新补图')
        return
      }
      const id = await api.repairImagesStart(code)
      if (generation !== this.generation) return
      this.jobId = id
    } catch (e) {
      if (generation === this.generation) {
        this.running = false
        this.buffered = []
        this.error = String(e)
      }
      return
    }
    // Completion may arrive before the start response; a settled task needs no stop.
    const buffered = this.buffered
    this.buffered = []
    for (const event of buffered) this.complete(event)
    if (this.running && this.cancelRequested) await this.cancel()
  }

  async cancel() {
    if (!this.running) return
    this.cancelRequested = true
    if (this.jobId === null || this.cancelSent) return
    const generation = this.generation
    const jobId = this.jobId
    this.cancelSent = true
    this.clearCancelNotice()
    try {
      await api.jobCancel(jobId)
    } catch (error) {
      if (generation !== this.generation || !this.running) return
      this.cancelSent = false
      this.cancelNotice = app.toast('error', `${this.code}：停止补图失败：${String(error)}`, 0, {
        label: '重试停止',
        run: () => { if (generation === this.generation) void this.cancel() },
      })
    }
  }

  clearResult() {
    if (!this.running) {
      this.result = null
      this.error = ''
    }
  }
}

export const repairFlow = new RepairFlow()
