import { mount } from 'svelte'
import './styles/base.css'
import App from './App.svelte'
import { reduced } from './lib/motion'

// 全局点击涟漪:按钮与 .ripple-host 元素在按下处泛起一圈
document.addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || reduced()) return
  const host = (e.target as HTMLElement | null)?.closest<HTMLElement>('.btn:not(:disabled), .ripple-host')
  if (!host) return
  const r = host.getBoundingClientRect()
  const size = Math.max(r.width, r.height) * 2.2
  const dot = document.createElement('span')
  dot.className = 'ripple'
  dot.style.cssText = `width:${size}px;height:${size}px;left:${e.clientX - r.left}px;top:${e.clientY - r.top}px`
  host.appendChild(dot)
  dot.addEventListener('animationend', () => dot.remove())
})

export default mount(App, { target: document.getElementById('app')! })
