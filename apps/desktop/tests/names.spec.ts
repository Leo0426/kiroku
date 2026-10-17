import {expect,test} from '@playwright/test'
import {seedLibrary} from './helpers'
test('name rules apply only after saved preview and can be undone',async({page})=>{
 await seedLibrary(page);await page.goto('/');
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click();
 await page.getByRole('tab',{name:'名称规则',exact:true}).click();
 await page.getByRole('button',{name:'读取库名称',exact:true}).click();
 const row=page.locator('[data-catalog-movie]').first();await row.getByRole('checkbox').check();
 const alias=await row.locator('[data-actors]').innerText();
 await page.getByLabel('规范名',{exact:true}).fill('统一演员');await page.getByLabel('别名（每行一个）').fill(alias.split('、')[0]);
 await page.getByRole('button',{name:'添加规则',exact:true}).click();
 await page.getByRole('button',{name:/保存更改/}).click();
 await page.getByRole('button',{name:'预览所选影片',exact:true}).click();
 await expect(page.getByText('原有影片位置保持不变')).toBeVisible();await expect(page.locator('[data-name-diff]').first()).toContainText('统一演员');
 await page.getByRole('button',{name:'应用名称规则',exact:true}).click();
 await expect(page.getByText('已完成 1 项')).toBeVisible();
 await page.getByRole('button',{name:'撤销本次修改',exact:true}).click();await expect(page.getByText('已撤销本次修改')).toBeVisible();
})
test('ambiguous aliases do not save',async({page})=>{
 await seedLibrary(page);await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click();await page.getByRole('tab',{name:'名称规则',exact:true}).click();
 await page.getByLabel('规范名',{exact:true}).fill('甲');await page.getByLabel('别名（每行一个）').fill('同名');await page.getByRole('button',{name:'添加规则',exact:true}).click();
 await page.getByLabel('规范名',{exact:true}).fill('乙');await page.getByLabel('别名（每行一个）').fill('同名');await page.getByRole('button',{name:'添加规则',exact:true}).click();
 await expect(page.getByRole('alert')).toContainText('别名');
})
test('imported canonical actors keep original source names available in the UI',async({page})=>{
 await seedLibrary(page)
 await page.addInitScript(()=>localStorage.setItem('kiroku-mock-settings-v2',JSON.stringify({library_root:'/mock/library',source_dirs:['/mock/source'],name_rules:{actors:[{canonical:'统一演员',aliases:['演员甲','演员乙','演员丙','演员丁','演员戊','演员己','演员庚','演员辛','演员壬','演员癸','演员子','演员丑','演员寅','演员卯']}],tags:[],studio:[],publisher:[],series:[]}})))
 await page.route('**/src/lib/api.ts*',async route=>{const response=await route.fetch();await route.fulfill({response,body:(await response.text())+'\nconst originalScrape=api.scrapeStart;api.scrapeStart=async ids=>originalScrape(ids.filter(id=>id===0));'})})
 await page.goto('/');await page.getByRole('button',{name:'导入整理',exact:true}).click();await page.getByRole('button',{name:'开始扫描',exact:true}).click();await page.getByRole('button',{name:/查找影片信息/}).click();await page.getByRole('button',{name:'预览整理',exact:true}).click();await page.getByRole('button',{name:/确认整理/}).click();await page.getByRole('button',{name:'查看媒体库',exact:true}).click()
 await page.locator('button.pcard').filter({hasText:'SMPL-421'}).click();const panel=page.getByLabel('影片详情',{exact:true});await expect(panel.locator('.chips').first()).toContainText('统一演员');await panel.getByRole('button',{name:'更新与来源',exact:true}).click();const source=panel.getByLabel('演员来源', {exact:true});await expect(source.locator('option')).toHaveCount(4);await expect(source).not.toContainText('统一演员')
})
test('failed library read clears stale name counts and movie selection',async({page})=>{
 await seedLibrary(page)
 await page.route('**/src/lib/api.ts*',async route=>{const response=await route.fetch();await route.fulfill({response,body:(await response.text())+'\nlet reads=0;const oldInventory=api.catalogInventory;api.catalogInventory=async()=>{if(++reads>1)throw new Error("媒体库不可用");return oldInventory();};'})})
 await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click();await page.getByRole('tab',{name:'名称规则',exact:true}).click();await page.getByRole('button',{name:'读取库名称',exact:true}).click();await expect(page.locator('[data-catalog-movie]').first()).toBeVisible();await page.getByRole('button',{name:'读取库名称',exact:true}).click();await expect(page.getByRole('alert')).toContainText('媒体库不可用');await expect(page.locator('[data-catalog-movie]')).toHaveCount(0)
})
test('stripping actor names from titles is previewed as a title change',async({page})=>{
 await seedLibrary(page)
 await page.route('**/src/lib/api.ts*',async route=>{const response=await route.fetch();await route.fulfill({response,body:(await response.text())+'\nconst oldGet=api.libraryGet;api.libraryGet=async code=>{const d=await oldGet(code);if(d&&d.meta.actors[0])d.meta.title=`温泉旅行 ${d.meta.actors[0].name}`;return d};'})})
 await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click();await page.getByRole('tab',{name:'名称规则',exact:true}).click()
 await page.getByRole('checkbox',{name:/标题剔除演员名/}).check()
 await page.getByRole('button',{name:/保存更改/}).click()
 await page.getByRole('button',{name:'读取库名称',exact:true}).click();await page.locator('[data-catalog-movie]').first().getByRole('checkbox').check()
 await page.getByRole('button',{name:'预览所选影片',exact:true}).click()
 await expect(page.locator('[data-name-diff]').first()).toContainText('title：')
 await expect(page.locator('[data-name-diff]').first()).toContainText('→ "温泉旅行"')
})
