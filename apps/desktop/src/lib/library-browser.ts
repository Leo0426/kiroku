import type { FacetKind, Query } from './types'

export type Layout = 'posters' | 'list'
export type Density = 'compact' | 'comfortable' | 'large'
export interface SavedView { id: string; name: string; query: Query }
export interface BrowserState {
  query: Query; layout: Layout; density: Density; scroll: number
  saved: SavedView[]; recent: string[]
}
const KEY = 'kiroku-library-browser-v1'
const kinds: FacetKind[] = ['tag', 'actor', 'studio', 'series', 'badge', 'favorite']
export function cleanQuery(raw: unknown): Query {
  const q = raw && typeof raw === 'object' ? raw as Query : {}
  return {
    text: typeof q.text === 'string' ? q.text.slice(0, 300) : '',
    contentType: typeof q.contentType === 'string' ? q.contentType.slice(0, 80) : '',
    year: Number.isInteger(q.year) && q.year! >= 1 && q.year! <= 9999 ? q.year : undefined,
    sort: ['added', 'title', 'year', 'score'].includes(q.sort ?? '') ? q.sort : 'added',
    reverse: q.reverse === true,
    minScore: Number.isInteger(q.minScore) && q.minScore! >= 0 && q.minScore! <= 100 ? q.minScore : undefined,
    missing: ['poster', 'score', 'year'].includes(q.missing ?? '') ? q.missing : undefined,
    filters: Array.isArray(q.filters) ? q.filters.filter(f => f && kinds.includes(f.kind) && typeof f.value === 'string' && f.value.length > 0 && f.value.length <= 300).slice(0, 30).filter((f, i, a) => a.findIndex(x => x.kind === f.kind && x.value === f.value) === i).map(f => ({ kind: f.kind, value: f.value })) : [],
  }
}
export const defaultBrowser = (): BrowserState => ({ query: cleanQuery({}), layout: 'posters', density: 'comfortable', scroll: 0, saved: [], recent: [] })
function records(): Record<string, unknown> {
  try { const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}'); return raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {} } catch { return {} }
}
export function readBrowser(root: string): BrowserState {
  const raw = records()[root] as Partial<BrowserState> | undefined
  if (!raw || typeof raw !== 'object') return defaultBrowser()
  return {
    query: cleanQuery(raw.query), layout: raw.layout === 'list' ? 'list' : 'posters',
    density: raw.density === 'compact' || raw.density === 'large' ? raw.density : 'comfortable',
    scroll: Number.isFinite(raw.scroll) ? Math.max(0, Math.min(raw.scroll!, 100_000_000)) : 0,
    saved: Array.isArray(raw.saved) ? raw.saved.filter(s => s && typeof s.id === 'string' && typeof s.name === 'string' && s.name.trim()).slice(0, 20).filter((s, i, a) => a.findIndex(x => x.id === s.id) === i).map(s => ({ id: s.id.slice(0, 100), name: s.name.trim().slice(0, 40), query: cleanQuery(s.query) })) : [],
    recent: Array.isArray(raw.recent) ? [...new Set(raw.recent.filter(s => typeof s === 'string' && s.trim()).map(s => s.trim().slice(0, 300)))].slice(0, 8) : [],
  }
}
export function writeBrowser(root: string, value: BrowserState): boolean {
  if (!root) return true
  try {
    const all = records(); delete all[root]
    const entries = Object.entries(all).slice(-9)
    localStorage.setItem(KEY, JSON.stringify(Object.fromEntries([...entries, [root, value]])))
    return true
  } catch { return false }
}
export function rememberSearch(value: BrowserState, text: string) {
  const word = text.trim().slice(0, 300)
  if (word) value.recent = [word, ...value.recent.filter(v => v !== word)].slice(0, 8)
}

/** Same CSV shape as the native core exporter, for browser preview downloads. */
export function csvCell(value: string): string {
  const unsafe = /^[\s\u0000-\u001f\u007f]*[=+\-@＝＋－＠]/u.test(value) || /^[\t\r\n]/.test(value)
  return `"${unsafe ? "'" : ''}${value.replaceAll('"', '""')}"`
}
