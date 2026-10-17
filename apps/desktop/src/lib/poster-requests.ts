import { api } from './api'
import type { Page, Query } from './types'
import { POSTER_REQUEST_LIMIT } from './poster-pages'

interface Request {
  query: Query; signal: AbortSignal; started: boolean
  resolve: (page: Page) => void; reject: (error: unknown) => void; abort: () => void
}
const waiting: Request[] = []
let active = 0

/** Shared across wall lifetimes. Retired invokes keep their transport slot until settled. */
export function posterRequest(query: Query, signal: AbortSignal): Promise<Page> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(new Error('列表请求已取消')); return }
    const request: Request = { query, signal, started: false, resolve, reject, abort: () => {} }
    request.abort = () => {
      reject(new Error('列表请求已取消'))
      if (!request.started) {
        const index = waiting.indexOf(request)
        if (index >= 0) waiting.splice(index, 1)
        signal.removeEventListener('abort', request.abort)
      }
    }
    signal.addEventListener('abort', request.abort, { once: true })
    waiting.push(request)
    drain()
  })
}
function drain() {
  while (active < POSTER_REQUEST_LIMIT && waiting.length) {
    const request = waiting.shift()!
    request.started = true
    active++
    // Defer the call so synchronous connector failures also release the slot.
    Promise.resolve().then(() => {
      if (request.signal.aborted) throw new Error('列表请求已取消')
      return api.libraryList(request.query)
    }).then(page => {
      if (!request.signal.aborted) request.resolve(page)
    }, error => request.reject(error)).finally(() => {
      request.signal.removeEventListener('abort', request.abort)
      active--
      drain()
    })
  }
}
