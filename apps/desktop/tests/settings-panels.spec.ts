import {test,expect} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'
test('source results and maintenance settings align and fit narrow windows',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`api.sourcesCheck=async()=>['javdb','javbus','jav321'].map((sourceId,i)=>({sourceId,elapsedMs:500+i*100,checkedAt:1794752800,outcome:{reason:i?'ok':'authenticated'}}));`)
 await page.goto('/')
 await page.getByRole('button',{name:'设置',exact:true}).click()
 await page.getByRole('tab',{name:'数据源',exact:true}).click()
 await page.getByRole('button',{name:'检测已用来源'}).click()
 await expect(page.getByRole('button',{name:'重测 JavDB',exact:true})).toBeVisible()
 const buttons=page.locator('.result-row button')
 const positions=await buttons.evaluateAll(els=>els.map(el=>el.getBoundingClientRect().right))
 expect(Math.max(...positions)-Math.min(...positions)).toBeLessThan(1)
 await page.screenshot({path:'/tmp/kiroku-source-panels.png'})
 for(const width of [390,720,960,1280]){
  await page.setViewportSize({width,height:860})
  expect(await page.locator('.view-shell').evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true)
 }
 await page.getByRole('tab',{name:'高级',exact:true}).click()
 await page.setViewportSize({width:1280,height:860})
 await page.screenshot({path:'/tmp/kiroku-advanced-panels.png'})
})
