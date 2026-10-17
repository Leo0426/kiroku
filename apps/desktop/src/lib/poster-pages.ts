/** Bounded LRU of immutable page responses. Visible pages are always pinned. */
export const POSTER_PAGE_LIMIT = 6
export const POSTER_REQUEST_LIMIT = 2
export interface PosterPage<T> { items: T[]; generation: number; used: number }

export class PosterPages<T> {
  private entries = new Map<number, PosterPage<T>>()
  private clock = 0
  fresh(page: number, generation: number) { return this.entries.get(page)?.generation === generation }
  clear() { this.entries.clear() }
  retain(required: number[]) {
    const pins = new Set(required)
    for (const page of this.entries.keys()) if (!pins.has(page)) this.entries.delete(page)
  }
  touch(required: number[]) {
    for (const page of required) { const entry = this.entries.get(page); if (entry) entry.used = ++this.clock }
  }
  trim(required: number[]) {
    const pins = new Set(required)
    const budget = Math.max(POSTER_PAGE_LIMIT, pins.size)
    const candidates = [...this.entries].filter(([page]) => !pins.has(page)).sort((a, b) => a[1].used - b[1].used)
    while (this.entries.size > budget && candidates.length) this.entries.delete(candidates.shift()![0])
  }
  set(page: number, items: T[], generation: number, required: number[]) {
    this.entries.set(page, { items, generation, used: ++this.clock })
    this.trim(required)
  }
  snapshot(): Record<number, PosterPage<T>> { return Object.fromEntries(this.entries) }
}
