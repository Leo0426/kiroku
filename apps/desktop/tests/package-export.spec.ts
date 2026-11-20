import { expect, test } from '@playwright/test'
import { seedLibrary, overrideApi } from './helpers'

test('exports a confirmed package and can open the same copy for restore', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page,`
    let saved = ''
    const copy = api.packageExport
    api.packageExport = async (...args) => {
      const result = await copy(...args)
      saved = result.destination
      return result
    }
    api.pickDirectory = async title => title.includes('保存') ? '/mock/copies' : saved
  `)
  await page.goto('/')
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  const panel=page.getByRole('region',{name:'导出资料包',exact:true})
  await panel.getByRole('button',{name:'导出资料包',exact:true}).click()
  await panel.getByRole('combobox').selectOption('selection')
  await panel.getByRole('group',{name:'影片范围'}).getByRole('checkbox').first().check()
  await panel.getByRole('button',{name:'选择保存位置'}).click()
  await panel.getByRole('button',{name:'预览资料包'}).click()
  await expect(panel.getByText('将复制 1 部影片')).toBeVisible()
  await panel.getByRole('button',{name:'确认导出副本'}).click()
  await expect(panel.getByRole('status')).toContainText('已导出 1 部影片')
  const restore=page.getByRole('region',{name:'资料包导回'})
  await restore.getByRole('button',{name:'从资料包导回'}).click()
  await restore.getByRole('button',{name:'选择资料包文件夹'}).click()
  await restore.getByRole('button',{name:'预览导回'}).click()
  await expect(restore.getByText('0 部可导回')).toBeVisible()
  await expect(restore.getByText(/已一致或按规则保留/)).toBeVisible()
})

test('manual selections persist across pages, and whole-library resolution reads beyond 500',async ({page})=>{
  await seedLibrary(page)
  await overrideApi(page,`
    const initialList=api.libraryList
    api.libraryList=async q=>{
      if(![1,50,500].includes(q.limit))return initialList(q)
      const size=Math.min(q.limit,Math.max(0,511-(q.offset??0)))
      return {total:511,items:Array.from({length:size},(_,n)=>({
        code:'TEST-'+String(n+1+(q.offset??0)).padStart(4,'0'),title:'测试影片'
      }))}
    }
    api.pickDirectory=async()=>'/mock/copies'
    api.packagePreview=async codes=>({
      token:'scope',destination:'/mock/copies/kiroku-test',movies:codes.length,files:codes.length,
      bytes:codes.length,examples:codes.slice(0,2),warnings:[],warningCount:0
    })
  `)
  await page.goto('/')
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  const panel=page.getByRole('region',{name:'导出资料包',exact:true})
  await panel.getByRole('button',{name:'导出资料包',exact:true}).click()
  await panel.getByRole('combobox').selectOption('selection')
  await panel.getByRole('checkbox',{name:/TEST-0001 /}).check()
  await panel.getByRole('button',{name:'下一页',exact:true}).click()
  await panel.getByRole('checkbox',{name:/TEST-0051 /}).check()
  await panel.getByRole('button',{name:'上一页',exact:true}).click()
  await expect(panel.getByRole('checkbox',{name:/TEST-0001 /})).toBeChecked()
  await panel.getByRole('button',{name:'选择保存位置'}).click()
  await panel.getByRole('button',{name:'预览资料包'}).click()
  await expect(panel.getByText('将复制 2 部影片')).toBeVisible()
  await panel.getByRole('button',{name:'返回修改'}).click()
  await panel.getByRole('combobox').selectOption('library')
  await panel.getByRole('button',{name:'预览资料包'}).click()
  await expect(panel.getByText('将复制 511 部影片')).toBeVisible()
})

test('a stalled scope read can be cancelled before it ever starts a package job',async({page})=>{
  await seedLibrary(page)
  await overrideApi(page,`
    const initialList=api.libraryList
    api.libraryList=q=>q.limit===500?new Promise(()=>{}):initialList(q)
    api.pickDirectory=async()=>'/mock/copies'
  `)
  await page.goto('/')
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  const panel=page.getByRole('region',{name:'导出资料包',exact:true})
  await panel.getByRole('button',{name:'导出资料包',exact:true}).click()
  await panel.getByRole('button',{name:'选择保存位置'}).click()
  await panel.getByRole('button',{name:'预览资料包'}).click()
  await panel.getByRole('button',{name:'停止任务'}).click()
  await expect(panel.getByRole('alert')).toContainText('已停止预览')
  await expect(panel.getByRole('button',{name:'预览资料包'})).toBeEnabled()
})
