import { api } from './api'
import type { LibraryOverview, Query } from './types'

interface Request {
  query: Query; signal: AbortSignal
  resolve: (result: LibraryOverview) => void; reject: (error: unknown) => void
  abort: () => void
}
const waiting = new Set<Request>()
let active = false

// An active native aggregate owns its transport slot until it actually settles.
// Cancelled waiting requests settle immediately and retain no queue references.
export function overviewRequest(query: Query, signal: AbortSignal): Promise<LibraryOverview> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(new Error('概览查询已取消')); return }
    const request: Request = { query, signal, resolve, reject, abort: () => {} }
    request.abort = () => {
      waiting.delete(request)
      signal.removeEventListener('abort', request.abort)
      reject(new Error('概览查询已取消'))
    }
    signal.addEventListener('abort', request.abort, { once: true })
    waiting.add(request)
    drain()
  })
}
function drain() {
  if (active) return
  const request = waiting.values().next().value
  if (!request) return
  waiting.delete(request)
  active = true
  Promise.resolve().then(() => {
    if (request.signal.aborted) throw new Error('概览查询已取消')
    return api.libraryOverview(request.query)
  }).then(result => {
    if (!request.signal.aborted) request.resolve(result)
  }, request.reject).finally(() => {
    request.signal.removeEventListener('abort', request.abort)
    active = false
    drain()
  })
}
