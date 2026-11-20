import { expect, test } from '@playwright/test'
import { readFile } from 'node:fs/promises'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({page}) => seedLibrary(page))

test('custom CSV exports chosen columns and films across pages',async({page})=>{
  await page.goto('/')
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await page.getByRole('button',{name:'自定义 CSV',exact:true}).click()
  const panel=page.getByRole('region',{name:'自定义 CSV',exact:true})
  await panel.getByLabel('CSV 影片范围').selectOption('selection')
  const choose=async()=>{
    const pick=panel.getByRole('checkbox',{name:/^CSV /}).first()
    await pick.check()
    return (await pick.getAttribute('aria-label'))!.slice(4)
  }
  const first=await choose()
  await panel.getByRole('button',{name:'下一页'}).click()
  const next=await choose()
  expect(first).not.toBe(next)
  const columns=panel.getByRole('group',{name:'导出字段'})
  for(const box of await columns.getByRole('checkbox').all())await box.uncheck()
  await expect(panel.getByRole('button',{name:'保存 CSV 清单'})).toBeDisabled()
  for(const name of ['编号','标签','文件路径'])await columns.getByRole('checkbox',{name,exact:true}).check()
  const download=page.waitForEvent('download')
  await panel.getByRole('button',{name:'保存 CSV 清单'}).click()
  const content=await readFile((await (await download).path())!, 'utf8')
  expect(content.split('\r\n')[0]).toBe('\ufeff编号,标签,文件路径')
  expect(content).toContain('"'+first+'"');expect(content).toContain('"'+next+'"')
  expect(content).toContain('/mock/library/');expect(content).toContain('.mp4')
  await expect(page.getByText('已导出 2 部影片的 CSV 清单')).toBeVisible()
})

test('batch NFO resolves the full library and shows the actual NFO path before writing',async({page})=>{
  await overrideApi(page,`const list=api.libraryList; api.nfoExportPreview=async(codes,language)=>{const card=(await list({offset:0,limit:1})).items[0]; window.resolvedCodes=codes;return {token:'preview',root:'/mock/library',items:[{code:card.code,nfoPath:'/mock/library/Example/movie.nfo',dir:'/mock/library/Example',changes:[],warnings:[]}]}};`)
  await page.goto('/')
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await page.getByRole('button',{name:'批量导出 NFO 语言',exact:true}).click()
  const batch=page.getByRole('region',{name:'批量 NFO 语言导出'})
  await batch.getByLabel('NFO 影片范围').selectOption('library')
  await batch.getByRole('button',{name:'预览批量 NFO 导出'}).click()
  await expect(batch).toContainText('/mock/library/Example/movie.nfo')
  expect(await page.evaluate(()=>(window as any).resolvedCodes.length)).toBeGreaterThan(500)
  await expect(batch.getByRole('button',{name:'确认批量导出'})).toBeEnabled()
})
