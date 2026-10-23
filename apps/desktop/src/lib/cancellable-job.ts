import { api } from './api'
import { listen } from './events'

export interface CancellableJob<T> {
  readonly jobId: number | null
  readonly cancelled: boolean
  promise: Promise<T>
  cancel(): Promise<void>
}

/** A long invoke returns its result, while job:started provides its cancellation owner. */
export function cancellableJob<T>(
  run: (requestId: string) => Promise<T>,
  onCancelError: (error: unknown) => void,
): CancellableJob<T> {
  const requestId = crypto.randomUUID()
  let jobId: number | null = null
  let cancelled = false
  let cancelSent = false
  let settled = false

  async function cancel() {
    if (settled) return
    cancelled = true
    if (jobId === null || cancelSent) return
    cancelSent = true
    try { await api.jobCancel(jobId) }
    catch (error) {
      if (settled) return
      cancelSent = false
      onCancelError(error)
    }
  }

  const promise = (async () => {
    const unlisten = await listen('job:started', event => {
      if (settled || event.requestId !== requestId || jobId !== null) return
      jobId = event.jobId
      if (cancelled) void cancel()
    })
    try {
      if (cancelled) throw new Error('任务已取消')
      return await run(requestId)
    } finally {
      settled = true
      unlisten()
    }
  })()
  return { promise, cancel, get jobId() { return jobId }, get cancelled() { return cancelled } }
}
