<script lang="ts">
  import { api } from '../lib/api'
  import Icon from './Icon.svelte'

  let { code, url, libraryRoot, revisionToken, cachedFile, localName, size = 20, placeholder = false }: {
    code: string
    url?: string | null
    libraryRoot: string | null | undefined
    revisionToken?: string
    cachedFile?: string
    localName?: string
    size?: number
    placeholder?: boolean
  } = $props()
  let src = $state<string | null>(null)
  let loading = $state(false)
  $effect(() => {
    const currentCode = code, currentUrl = url, token = revisionToken, name = localName
    libraryRoot
    cachedFile
    src = null
    const hasImage = !!(currentUrl || name)
    loading = hasImage
    if (!hasImage) return
    let alive = true
    let objectUrl: string | null = null
    api.actorImage(currentCode, currentUrl ?? '', token, name).then(image => {
      if (!alive) return
      objectUrl = URL.createObjectURL(new Blob([new Uint8Array(image.bytes)], { type: image.mime }))
      src = objectUrl
    }).catch(() => { /* Keep the actor's name visible when an upstream image fails. */ })
      .finally(() => { if (alive) loading = false })
    return () => {
      alive = false
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  })
</script>

{#if src}
  <img class="avatar" width={size} height={size} {src} alt="" onerror={() => { src = null }} />
{:else if placeholder}
  {@const label = loading ? '正在加载头像' : url ? '头像无法加载' : '暂无头像'}
  <span class="placeholder" style:--size="{size}px" role="img" aria-label={label} title={label}>
    <Icon name={loading ? 'loader' : 'image'} size={15} />
  </span>
{/if}

<style>
  .avatar, .placeholder { display: block; flex-shrink: 0; border-radius: 50%; }
  .avatar { object-fit: cover; }
  .placeholder { width: var(--size); height: var(--size); display: grid; place-items: center; background: var(--surface-3); color: var(--text-2); }
</style>
