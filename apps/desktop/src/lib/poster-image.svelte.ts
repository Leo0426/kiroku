import { api } from './api'
import { forgetPosterPreview, posterPreview } from './poster-previews'
import type { MovieCard } from './types'

/** One original image attempt, followed by a read-only preview for that exact revision. */
export function posterImage(
  movie: () => MovieCard | null,
  libraryRoot: () => string | null | undefined,
  onRetryFailed?: (code: string) => void,
) {
  const root = $derived(libraryRoot())
  const code = $derived(movie()?.code)
  const path = $derived(movie()?.poster ?? null)
  const version = $derived(movie()?.posterVersion ?? null)
  let retryToken = $state('')
  let cachedUrl = $state<string | null>(null)
  let stage = $state<'original' | 'lookup' | 'cached' | 'failed'>('original')
  let loaded = $state(false)
  let generation = 0
  let reportFailure = false
  const original = $derived(api.posterUrl(path, retryToken ? `${version ?? 'legacy'}.${retryToken}` : version))
  const src = $derived(stage === 'original' ? original : stage === 'cached' ? cachedUrl : null)

  $effect.pre(() => {
    root; code; path; version
    stage = 'original'
    reportFailure = false
    loaded = false
    retryToken = ''
    cachedUrl = null
    return () => { generation++ }
  })

  function finishFailure() {
    stage = 'failed'
    if (reportFailure && code) onRetryFailed?.(code)
    reportFailure = false
  }

  function displayed(event: Event) {
    const image = event.currentTarget as HTMLImageElement
    if (!image.isConnected || image.getAttribute('src') !== src) return
    loaded = true
    if (stage === 'original' && root && code && version && original && /^(asset:\/\/|https?:\/\/asset\.localhost\/)/i.test(original)) {
      const attempt = generation
      void posterPreview(root, code, version, () => attempt === generation, true)
    }
  }

  async function unavailable(event: Event) {
    const image = event.currentTarget as HTMLImageElement
    if (!image.isConnected || !src || image.getAttribute('src') !== src) return
    loaded = false
    if (stage === 'cached' || !root || !code || !version) {
      if (stage === 'cached' && root && code && version) forgetPosterPreview(root, code, version)
      finishFailure()
      return
    }
    stage = 'lookup'
    const attempt = generation
    const preview = await posterPreview(root, code, version, () => attempt === generation)
    if (attempt !== generation) return
    if (preview) {
      cachedUrl = preview
      stage = 'cached'
    } else finishFailure()
  }

  function retry(manual = false) {
    if (stage === 'original' || !path) return
    generation++
    reportFailure = manual
    loaded = false
    retryToken = crypto.randomUUID()
    stage = 'original'
  }

  return {
    get src() { return src },
    get loaded() { return loaded },
    get cached() { return stage === 'cached' && loaded },
    get failed() { return stage === 'failed' },
    displayed, unavailable, retry,
  }
}
