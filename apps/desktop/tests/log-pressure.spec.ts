import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'

test('large live log keeps rendering bounded and allows reviewing older messages',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`
 const s=await api.getSettings();s.source_dirs=['/mock/source'];s.log.app=true;await api.saveSettings(s);
 const scrape=api.scrapeStart;api.scrapeStart=ids=>scrape(ids.filter(id=>id===3));
 const {emitMock}=await import('/src/lib/events.ts');
 api.applyStart=async()=>{emitMock('apply:progress',{jobId:700,phase:'images',done:0,total:5000,failed:0,code:null});return 700};
 window.logBurst=(start,count)=>{for(let i=start;i<start+count;i++)emitMock('log:line',{jobId:700,time:'00:00:00.000',level:i%10===0?'warn':'info',target:'organize',message:'entry-'+i+' '+('/example/neutral-folder/'.repeat(30))})};
 `)
 await page.goto('/')
 await page.getByRole('button',{name:'导入整理',exact:true}).click()
 await page.getByRole('button',{name:'开始扫描',exact:true}).click()
 await page.getByRole('button',{name:/查找影片信息/}).click()
 await page.getByRole('button',{name:'预览整理',exact:true}).click()
 await page.getByRole('button',{name:/确认整理/}).click()
 await expect(page.getByRole('heading',{name:'正在下载图片',exact:true})).toBeVisible()
 await page.evaluate(()=>(window as any).logBurst(0,3000))
 await expect(page.getByRole('log').locator('.line')).toHaveCount(200)
 await expect(page.getByRole('log')).toContainText('entry-2999 ')
 await page.getByRole('button',{name:'较早日志',exact:true}).click()
 await expect(page.getByRole('log')).toContainText('entry-2799 ')
 await page.getByRole('button',{name:'最新日志',exact:true}).click()
 await page.evaluate(()=>(window as any).logBurst(3000,10))
 await expect(page.getByRole('log')).toContainText('entry-3009 ')
 await expect(page.getByRole('log').locator('.line')).toHaveCount(200)
 for (let start = 3010; start < 6010; start += 100) {
   await page.evaluate((first) => (window as any).logBurst(first, 100), start)
   await expect(page.getByRole('log')).toContainText(`entry-${start + 99} `)
   await expect(page.getByRole('log').locator('.line')).toHaveCount(200)
 }
 await page.getByRole('checkbox',{name:'只看问题'}).check()
 await expect(page.getByRole('log').locator('.line')).toHaveCount(200)
 await page.locator('details.log > summary').click({position:{x:20,y:10}})
 await expect(page.getByRole('log').locator('.line')).toHaveCount(0)
 await page.getByRole('button',{name:'媒体库',exact:true}).click()
 await expect(page.getByRole('heading',{name:'媒体库',exact:true})).toBeVisible()
})
