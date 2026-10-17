/** Versions describe bytes, so unchanged images retain the exact same URL. */
export function assetUrl(path: string | null, revision?: string | null, resolve: (path: string) => string = path => path): string | null {
  if (!path) return null
  // Inline images carry their content in the URL; query strings would corrupt it.
  if (/^(data|blob):/i.test(path)) return path
  const resolved = resolve(path)
  const hashAt = resolved.indexOf('#')
  const head = hashAt < 0 ? resolved : resolved.slice(0, hashAt)
  const fragment = hashAt < 0 ? '' : resolved.slice(hashAt)
  const queryAt = head.indexOf('?')
  const base = queryAt < 0 ? head : head.slice(0, queryAt)
  const parameters = new URLSearchParams(queryAt < 0 ? '' : head.slice(queryAt + 1))
  parameters.set('v', revision ?? 'legacy')
  return `${base}?${parameters}${fragment}`
}
