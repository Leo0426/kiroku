<script lang="ts">
  import { Tween } from 'svelte/motion'
  import { cubicOut } from 'svelte/easing'
  import { reduced } from '../lib/motion'

  /** 数字变化时滚动过去(减少动态效果时直接跳到目标值) */
  let { value }: { value: number } = $props()
  const t = new Tween(0, { duration: 600, easing: cubicOut })
  $effect(() => {
    t.set(value, { duration: reduced() ? 0 : 600 })
  })
</script>

<span class="tnum">{Math.round(t.current)}</span>
