import { api } from './api'

interface Request {
  key: string
  code: string
  version: string
  current: () => boolean
  warm: boolean
  resolve: (url: string | null) => void
}

const waiting: Request[] = []
const running = new Map<string, Promise<string | null>>()
const recent = new Map<string, string | null>()
const LIMIT = 2

function drain() {
  while (waiting.length && running.size < LIMIT) {
    const req = waiting.shift()!
    if (!req.current()) { req.resolve(null); continue }
    const known = recent.get(req.key)
    if (known || (req.warm && known === null)) { req.resolve(known); continue }

    let request = running.get(req.key)
    if (!request) {
      request = Promise.resolve().then(() => api.cachedPoster(req.code, req.version)).catch(() => null)
      running.set(req.key, request)
      void request.then(url => {
        if (req.current()) {
          recent.set(req.key, url)
          if (recent.size > 256) recent.delete(recent.keys().next().value!)
        }
        running.delete(req.key)
        drain()
      })
    }
    void request.then(req.resolve)
  }
}

/** Failed sources get priority. Warming a visible native image is best effort. */
export function posterPreview(root: string, code: string, version: string, current: () => boolean, warm = false): Promise<string | null> {
  const key = JSON.stringify([root, code, version])
  const pending = running.get(key)
  if (pending) return pending
  const known = recent.get(key)
  if (known || (warm && known === null)) return Promise.resolve(known)
  for (let i = waiting.length - 1; i >= 0; i--) {
    if (!waiting[i].current()) waiting.splice(i, 1)[0].resolve(null)
  }
  if (warm && (waiting.length >= 32 || waiting.some(r => r.key === key))) return Promise.resolve(null)
  return new Promise(resolve => {
    const request = { key, code, version, current, warm, resolve }
    if (warm) waiting.push(request)
    else waiting.unshift(request)
    drain()
  })
}

export function forgetPosterPreview(root: string, code: string, version: string) {
  recent.delete(JSON.stringify([root, code, version]))
}
