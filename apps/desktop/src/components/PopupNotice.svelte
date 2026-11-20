<script lang="ts">
  import { untrack } from 'svelte'
  import { app, type Toast } from '../lib/state.svelte'

  let { text, title, action }: Pick<Toast, 'text' | 'title' | 'action'> = $props()
  $effect(() => {
    const notice = { text, title, action }
    const id = untrack(() => app.toast('error', notice.text, 0, notice.action, notice.title))
    return () => app.dismiss(id)
  })
</script>
