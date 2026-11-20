import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'
test('recovery opens over the desktop layout without pushing posters down', async ({page}) => {
  await seedLibrary(page)
  await overrideApi(page,`api.recoveryInspect=async()=>({root:'/mock/library',entries:[{id:'200',kind:'partial',codes:['ABC-001'],pendingOperations:1,message:'上次操作部分完成',canUndo:true}]});`)
  await page.emulateMedia({colorScheme:'dark'})
  await page.setViewportSize({width:960,height:720})
  await page.goto('/')
  const poster = page.locator('button.pcard').first()
  const dialog = page.getByRole('dialog',{name:'启动恢复'})
  await expect(dialog).toContainText('ABC-001')
  await expect(dialog.getByRole('button',{name:'检查并撤销'})).toBeVisible()
  await expect(poster).toBeVisible()
  const before = await poster.boundingBox()
  await page.screenshot({path:'/tmp/kiroku-layout-desktop.png'})
  await dialog.getByRole('button',{name:'关闭恢复事项'}).click()
  await expect(dialog).toBeHidden()
  const after = await poster.boundingBox()
  expect(before?.y).toBe(after?.y)
  expect(after!.y).toBeLessThan(300)
  expect(after!.y+after!.height).toBeLessThan(720)
})
test('library utilities are grouped and do not appear on pending tab',async({page})=>{
 await seedLibrary(page);await page.goto('/');await expect(page.getByRole('button',{name:'检查媒体库',exact:true})).toHaveCount(0);await page.getByRole('button',{name:'媒体库工具',exact:true}).click();await expect(page.getByRole('button',{name:'检查媒体库',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:'批量导出 NFO 语言',exact:true})).toBeVisible();await page.screenshot({path:'/tmp/kiroku-layout-tools.png'});await page.getByRole('tab',{name:/待整理/}).click();await expect(page.getByRole('button',{name:'检查媒体库',exact:true})).toHaveCount(0);await expect(page.getByRole('button',{name:'批量导出 NFO 语言',exact:true})).toHaveCount(0)
})
test('compact and wide layouts keep controls inside viewport',async({page})=>{
 await seedLibrary(page);await page.goto('/');for(const width of [720,960,1440]){await page.setViewportSize({width,height:800});await expect(page.getByRole('navigation',{name:'主导航'})).toBeVisible();expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);const search=await page.getByLabel('搜索',{exact:true}).boundingBox();expect(search!.x+search!.width).toBeLessThanOrEqual(width);await page.getByLabel('搜索',{exact:true}).focus();expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)}
})
test('history prioritizes operation records over maintenance controls',async({page})=>{await seedLibrary(page);await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'历史',exact:true}).click();await expect(page.getByRole('button',{name:'预览空间清理',exact:true})).toHaveCount(0);await page.getByRole('button',{name:'空间维护',exact:true}).click();await expect(page.getByRole('button',{name:'预览空间清理',exact:true})).toBeVisible()})

test('startup shows an indeterminate loading page until settings are ready', async ({page}) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => {
      await new Promise(resolve => window.finishStartup = resolve);
      return getSettings();
    };
  `)
  await page.goto('/')
  const progress = page.getByRole('progressbar', {name:'正在读取应用设置…'})
  await expect(progress).toBeVisible()
  expect(await progress.getAttribute('aria-valuenow')).toBeNull()
  await expect(page.getByRole('navigation', {name:'主导航'})).toBeHidden()
  await page.evaluate(() => (window as any).finishStartup())
  await expect(progress).toBeHidden()
  await expect(page.getByRole('heading', {name:'媒体库', exact:true})).toBeVisible()
})
