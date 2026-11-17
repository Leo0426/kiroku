<script lang="ts">
  import { api } from '../lib/api'
  let { code, url, libraryRoot }: { code: string; url: string; libraryRoot: string | null | undefined } = $props()
  let src = $state<string | null>(null)
  $effect(() => {
    const currentCode = code, currentUrl = url
    libraryRoot
    src = null
    let alive = true
    let objectUrl: string | null = null
    api.actorImage(currentCode, currentUrl).then(image => {
      if (!alive) return
      objectUrl = URL.createObjectURL(new Blob([new Uint8Array(image.bytes)], { type: image.mime }))
      src = objectUrl
    }).catch(() => { /* Keep the actor's name visible when an upstream image fails. */ })
    return () => {
      alive = false
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  })
</script>

{#if src}<img class="avatar" {src} alt="" onerror={() => { src = null }} />{/if}

<style>
  .avatar { width: 20px; height: 20px; border-radius: 50%; object-fit: cover; }
</style>
