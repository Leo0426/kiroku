import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'
import { createServer } from 'node:http'

test('long scrolling keeps retained page data and heap bounded', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.listCalls = [];
    api.libraryList = async q => {
      window.listCalls.push(q.offset || 0);
      return {total:100000,items:JSON.parse(JSON.stringify(Array.from({length:q.limit || 120},(_,i)=>{
        const code='CACHE-'+String((q.offset||0)+i).padStart(6,'0');
        return {code,title:('片名'+code).repeat(256),year:2025,actors:[],poster:null,posterVersion:null,
          contentType:'censored',score:null,addedAt:0,parts:1};
      })))};
    };
  `)
  await page.goto('/')
  const scroller = page.locator('.scroller')
  await expect(scroller.locator('button.pcard').first()).toBeVisible()
  const geometry = await scroller.evaluate(element => {
    const cards = [...element.querySelectorAll('button.pcard')]
    const top = cards[0].getBoundingClientRect().top
    const cols = cards.filter(card => Math.abs(card.getBoundingClientRect().top - top) < 1).length
    return {cols,row:cards[cols].getBoundingClientRect().top-top}
  })
  async function scrollTo(p: number) {
    await scroller.evaluate((element, { p, cols, row }) => { element.scrollTop = Math.floor(p * 120 / cols) * row }, { p, ...geometry })
    await expect(scroller).toHaveAttribute('aria-busy', 'false')
    await expect.poll(() => scroller.locator('.skel').count()).toBe(0)
    await expect(scroller.locator(`button.pcard[aria-label^="CACHE-${String(p * 120).padStart(6, '0')}"]`)).toHaveCount(1)
    expect(Number(await scroller.getAttribute('data-cached-pages'))).toBeLessThanOrEqual(6)
    expect(Number(await scroller.getAttribute('data-cached-cards'))).toBeLessThanOrEqual(720)
  }
  const cdp = await page.context().newCDPSession(page)
  async function heap() { await cdp.send('HeapProfiler.collectGarbage'); return (await cdp.send('Runtime.getHeapUsage')).usedSize }
  for (let p = 1; p <= 12; p++) await scrollTo(p)
  const warm = await heap()
  for (let p = 13; p <= 48; p++) await scrollTo(p)
  const end = await heap()
  console.log(JSON.stringify({warmHeap:warm,endHeap:end,growth:end-warm}))
  const nearby = await page.evaluate(() => (window as any).listCalls.length)
  await scrollTo(46)
  expect(await page.evaluate(() => (window as any).listCalls.length)).toBe(nearby)
  await scrollTo(0)
  expect(await page.evaluate(() => (window as any).listCalls.length)).toBeGreaterThan(nearby)
  expect(end - warm).toBeLessThan(8 * 1024 * 1024)
})

test('request concurrency stays bounded across rapid navigation and late responses', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const list = api.libraryList;
    window.holdPages = false; window.held = []; window.activePages = 0; window.peakPages = 0;
    api.libraryList = async q => {
      window.activePages++; window.peakPages = Math.max(window.peakPages, window.activePages);
      try { const result = await list(q); if(window.holdPages) await new Promise(resolve => window.held.push(resolve)); return result }
      finally {window.activePages--}
    };
  `)
  await page.goto('/')
  const wall = page.locator('.scroller')
  await expect(wall.locator('button.pcard').first()).toBeVisible()
  await page.evaluate(async () => { (window as any).holdPages = true; (await import('/src/lib/state.svelte.ts' as string)).app.libraryVersion++ })
  await wall.evaluate(element => {
    const cards = [...element.querySelectorAll('button.pcard')], top = cards[0].getBoundingClientRect().top
    const cols = cards.filter(card => Math.abs(card.getBoundingClientRect().top - top) < 1).length
    element.scrollTop = Math.floor(120 / cols) * (cards[cols].getBoundingClientRect().top-top)
  })
  await expect.poll(() => page.evaluate(() => (window as any).held.length)).toBe(2)
  for (let i = 0; i < 3; i++) {
    await page.getByRole('navigation').getByRole('button', { name: '历史', exact: true }).click()
    await page.getByRole('navigation').getByRole('button', { name: '媒体库', exact: true }).click()
    await expect(wall).toHaveAttribute('aria-busy', 'true')
  }
  expect(await page.evaluate(() => (window as any).activePages)).toBe(2)
  expect(await page.evaluate(() => (window as any).peakPages)).toBeLessThanOrEqual(2)
  await page.evaluate(() => { (window as any).holdPages = false; (window as any).held.forEach((release: () => void) => release()) })
  await expect(wall.locator('button.pcard').first()).toBeVisible()
  await expect(wall).toHaveAttribute('aria-busy', 'false')
  expect(await page.evaluate(() => (window as any).peakPages)).toBeLessThanOrEqual(2)
})

