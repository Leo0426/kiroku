import { expect, test, type Page } from '@playwright/test'
import { overrideApi } from './helpers'

const nav = (page: Page) => page.getByRole('navigation', { name:'主导航' })
test.beforeEach(async ({page}) => {
  await page.addInitScript(() => localStorage.getItem('kiroku-mock-settings-v2') || localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
    library_root:'/mock/library', source_dirs:['/mock/companion-import'], file_template:'{code}', dir_template:'{code}' })))
})

test('batch settings change the preview without persisting and superseded previews cannot execute', async ({page}) => {
  await overrideApi(page,`
    const builds=[];
    const build=api.planBuild;
    api.planBuild=async(...args)=>{
      const plans=await build(...args);
      builds.push({token:args[1],ids:plans.filter(p=>p.selected).map(p=>p.itemId)});
      window.checkOldImportPlan=async()=>{
        if(builds.length<2 || !builds[0].token || builds[0].token===builds.at(-1).token || !builds[0].ids.length)
          throw new Error('测试必须先生成两个不同且可执行的预览');
        try{await api.applyStart(builds[0].ids,builds[0].token);return 'executed'}catch(e){return String(e)}
      };
      window.readImportDefault=async()=>(await api.getSettings()).organize_mode;
      return plans;
    };
  `)
  await page.goto('/'); await nav(page).getByRole('button',{name:'导入整理'}).click()
  await page.getByRole('button',{name:'开始扫描'}).click()
  await page.getByRole('button',{name:'预览整理',exact:true}).click()
  await page.getByRole('button',{name:'调整本批规则'}).click()
  await page.getByRole('combobox',{name:'文件处理方式',exact:true}).selectOption('copy')
  await page.getByLabel('库内子目录').fill('Archive')
  await page.getByLabel('本批文件名模板').fill('{code} Alternate')
  await page.getByRole('button',{name:'应用并重新预览'}).click()
  await expect(page.locator('.to').first()).toContainText('Archive/')
  await expect(page.locator('.to').first()).toContainText('Alternate')
  await expect(page.getByRole('note',{name:'文件处理方式'})).toContainText('复制')
  expect(await page.evaluate(()=>(window as any).checkOldImportPlan())).toContain('重新预览')
  expect(await page.evaluate(()=>(window as any).readImportDefault())).toBe('move')
  await page.getByRole('button',{name:'操作明细'}).first().click()
  await expect(page.locator('.item-detail')).toContainText('复制')
})

test('reselecting sources keeps the batch rules and cache choice until a new batch starts', async ({ page }) => {
  await overrideApi(page, `
    const settings = api.getSettings;
    api.getSettings = async () => ({
      ...await settings(),
      source_dirs: ['/mock/companion-mixed/SMPL-421.mp4', '/mock/companion-mixed/MOVX-679-C.mp4'],
    });
  `)
  await page.goto('/')
  await nav(page).getByRole('button', { name: '导入整理' }).click()
  await page.getByRole('button', { name: '调整本批规则' }).click()
  await page.getByRole('combobox', { name: '文件处理方式', exact: true }).selectOption('copy')
  await page.getByLabel('库内子目录').fill('Archive')
  await page.getByRole('button', { name: '应用到本批' }).click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByText('查询选项', { exact: true }).click()
  await page.getByLabel('仅使用站点资料缓存').check()
  await page.getByRole('button', { name: '仅预览就绪影片' }).click()
  const plan = page.getByRole('region', { name: '整理计划', exact: true })
  await expect(plan.getByLabel('整理 SMPL-421')).toBeChecked()
  await page.getByRole('button', { name: '上一步' }).click()
  await page.getByRole('button', { name: '重新选择' }).click()
  await expect(page.getByRole('heading', { name: '复制到媒体库' })).toBeVisible()
  await expect(page.getByLabel(/只复用已有来源缓存/)).toBeChecked()
  await page.getByRole('button', { name: '移除 /mock/companion-mixed/SMPL-421.mp4' }).click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await expect(plan.getByRole('checkbox')).toHaveCount(1)
  await expect(plan.getByLabel('整理 MOVX-679')).toBeEnabled()
  await expect(plan).toContainText('Archive/')
  await expect(page.getByRole('note', { name: '文件处理方式' })).toContainText('复制')
  await plan.getByLabel('整理 MOVX-679').check()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await page.getByRole('button', { name: '再来一批' }).click()
  await expect(page.getByRole('heading', { name: '移动到媒体库' })).toBeVisible()
  await expect(page.getByLabel(/只复用已有来源缓存/)).toHaveCount(0)
  await expect(page.getByRole('region', { name: '整理目标' })).not.toContainText('Archive')
})

