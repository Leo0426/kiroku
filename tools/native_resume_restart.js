window.addEventListener('DOMContentLoaded',()=>{
 const report=message=>window.__TAURI_INTERNALS__.invoke('restart_report',{message});
 window.addEventListener('error',e=>report('ERROR: '+e.message));
 window.addEventListener('unhandledrejection',e=>report('ERROR: '+String(e.reason)));
 const wait=async(predicate,label)=>{const until=Date.now()+12000;while(!predicate()){if(Date.now()>until)throw new Error('Timed out: '+label+'; body='+document.body.innerText.slice(0,1600));await new Promise(r=>setTimeout(r,30))}};
 const button=text=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text&&!b.disabled);
 const click=async text=>{await wait(()=>button(text),text);const b=button(text);b.scrollIntoView({block:'center'});b.click()};
 const text=()=>document.body.innerText;
 (async()=>{
  await click('保留成果后继续');await click('核对并继续');
  await click('开始扫描');await click('查找影片信息（2 部）');
  await wait(()=>text().includes('影片信息查询完成'),'cached scrape');
  if(!text().includes('复用 2'))throw new Error('Source cache was not reused after restart');
  await click('预览整理');await click('确认整理（2 个文件）');
  await wait(()=>text().includes('整理完成'),'remaining apply');
  await click('撤销本批');await click('确认撤销');await wait(()=>text().includes('本批已撤销'),'new batch undo');
  await click('查看恢复事项');await click('检查并撤销');await click('确认撤销');
  await wait(()=>!document.querySelector('dialog[open][aria-labelledby="recovery-title"]'),'original batch undo');
  await report('DONE');
 })().catch(e=>report('ERROR: '+String(e)));
});
