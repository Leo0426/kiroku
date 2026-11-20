import type { MovieDetail, SupplementEntry, SupplementKind, SupplementReview } from '../types'

const newFiles = new Set<string>()
let pending: { review: SupplementReview; root: string; generation: number } | null = null

export function previewSupplement(movie: MovieDetail, root: string, generation: number, token: string): SupplementReview {
  const entries: SupplementEntry[] = []
  function add(kind: SupplementKind, path: string, existing: boolean, available = true) {
    const status = existing || newFiles.has(path) ? 'existing' : available ? 'missing' : 'unavailable'
    entries.push({ kind, path, status, reason: status === 'existing' ? '保留已有文件' : status === 'missing' ? '可补齐' : '该演员没有可用头像来源' })
  }
  add('nfo', movie.dir + '/' + movie.code + '.nfo', true)
  add('poster', movie.dir + '/poster.jpg', !!movie.poster)
  add('fanart', movie.dir + '/fanart.jpg', !!movie.fanart)
  for (const actor of movie.meta.actors) add('actors', movie.dir + '/.actors/' + actor.name + '.png', false, !!actor.thumb)
  if (!movie.meta.actors.length) entries.push({kind:'actors',path:null,status:'unavailable',reason:'影片资料中没有演员'})
  const review = {token, code: movie.code, dir: movie.dir, entries}
  pending = {review, root, generation}
  return structuredClone(review)
}
export function applySupplement(token: string, kinds: SupplementKind[], root: string, generation: number) {
  if (!pending || token !== pending.review.token || pending.root !== root || pending.generation !== generation) throw '资料预览已过期，请重新预览'
  if (!kinds.length) throw '请选择要补的资料'
  const {review} = pending
  pending = null
  const added = review.entries.filter(e => e.status === 'missing' && e.path && kinds.includes(e.kind) && !newFiles.has(e.path))
  for (const e of added) newFiles.add(e.path!)
  return {code:review.code, added}
}
export function hasSupplement(path:string) { return newFiles.has(path) }
export function undoSupplement(paths: string[]) { for (const path of paths) newFiles.delete(path) }
