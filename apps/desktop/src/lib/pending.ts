import { api } from './api'

interface PendingRequest {
  started: boolean
  cancelled: boolean
  run: () => Promise<void>
  cancel: () => void
}
const waiting: PendingRequest[] = []
const owners = new Set<PendingRequest>()
let active = false
let completion: Promise<void> = Promise.resolve()

/** Cancel each owner, then wait for the actual dispatched read to release the job slot. */
export async function cancelPendingReads() {
  for (const owner of [...owners]) owner.cancel()
  await completion
}
export function pendingRead<T>(read: (requestId: string) => Promise<T>) {
  const requestId = crypto.randomUUID()
  let request: PendingRequest
  const promise = new Promise<T>((resolve, reject) => {
    request = {
      started: false,
      cancelled: false,
      run: () => Promise.resolve().then(() => read(requestId)).then(resolve, reject),
      cancel: () => {
        if (request.cancelled) return
        request.cancelled = true
        if (request.started) {
          // Native disk I/O keeps its slot until the invoke really returns.
          void api.pendingCancel(requestId).catch(() => {})
        } else {
          const index = waiting.indexOf(request)
          if (index >= 0) waiting.splice(index, 1)
          owners.delete(request)
          reject(new Error('检查已取消'))
        }
      },
    }
    owners.add(request)
    waiting.push(request)
    drain()
  })
  return { promise, cancel: () => request.cancel() }
}
function drain() {
  if (active || !waiting.length) return
  const request = waiting.shift()!
  active = true
  request.started = true
  completion = request.run().finally(() => {
    owners.delete(request)
    active = false
    drain()
  })
}
