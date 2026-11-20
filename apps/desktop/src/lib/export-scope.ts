import { api } from './api'
import type { Page, Query } from './types'

export type ExportScope = {kind:'selection'; codes:string[]} | {kind:'filtered'} | {kind:'library'}

/** A slow library read may finish after its owner closed the panel or cancelled. */
export function readScopePage(query:Query, signal?:AbortSignal):Promise<Page> {
  return new Promise((resolve,reject) => {
    signal?.throwIfAborted()
    const abort = () => finish(() => reject(signal?.reason))
    const timer = setTimeout(() => finish(() => reject(new Error('读取影片列表超时，请检查媒体库连接后重试'))), 60_000)
    function finish(callback:()=>void) {
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      callback()
    }
    signal?.addEventListener('abort',abort,{once:true})
    api.libraryList(query).then(page=>finish(()=>resolve(page)),error=>finish(()=>reject(error)))
  })
}

export async function resolveExportCodes(scope:ExportScope, query:Query, signal?:AbortSignal):Promise<string[]> {
  signal?.throwIfAborted()
  if (scope.kind === 'selection') return [...new Set(scope.codes)]
  const filter = scope.kind === 'library' ? {} : query
  const codes = new Set<string>()
  let offset = 0
  let expected: number | undefined
  while (true) {
    const page = await readScopePage({...filter,offset,limit:500},signal)
    signal?.throwIfAborted()
    expected ??= page.total
    const previous = codes.size
    for (const movie of page.items) codes.add(movie.code)
    if (page.total !== expected || codes.size - previous !== page.items.length || codes.size > expected) {
      throw new Error('影片列表已变化，请重新预览')
    }
    offset += page.items.length
    if (offset === expected) return [...codes]
    if (!page.items.length) throw new Error('影片列表未读取完整，请重试')
  }
}
