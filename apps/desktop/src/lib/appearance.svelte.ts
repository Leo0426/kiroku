export type Appearance = { theme: 'system' | 'light' | 'dark'; textScale: 100 | 115 | 130; motion: 'system' | 'reduced' }
const KEY = 'kiroku-appearance-v1'
const defaults: Appearance = { theme: 'system', textScale: 100, motion: 'system' }
function read(): Appearance {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return { theme: ['light', 'dark'].includes(p?.theme) ? p.theme : 'system', textScale: [115, 130].includes(p?.textScale) ? p.textScale : 100, motion: p?.motion === 'reduced' ? 'reduced' : 'system' }
  } catch { return { ...defaults } }
}
class AppearanceStore {
  value = $state<Appearance>(read())
  apply() {
    const root = document.documentElement
    root.dataset.theme = this.value.theme === 'system' ? (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : this.value.theme
    root.dataset.motion = this.value.motion === 'reduced' || matchMedia('(prefers-reduced-motion: reduce)').matches ? 'reduced' : 'full'
    root.style.setProperty('--text-scale', String(this.value.textScale / 100))
  }
  update(patch: Partial<Appearance>): boolean {
    this.value = { ...this.value, ...patch }
    this.apply()
    try { localStorage.setItem(KEY, JSON.stringify(this.value)); return true } catch { return false }
  }
  init() {
    this.apply()
    for (const query of ['(prefers-color-scheme: light)', '(prefers-reduced-motion: reduce)']) matchMedia(query).addEventListener('change', () => this.apply())
  }
}
export const appearance = new AppearanceStore()