test('stable image versions reuse loaded bytes and show edits crops and undo correctly', async ({ page }) => {
  await seedLibrary(page)
  const bodies = new Map<string, { bytes: Buffer; type: string }>()
  const gets = new Map<string, number>()
  const server = createServer(async (request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*')
    if (request.method === 'OPTIONS') { response.setHeader('Access-Control-Allow-Methods', 'GET, POST'); response.setHeader('Access-Control-Allow-Headers', 'Content-Type'); response.end(); return }
    const url = new URL(request.url!, 'http://localhost')
    if (request.method === 'POST') {
      let body = ''; for await (const bytes of request) body += bytes.toString()
      const { version, source } = JSON.parse(body)
      const comma = source.indexOf(','), head = source.slice(0, comma), data = source.slice(comma + 1)
      bodies.set(version, { type: head.slice(5).split(';')[0], bytes: head.includes(';base64') ? Buffer.from(data, 'base64') : Buffer.from(decodeURIComponent(data), 'utf8') })
      response.end('ok'); return
    }
    const version = url.searchParams.get('v')!
    gets.set(version, (gets.get(version) ?? 0) + 1)
    response.setHeader('Cache-Control', 'public, max-age=3600')
    response.setHeader('Content-Type', bodies.get(version)?.type ?? 'image/svg+xml')
    response.end(bodies.get(version)?.bytes ?? '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>')
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = (server.address() as import('node:net').AddressInfo).port
  const base = `http://127.0.0.1:${port}`
  try {
    await page.goto('/')
    await expect(page.locator('button.pcard').first()).toBeVisible()
    const code = await page.evaluate(async base => {
      const { api } = await import('/src/lib/api.ts' as string)
      const code = (await api.libraryList({})).items[0].code
      const list = api.libraryList, get = api.libraryGet
      async function publish<T extends { code: string; poster: string | null; posterVersion: string | null }>(card: T): Promise<T> {
        if (card.code !== code || !card.poster?.startsWith('data:')) return card
        await fetch(base + '/register', { method: 'POST', headers: {'Content-Type':'application/json'}, body:JSON.stringify({version:card.posterVersion,source:card.poster}) })
        return { ...card, poster: base + '/poster.svg' }
      }
      api.libraryList = async query => { const result = await list(query); return { ...result, items: await Promise.all(result.items.map(publish)) } }
      api.libraryGet = async code => { const detail = await get(code); return detail ? publish(detail) : null }
      ;(window as any).imageApi = api
      ;(await import('/src/lib/state.svelte.ts' as string)).app.libraryVersion++
      return code
    }, base)
    const image = page.locator(`button.pcard[aria-label^="${code} "] img`)
    await expect.poll(() => image.getAttribute('src')).toContain(base)
    await expect.poll(() => image.evaluate(img => (img as HTMLImageElement).naturalWidth)).toBeGreaterThan(1)
    const originalUrl = (await image.getAttribute('src'))!
    const originalVersion = new URL(originalUrl).searchParams.get('v')!
    const originalWidth = await image.evaluate(img => (img as HTMLImageElement).naturalWidth)
    expect(gets.get(originalVersion)).toBe(1)
    // Metadata edits retain the image URL and already loaded image element.
    await page.evaluate(async code => {
      const api = (window as any).imageApi
      const preview = await api.revisionPreview(code, {title:'缓存测试新标题'})
      await api.revisionSave(preview.token, preview.root, code)
    }, code)
    await expect(page.locator(`button.pcard[aria-label^="${code} "]`)).toContainText('缓存测试新标题')
    expect(await image.getAttribute('src')).toBe(originalUrl)
    expect(gets.get(originalVersion)).toBe(1)
    // Real mock crop transactions produce new encoded image bytes/dimensions.
    const journalId = await page.evaluate(async code => {
      const api = (window as any).imageApi, original = await api.artworkOpen(code)
      const preview = await api.artworkPreview(original.token, original.root, code, {x:0,y:0,width:220,height:300})
      return (await api.revisionSave(preview.revision.token, preview.revision.root, code)).journalId
    }, code)
    await expect.poll(() => image.getAttribute('src')).not.toBe(originalUrl)
    await expect.poll(() => image.evaluate(img => (img as HTMLImageElement).naturalWidth)).toBe(220)
    const croppedUrl = (await image.getAttribute('src'))!
    expect(gets.get(new URL(croppedUrl).searchParams.get('v')!)).toBe(1)
    // Destroy/recreate the wall: an unchanged URL reuses HTTP-cached bytes.
    await page.getByRole('navigation').getByRole('button', { name:'历史', exact:true }).click()
    await page.getByRole('navigation').getByRole('button', { name:'媒体库', exact:true }).click()
    await expect(image).toHaveAttribute('src', croppedUrl)
    await expect.poll(() => image.evaluate(img => (img as HTMLImageElement).naturalWidth)).toBe(220)
    expect(gets.get(new URL(croppedUrl).searchParams.get('v')!)).toBe(1)
    await page.evaluate(async journalId => { await (window as any).imageApi.rollback(journalId) }, journalId)
    await expect(image).toHaveAttribute('src', originalUrl)
    await expect.poll(() => image.evaluate(img => (img as HTMLImageElement).naturalWidth)).toBe(originalWidth)
    expect(gets.get(originalVersion)).toBe(1)
  } finally {
    server.closeAllConnections()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('empty count checks do not compete with foreground revisions', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  const result = await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts' as string)
    const code = (await api.libraryList({})).items[0].code
    const counts = api.pendingSummary('empty-count', true)
    const preview = await api.revisionPreview(code, { title:'仅预览，不保存' })
    return {count:await counts,code:preview.code}
  })
  expect(result.count.total).toBe(0)
  expect(result.count.complete).toBe(true)
  expect(result.code).toBeTruthy()
})

test('asset URLs preserve escaped paths and inline images while replacing only the revision', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  const result = await page.evaluate(async () => {
    const { assetUrl } = await import('/src/lib/asset-url.ts' as string)
    const inline = 'data:image/svg+xml;charset=utf-8,%3Csvg%3E%3C%2Fsvg%3E'
    return { inline:assetUrl(inline,'new'), first:assetUrl('/poster%20%E7%94%B5%E5%BD%B1.jpg?source=local&v=old#anchor','a b'),
      legacy:assetUrl('/poster.jpg'), absent:assetUrl(null), blob:assetUrl('blob:test','new') }
  })
  expect(result.first).toBe('/poster%20%E7%94%B5%E5%BD%B1.jpg?source=local&v=a+b#anchor')
  expect(result.inline).toBe('data:image/svg+xml;charset=utf-8,%3Csvg%3E%3C%2Fsvg%3E')
  expect(result.legacy).toBe('/poster.jpg?v=legacy')
  expect(result.absent).toBeNull()
  expect(result.blob).toBe('blob:test')
})
