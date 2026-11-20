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

test('missing originals display a labeled preview in the wall, list and detail until a refresh restores the original', async ({ page }) => {
  await seedLibrary(page)
  let restored = false
  await page.route('**/image-test/**', async route => {
    const kind = new URL(route.request().url()).pathname.split('/').pop()
    if (kind === 'preview' || (restored && kind === 'poster')) {
      await route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="6"/>' })
    } else await route.fulfill({ status:404, body:'' })
  })
  await overrideApi(page, `
    const list=api.libraryList, get=api.libraryGet;
    const image=location.origin+'/image-test/';
    const patch=m=>({...m,poster:image+'poster',posterVersion:'v-one'});
    api.libraryList=async q=>{const p=await list(q);return {...p,total:1,items:p.items.slice(0,1).map(patch)}};
    api.libraryGet=async code=>({...patch(await get(code)),fanart:image+'fanart',files:[]});
    window.cacheReads=[];
    api.cachedPoster=async (code,v)=>{window.cacheReads.push([code,v]);return image+'preview'};
    api.libraryPruneMissing=async root=>({root,removed:0,skipped:false});
  `)
  await page.goto('/')
  const card = page.locator('button.pcard').first()
  await expect(card).toContainText('缓存海报')
  await expect(card.locator('img')).toHaveAttribute('src', /image-test\/preview/)
  await page.getByLabel('列表视图', {exact:true}).click()
  await expect(card).toContainText('缓存海报')
  await card.click()
  const panel = page.getByRole('complementary', {name:'影片详情'})
  await expect(panel).toContainText('缓存海报')
  await expect(panel.locator('.hero-img')).toHaveAttribute('src', /image-test\/preview/)
  await expect(panel.getByRole('button', {name:'播放',exact:true})).toBeDisabled()
  expect(await page.evaluate(() => (window as any).cacheReads.length)).toBe(1)
  await panel.getByRole('button', {name:'关闭详情'}).click()
  restored = true
  await page.getByRole('button', {name:'刷新媒体库',exact:true}).click()
  await expect(card).not.toContainText('缓存海报')
  await expect(card.locator('img.ready')).toHaveAttribute('src', /image-test\/poster/)
})

test('retired preview lookups cannot replace a newer image or leak between libraries', async ({ page }) => {
  await seedLibrary(page)
  await page.route('**/revision-test/**', async route => {
    if (route.request().url().includes('new')) await route.fulfill({contentType:'image/svg+xml',body:'<svg xmlns="http://www.w3.org/2000/svg" width="4" height="6"/>'})
    else await route.fulfill({status:404,body:''})
  })
  await overrideApi(page, `
    const list=api.libraryList;
    window.rev='old'; window.lookups=[];
    api.libraryList=async q=>{const p=await list(q);return {...p,total:1,items:p.items.slice(0,1).map(m=>({...m,poster:location.origin+'/revision-test/'+window.rev,posterVersion:window.rev}))}};
    api.cachedPoster=(code,v)=>new Promise(resolve=>window.lookups.push({v,resolve}));
  `)
  await page.goto('/')
  await expect.poll(() => page.evaluate(() => (window as any).lookups.length)).toBe(1)
  await page.evaluate(async () => {
    const {app}=await import('/src/lib/state.svelte.ts')
    ;(window as any).rev='new'
    app.libraryVersion++
  })
  const image = page.locator('button.pcard img.ready').first()
  await expect(image).toHaveAttribute('src', /revision-test\/new/)
  await page.evaluate(() => (window as any).lookups[0].resolve('/revision-test/wrong'))
  await expect(image).toHaveAttribute('src', /revision-test\/new/)
  await page.evaluate(async () => {
    const {app}=await import('/src/lib/state.svelte.ts')
    ;(window as any).rev='old'
    app.settings={...app.settings!,library_root:'/mock/other-library'}
  })
  await expect.poll(() => page.evaluate(() => (window as any).lookups.length)).toBe(2)
})

test('visible native assets warm at most two previews together and reuse the same revisions', async ({ page }) => {
  await seedLibrary(page)
  await page.route('http://asset.localhost/**', route => route.fulfill({
    contentType:'image/svg+xml', body:'<svg xmlns="http://www.w3.org/2000/svg" width="4" height="6"/>',
  }))
  await overrideApi(page, `
    const list=api.libraryList;
    api.libraryList=async q=>{const p=await list(q);return {...p,total:4,items:p.items.slice(0,4).map((m,i)=>({...m,poster:'http://asset.localhost/native/'+i,posterVersion:'r1'}))}};
    window.warming={calls:0,active:0,max:0};
    api.cachedPoster=async (code,v)=>{
      const state=window.warming;
      state.calls++;state.active++;state.max=Math.max(state.max,state.active);
      await new Promise(r=>setTimeout(r,75));
      state.active--;return 'http://asset.localhost/preview/'+code;
    };
  `)
  await page.goto('/')
  await expect(page.locator('button.pcard img.ready')).toHaveCount(4)
  await expect.poll(() => page.evaluate(() => (window as any).warming)).toEqual({calls:4,active:0,max:2})
  await page.getByRole('navigation').getByRole('button', {name:'历史',exact:true}).click()
  await page.getByRole('navigation').getByRole('button', {name:'媒体库',exact:true}).click()
  await expect(page.locator('button.pcard img.ready')).toHaveCount(4)
  expect(await page.evaluate(() => (window as any).warming.calls)).toBe(4)
})

test('an unreadable cached preview stops at the retry action', async ({ page }) => {
  await seedLibrary(page)
  const paths: string[] = []
  await page.route('**/unreadable/**', async route => {
    paths.push(new URL(route.request().url()).pathname)
    await route.fulfill({status:404,body:''})
  })
  await overrideApi(page, `
    const list=api.libraryList;
    api.libraryList=async q=>{const p=await list(q);return {...p,total:1,items:p.items.slice(0,1).map(m=>({...m,code:'DEMO-001',poster:location.origin+'/unreadable/original',posterVersion:'one'}))}};
    api.cachedPoster=async()=>location.origin+'/unreadable/cache';
  `)
  await page.goto('/')
  const retry = page.getByRole('button', {name:'重试 DEMO-001 的海报'})
  await expect(retry).toBeVisible()
  expect(paths).toEqual(['/unreadable/original','/unreadable/cache'])
  await retry.click()
  await expect(page.getByRole('status', {name:'仍无法读取 DEMO-001 的海报'})).toBeVisible()
  expect(paths).toEqual(['/unreadable/original','/unreadable/cache','/unreadable/original','/unreadable/cache'])
})
