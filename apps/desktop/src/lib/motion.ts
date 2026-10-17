import { cubicOut } from 'svelte/easing'
import type { TransitionConfig } from 'svelte/transition'

/** 用户开启「减少动态效果」时,所有位移/缩放过渡直接取消 */
export const reduced = (): boolean =>
  typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches

export const dur = (ms: number): number => (reduced() ? 0 : ms)

/** 淡入 + 轻微上移:视图切换、列表项入场 */
export function rise(_node: Element, { delay = 0, duration = 260, y = 8 } = {}): TransitionConfig {
  return {
    delay: reduced() ? 0 : delay,
    duration: dur(duration),
    easing: cubicOut,
    css: (t) => `opacity:${t};transform:translateY(${(1 - t) * y}px)`,
  }
}

/** 从右侧滑入:详情面板 */
export function slideIn(_node: Element, { duration = 320, x = 40 } = {}): TransitionConfig {
  return {
    duration: dur(duration),
    easing: cubicOut,
    css: (t) => `opacity:${t};transform:translateX(${(1 - t) * x}px)`,
  }
}
