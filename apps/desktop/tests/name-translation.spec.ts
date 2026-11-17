import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'

async function setup(page:import('@playwright/test').Page, enabled=true){
 await seedLibrary(page)
 if(enabled)await overrideApi(page,`const get=api.getSettings.bind(api);api.getSettings=async()=>{const s=await get();s.ai.enabled=true;s.ai.providers[0].api_key='synthetic';await api.saveSettings(s);return s}`)
 await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.getByRole('tab',{name:'名称规则',exact:true}).click()
 await page.getByLabel('名称类别').selectOption('series')
}

test('series candidate is editable and saved only after accepting a rule',async({page})=>{
 await setup(page)
 await page.getByLabel('待汉化原名').fill('海辺の町')
 await page.getByRole('button',{name:'生成中文候选',exact:true}).click()
 await expect(page.getByLabel('规范名',{exact:true})).toHaveValue('示例系列译名')
 await expect(page.getByLabel('别名（每行一个）')).toHaveValue('海辺の町')
 expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!).name_rules.series)).toEqual([])
 await page.getByLabel('规范名',{exact:true}).fill('海边小镇')
 await page.getByRole('button',{name:'添加规则',exact:true}).click()
 await page.getByRole('button',{name:/保存更改/}).click()
 await expect.poll(()=>page.evaluate(()=>JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!).name_rules.series)).toEqual([{canonical:'海边小镇',aliases:['海辺の町']}])
 await page.getByRole('button',{name:'生成中文候选',exact:true}).click()
 await expect(page.getByRole('alert')).toContainText('已有规则')
})

test('unknown tags require configured AI without changing the rules',async({page})=>{
 await setup(page,false)
 await page.getByLabel('名称类别').selectOption('tags')
 await page.getByLabel('待汉化原名').fill('サンプル分類XYZ')
 await page.getByRole('button',{name:'生成中文候选',exact:true}).click()
 await expect(page.getByRole('alert')).toContainText('启用 AI')
 await expect(page.getByLabel('规范名',{exact:true})).toHaveValue('')
})

test('inventory exposes unmapped names and generated tags can be accepted',async({page})=>{
 await setup(page)
 await page.getByRole('button',{name:'读取库名称',exact:true}).click()
 await expect(page.getByLabel('库中未建规则的名称')).toBeVisible()
 await page.getByLabel('名称类别').selectOption('tags')
 await page.getByLabel('待汉化原名').fill('サンプル分類XYZ')
 await page.getByRole('button',{name:'生成中文候选',exact:true}).click()
 await expect(page.getByLabel('规范名',{exact:true})).toHaveValue('示例标签译名')
 await page.getByRole('button',{name:'添加规则',exact:true}).click()
 await expect(page.locator('.rule').filter({hasText:'示例标签译名'})).toContainText('サンプル分類XYZ')
})
