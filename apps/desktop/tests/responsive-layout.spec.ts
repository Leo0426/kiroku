import { expect, test, type Page } from '@playwright/test'
import { seedLibrary, overrideApi } from './helpers'

async function fits(page: Page) {
  const overflow = await page.locator('.view-shell, [aria-label="影片详情"]').evaluateAll(els => els.map(el => ({name: el.getAttribute('aria-label') ?? el.className, width: el.clientWidth, content: el.scrollWidth})).filter(el => el.content > el.width + 1))
  expect(overflow).toEqual([])
}

test('history keeps metadata together and long errors below it at every window size', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `api.historyList=async()=>({total:1,next:null,items:[{id:'long-error',ts:1791699480000,status:'partial',itemCount:12,opCount:123,trashed:0,codes:['ABC-001'],pendingOperations:3,error:'无法读取日志 /Volumes/'+ 'verylongdirectory'.repeat(60)+'journal.json'}]});`)
  await page.goto('/')
  await page.getByRole('navigation', {name:'主导航'}).getByRole('button', {name:'历史',exact:true}).click()
  for (const width of [480,720,820,960,1280,1920]) {
    await page.setViewportSize({width,height:720})
    await fits(page)
    const when = await page.locator('.when').boundingBox()
    expect(when!.height).toBeLessThan(30)
    const error = await page.locator('.item [role=alert]').boundingBox()
    const meta = await page.locator('.item .main').boundingBox()
    expect(error!.y).toBeGreaterThanOrEqual(meta!.y + meta!.height)
    const actions = await page.getByRole('button',{name:'查看日志',exact:true}).boundingBox()
    expect(actions!.x + actions!.width).toBeLessThanOrEqual(width)
    if(width===960) await page.screenshot({path:'/tmp/kiroku-responsive-history.png'})
  }
})

test('all main pages and settings categories fit compact and large windows with long paths', async ({page}) => {
  test.setTimeout(60000)
  await seedLibrary(page)
  await overrideApi(page, `const originalSettings=api.getSettings;api.getSettings=async()=>({...await originalSettings(),library_root:'/mock/'+ 'long-library-path/'.repeat(30)});`)
  await page.goto('/')
  for (const width of [480,720,820,960,1280,1920]) {
    await page.setViewportSize({width,height:720})
    for (const name of ['媒体库','导入整理','历史','设置']) {
      await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name,exact:true}).click()
      await fits(page)
      if (name === '设置') {
        for (const tab of await page.getByRole('tab').all()) {
          await tab.click()
          while(await page.locator('details:not([open]) > summary:visible').count()) { await page.locator('details:not([open]) > summary:visible').first().click() }
          await fits(page)
        }
      }
    }
  }
})

 test('confirmation with a long path remains scrollable in short windows',async({page})=>{
  await seedLibrary(page);await page.setViewportSize({width:480,height:360});await page.goto('/');
  await page.evaluate(async()=>{const {app}=await import('/src/lib/state.svelte.ts');void app.confirm({title:'确认操作',body:'/Volumes/'+ 'longfilename'.repeat(150),confirmLabel:'确认'});});
  const dialog=page.getByRole('alertdialog');await expect(dialog).toBeVisible();
  expect(await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
  const box=await dialog.boundingBox();expect(box!.y).toBeGreaterThanOrEqual(0);expect(box!.height).toBeLessThanOrEqual(360);
  await dialog.getByRole('button',{name:'取消',exact:true}).click();await expect(dialog).toHaveCount(0);
 })

 test('details and editors stay above compact navigation and fit long metadata',async({page})=>{
  await seedLibrary(page);await overrideApi(page,`const originalGet=api.libraryGet;api.libraryGet=async(code)=>{const d=await originalGet(code);if(d){d.meta.title='LongTitle'.repeat(70);d.meta.studio='LongStudio'.repeat(70);d.meta.actors=[{name:'LongActor'.repeat(70),gender:'female'}]}return d};`);
  await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true});
  for(const width of [480,720,820,960,1440]){
    await page.setViewportSize({width,height:600});await fits(page);
    // Even reduced-motion CSS transitions settle on the next animation frame after resize.
    // Read both bounds together and wait for the required layout, without a fixed delay.
    if(width<=720) await expect.poll(() => panel.evaluate(el => {
      const nav = document.querySelector('[aria-label="主导航"]')!
      return el.getBoundingClientRect().bottom - nav.getBoundingClientRect().top
    })).toBeLessThanOrEqual(1)
    await panel.getByRole('button',{name:'编辑元数据',exact:true}).click();await fits(page);
    await panel.getByRole('button',{name:'取消编辑',exact:true}).click();
    if(width===480) await page.screenshot({path:'/tmp/kiroku-responsive-detail.png'});
  }
 })

 test('scan, scrape and plan lists fit compact windows',async({page})=>{
  test.setTimeout(60000);await seedLibrary(page);await overrideApi(page,`const getSettings=api.getSettings;api.getSettings=async()=>({...await getSettings(),source_dirs:['/mock/source']});const scrapeStart=api.scrapeStart;api.scrapeStart=async(ids)=>scrapeStart(ids.filter(id=>id===3||id===4));`);
  await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'导入整理',exact:true}).click();
  await page.getByRole('button',{name:'开始扫描',exact:true}).click();await expect(page.getByRole('button',{name:/查找影片信息/})).toBeEnabled();
  await page.getByText(/查看已识别文件/).click();
  for(const width of [480,720,820,960,1440]){await page.setViewportSize({width,height:600});await fits(page)}
  await page.getByRole('button',{name:/查找影片信息/}).click();await expect(page.getByRole('button',{name:'预览整理',exact:true})).toBeEnabled({timeout:20000});
  for(const width of [480,720,820,960,1440]){await page.setViewportSize({width,height:600});await fits(page)}
  await page.getByRole('button',{name:'预览整理',exact:true}).click();await expect(page.getByRole('button',{name:/确认整理/})).toBeVisible();
  for(const width of [480,720,820,960,1440]){await page.setViewportSize({width,height:600});await fits(page)}
 })

test('library utilities and pending queue fit every sidebar breakpoint',async({page})=>{
 await seedLibrary(page);await page.goto('/');
 for(const width of [480,720,800,820,900,960,1440]){
  await page.setViewportSize({width,height:600});
  await page.getByRole('tab',{name:/已整理/}).click();await page.getByRole('button',{name:'媒体库工具',exact:true}).click();await fits(page);
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click();await page.getByRole('tab',{name:/待整理/}).click();await fits(page);
 }
})
