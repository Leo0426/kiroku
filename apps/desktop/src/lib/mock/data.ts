// 浏览器预览用的假数据:确定性生成,海报是内联 SVG(渐变 + 番号),不依赖任何图片文件。

export interface MockMovie {
  code: string
  title: string
  originalTitle: string
  year: number
  releaseDate: string
  actors: string[]
  studio: string
  publisher: string
  series: string | null
  tags: string[]
  plot: string
  runtime: number
  score: number
  contentType: string
  addedAt: number
  parts: number
  missingPoster?: boolean
  favorite?: boolean
  missingFanart?: boolean
  hue: number
}

/** 可复现的伪随机数 */
function rng(seed: number) {
  let s = seed >>> 0
  return () => {
    s = (s + 0x6d2b79f5) >>> 0
    let t = s
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const ACTORS = ['演员甲', '演员乙', '演员丙', '演员丁', '演员戊', '演员己', '演员庚', '演员辛', '演员壬', '演员癸', '演员子', '演员丑', '演员寅', '演员卯']
const STUDIOS: [string, string][] = [
  ['Studio One', 'SMPL'], ['Studio Two', 'MOVX'], ['Studio Three', 'QRS'], ['Studio Four', 'PQRST'], ['Studio Five', 'TUV'],
  ['Studio Six', 'TUW'], ['Studio Seven', 'EFGH'], ['Studio Eight', 'WXYZ'], ['Studio Nine', 'PRVW'], ['Studio Ten', 'DEFG'],
]
const T1 = ['海辺の', '放課後の', '隣町の', '深夜の図書館で', '温泉旅行で', '雨の日の', '夏休みの', '小さな', '突然の', '秘密の']
const T2 = ['友人', '上司', '後輩', '家族', '同級生', '隣人', '先輩', '幼なじみ', '先生', '旅人']
const T3 = ['との三日間', 'と二人旅', 'の約束', 'の手紙', 'と過ごした夏', 'の本音', '物語', '日記']
const TAGS = ['ドラマ', '恋愛', 'コメディ', '単体作品', 'ハイビジョン', '独占配信', 'ドキュメンタリー', '旅行', '日常', 'メガネ', 'ショートカット', '音楽']
const PLOT = '小さな港町に引っ越してきた主人公が、古い写真館の店主と出会い、町に残された手紙の謎を追いかける。——数日間の、静かで少し不思議な暮らしの記録。'

export function genMovie(i: number): MockMovie {
  const r = rng(i * 7919 + 13)
  const pick = <T,>(a: T[]) => a[Math.floor(r() * a.length)]
  const [studio, prefix] = STUDIOS[i % STUDIOS.length]
  const num = 1 + Math.floor(r() * 899)
  const year = 2019 + Math.floor(r() * 7)
  const month = 1 + Math.floor(r() * 12)
  const actors = [pick(ACTORS)]
  if (r() > 0.8) actors.push(pick(ACTORS).replace(actors[0], ''))
  const title = pick(T1) + pick(T2) + pick(T3)
  return {
    code: `${prefix}-${String(num).padStart(3, '0')}`,
    title,
    originalTitle: title,
    year,
    releaseDate: `${year}-${String(month).padStart(2, '0')}-${String(1 + Math.floor(r() * 27)).padStart(2, '0')}`,
    actors: actors.filter(Boolean),
    studio,
    publisher: studio,
    series: r() > 0.7 ? `${pick(T2)}シリーズ` : null,
    tags: [pick(TAGS), pick(TAGS), pick(TAGS)].filter((v, k, a) => a.indexOf(v) === k),
    plot: PLOT,
    runtime: 90 + Math.floor(r() * 90),
    score: 60 + Math.floor(r() * 38),
    contentType: 'censored',
    addedAt: 1_700_000_000 + i * 3600,
    parts: r() > 0.92 ? 2 : 1,
    hue: Math.floor(r() * 360),
  }
}

/** 渐变 + 番号的 SVG 海报(2:3 附近,与真实 poster 的 309×438 同比例) */
export function posterSvg(code: string, hue: number): string {
  const h2 = (hue + 40) % 360
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 309 438">
<defs>
<linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 48% 34%)"/><stop offset="1" stop-color="hsl(${h2} 55% 12%)"/></linearGradient>
<radialGradient id="l" cx="0.2" cy="0.1" r="0.9"><stop offset="0" stop-color="#fff" stop-opacity="0.22"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>
</defs>
<rect width="309" height="438" fill="url(#g)"/><rect width="309" height="438" fill="url(#l)"/>
<circle cx="230" cy="130" r="92" fill="hsl(${h2} 60% 55%)" opacity="0.16"/>
<circle cx="90" cy="260" r="140" fill="hsl(${hue} 60% 50%)" opacity="0.1"/>
<rect x="24" y="352" width="34" height="3" rx="1.5" fill="#fff" opacity="0.7"/>
<text x="24" y="392" font-family="Helvetica, Arial, sans-serif" font-size="30" font-weight="700" letter-spacing="1" fill="#fff" opacity="0.92">${code}</text>
</svg>`
  return 'data:image/svg+xml;utf8,' + encodeURIComponent(svg)
}
