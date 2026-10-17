export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const u = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`
}

/** 秒 → "1 分 05 秒" / "42 秒" */
export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs))
  if (s < 60) return `${s} 秒`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} 分 ${String(s % 60).padStart(2, '0')} 秒`
  return `${Math.floor(m / 60)} 小时 ${String(m % 60).padStart(2, '0')} 分`
}

export function fmtDateTime(ts: number): string {
  const d = new Date(ts)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** 路径的最后一段,同时兼容 / 与 \ */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

/** 把过长的路径从中间截断,保留盘符/根与文件名 */
export function shortPath(path: string, max = 56): string {
  if (path.length <= max) return path
  const parts = path.split(/([\\/])/)
  const tail = parts.slice(-5).join('')
  return '…' + tail.slice(-(max - 1))
}