test('the next batch starts with the saved folders after organizing a single pending file', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('tab', { name: /待整理/ }).click()
  await page.getByRole('checkbox', { name: '选择 SMPL-421.mp4', exact: true }).check()
  await page.getByRole('button', { name: '仅整理所选文件' }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: '确认整理（1 个文件）' }).click()
  await page.getByRole('button', { name: '再来一批' }).click()
  await expect(page.getByRole('button', { name: '移除 /mock/companion-import', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '移除 /mock/companion-import/SMPL-421.mp4' })).toHaveCount(0)
})

test('defaults are saved only by the explicit action, and invalid subdirectories can be fixed in place',async({page})=>{
  await page.goto('/');await nav(page).getByRole('button',{name:'导入整理'}).click()
  await page.getByRole('button',{name:'调整本批规则'}).click()
  await page.getByLabel('库内子目录').fill('../unsafe')
  await page.getByRole('button',{name:'设为导入默认'}).click()
  await expect(page.getByRole('alert')).toBeVisible()
  await page.getByLabel('库内子目录').fill('Shelf')
  await page.getByRole('combobox',{name:'文件处理方式',exact:true}).selectOption('hardlink')
  await page.getByRole('button',{name:'设为导入默认'}).click()
  await expect(page.getByRole('button',{name:'开始扫描'})).toBeEnabled()
  await page.reload();await nav(page).getByRole('button',{name:'导入整理'}).click()
  await page.getByRole('button',{name:'调整本批规则'}).click()
  await expect(page.getByRole('combobox',{name:'文件处理方式',exact:true})).toHaveValue('hardlink')
  await expect(page.getByLabel('本批目录模板')).toHaveValue('Shelf/{code}')
})

test('a conflicting row can be skipped or saved under a different edition name', async({page})=>{
  await overrideApi(page,`const g=api.getSettings;api.getSettings=async()=>({...await g(),source_dirs:['/mock/source']});const scrape=api.scrapeStart;api.scrapeStart=(ids)=>scrape(ids.filter(id=>id===11));`)
  await page.goto('/');await nav(page).getByRole('button',{name:'导入整理'}).click()
  await page.getByRole('button',{name:'开始扫描'}).click()
  await page.getByRole('button',{name:/查找影片信息/}).click()
  await page.getByRole('button',{name:'预览整理',exact:true}).click()
  await page.getByRole('button',{name:'处理此项'}).click()
  await page.getByRole('button',{name:'跳过此项'}).click()
  await expect(page.getByText('已跳过',{exact:true})).toBeVisible()
  await page.getByRole('button',{name:'处理此项'}).click()
  await page.getByLabel('另存文件名后缀').fill('修订版')
  await page.getByRole('button',{name:'改名并重新预览'}).click()
  await expect(page.locator('.to').first()).toContainText('修订版')
  const row=page.getByRole('checkbox',{name:'整理 ABQ-100'})
  await expect(row).toBeEnabled(); await expect(row).not.toBeChecked()
  await row.check();await expect(page.getByRole('button',{name:/确认整理/})).toBeEnabled()
})

test('checking supplementary files keeps the reviewed rows visible and waits before applying the batch',async({page})=>{
  await overrideApi(page,`const get=api.getSettings;api.getSettings=async()=>({...await get(),source_dirs:['/mock/source']});
    const scrape=api.scrapeStart;api.scrapeStart=ids=>scrape(ids.filter(id=>id===3||id===11));
    api.supplementPreview=()=>new Promise((_,reject)=>window.finishSupplement=()=>reject('媒体库中没有匹配编号的影片'));`)
  await page.goto('/');await nav(page).getByRole('button',{name:'导入整理'}).click()
  await page.getByRole('button',{name:'开始扫描'}).click()
  await page.getByRole('button',{name:/查找影片信息/}).click()
  await page.getByRole('button',{name:'预览整理',exact:true}).click()
  const confirm=page.getByRole('button',{name:/确认整理/})
  await expect(confirm).toBeEnabled()
  const occupied=page.locator('.planrow').filter({has:page.getByRole('checkbox',{name:'整理 ABQ-100',exact:true})})
  await occupied.getByRole('button',{name:'处理此项'}).click()
  await page.getByRole('button',{name:'只补资料'}).click()
  await expect(confirm).toBeDisabled()
  await page.evaluate(()=>(window as any).finishSupplement())
  await expect(page.getByRole('region',{name:'只补资料'})).toContainText('媒体库中没有匹配编号的影片')
  await expect(confirm).toBeEnabled()
})
