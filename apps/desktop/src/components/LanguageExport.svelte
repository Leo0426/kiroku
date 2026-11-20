<script lang="ts">
  import Icon from './Icon.svelte'
 import { onDestroy } from 'svelte'
 import { api } from '../lib/api'
 import { app } from '../lib/state.svelte'
 import type { Metadata, NfoLanguage, RevisionPreview } from '../lib/types'
 let {meta,onsaved,onactive}:{meta:Metadata;onsaved:()=>void;onactive?:(active:boolean)=>void}=$props()
 let language=$state<NfoLanguage>('current'),preview=$state<RevisionPreview|null>(null),error=$state(''),busy=$state(false),journalId=$state<string|null>(null)
 let opened=$state(false)
 let editVersions=$state(false)
 let languageTag=$state('zh-Hans'),versionTitle=$state(''),versionPlot=$state(''),includeTitle=$state(true),includePlot=$state(true),selectVersion=$state(false)
 let identity=''
 let generation=0;onDestroy(()=>generation++)
 $effect(()=>{const next=`${meta.code}|${app.settings?.library_root??''}`;if(next!==identity){identity=next;generation++;preview=null;error='';journalId=null;language=meta.nfo_language??'current'}})
 async function inspect(){const current=generation;busy=true;error='';try{const p=await api.metadataUpdatePreview(meta.code,{kind:'nfoLanguage',language});if(current===generation)preview=p}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
 async function save(){if(!preview)return;const p=preview,current=generation;busy=true;error='';try{const result=await api.revisionSave(p.token,p.root,p.code);if(current!==generation)return;if(result.error||result.indexError)throw [result.error,result.indexError].filter(Boolean).join('；');journalId=result.journalId;preview=null;onsaved();app.toast('ok','NFO 已导出，可撤销')}catch(e){if(current===generation){error=String(e);preview=null}}finally{if(current===generation)busy=false}}
 async function undo(){if(!journalId)return;const current=generation;busy=true;error='';try{const result=await api.rollback(journalId);if(result.failures.length||result.indexError)throw [...result.failures,result.indexError].filter(Boolean).join('；');if(current===generation){journalId=null;onsaved()}}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
 async function inspectText(){const current=generation;busy=true;error='';try{const p=await api.metadataUpdatePreview(meta.code,{kind:'textVersion',language:languageTag,text:{...(includeTitle?{title:versionTitle}:{}),...(includePlot?{plot:versionPlot}:{})},select:selectVersion});if(current===generation)preview=p}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
 async function selectSaved(tag:string){const current=generation;busy=true;error='';try{const p=await api.metadataUpdatePreview(meta.code,{kind:'textVersion',language:tag,text:meta.text_versions?.localized[tag]??{},select:true});if(current===generation)preview=p}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
  $effect(() => onactive?.(opened))
</script>
<section class:expanded={opened} aria-label="NFO 语言导出">
 <button class="btn" type="button" aria-expanded={opened} disabled={busy} title={opened ? '收起（保留当前填写内容）' : '展开'} onclick={()=>opened=!opened}><Icon name="globe" size={15}/>语言与 NFO<Icon name="chevron-down" size={16}/></button>
 <div class="tool-content" hidden={!opened}>
 <div class="tool-fields">
  <div class="export-group">
   <h3>影片文本</h3>
   <p class="muted">原文标题：{meta.text_versions?.original.title??meta.original_title??'原文未保存'}<br/>原文简介：{meta.text_versions?.original.plot??'原文未保存'}</p>
   {#each Object.entries(meta.text_versions?.localized??{}) as [tag,text]}
    <div class="saved-text"><strong>{tag} 已保存文本</strong><p>{text.title??'标题未保存'}</p><p>{text.plot??'简介未保存'}</p><div class="tool-actions start"><button class="btn" disabled={busy} onclick={()=>selectSaved(tag)}>预览选择 {tag} 为当前文本</button></div></div>
   {/each}
   <div class="tool-actions start"><button class="btn" aria-expanded={editVersions} disabled={busy} onclick={()=>editVersions=!editVersions}>保存已有语言文本</button></div>
  </div>
  <div class="export-group">
   <h3>NFO 导出</h3>
   <p class="muted">将所选语言的文本写回这部影片目录中的 NFO。</p>
   <label class="tool-field">导出语言<select class="input" aria-label="NFO 导出语言" bind:value={language} disabled={busy}><option value="current">当前文本</option><option value="zh-Hans">简体中文</option><option value="original">来源原文</option></select></label>
   <div class="tool-actions start">
    <button class="btn" disabled={busy} onclick={inspect}>预览 NFO 导出</button>
    {#if journalId}<button class="btn" disabled={busy} onclick={undo}>撤销 NFO 导出</button>{/if}
   </div>
  </div>
 </div>
 {#if editVersions}
  <fieldset disabled={busy}>
   <legend>语言版本</legend>
   <p>勾选的字段会保存为此语言的版本，支持保存空值。AI 汉化生成简体中文。</p>
   <div class="tool-fields">
    <label class="tool-field">语言标签<input class="input" aria-label="文本语言标签" bind:value={languageTag}/></label>
    <label class="check"><input class="checkbox" type="checkbox" bind:checked={selectVersion}/>同时选为当前文本（锁定字段优先）</label>
    <div class="tool-field wide">
     <label class="check"><input class="checkbox" type="checkbox" bind:checked={includeTitle}/>保存标题</label>
     <input class="input" aria-label="语言版本标题" bind:value={versionTitle}/>
    </div>
    <div class="tool-field wide">
     <label class="check"><input class="checkbox" type="checkbox" bind:checked={includePlot}/>保存简介</label>
     <textarea class="input" aria-label="语言版本简介" bind:value={versionPlot} rows="3"></textarea>
    </div>
   </div>
   <div class="tool-actions"><button class="btn" onclick={inspectText}>预览保存语言版本</button></div>
  </fieldset>
 {/if}
 {#if preview}
  <div aria-label="NFO 语言差异">
   <p>写入位置 <span class="mono">{preview.nfoPath??preview.dir}</span></p>
   {#each preview.changes as change}<p>{change.field}：<del>{String(change.before??'（空）')}</del> → {String(change.after??'（空）')}</p>{/each}
   {#each preview.warnings??[] as warning}<p role="status">{warning}</p>{/each}
   <p>只更新 NFO 与文本版本记录，影片路径保持原位。人工锁定字段（包括空值）优先。</p>
   <div class="tool-actions"><button class="btn primary" disabled={busy} onclick={save}>确认导出 NFO</button></div>
  </div>
 {/if}
 {#if error}<p role="alert">{error}</p>{/if}
</div>
</section>

<style>
 p { font-size: var(--fs-sm); overflow-wrap: anywhere; }
 h3 { margin-bottom: var(--s2); font-size: var(--fs-sm); }
 .export-group, fieldset { min-width: 0; padding: var(--s3); border: 1px solid var(--line); border-radius: var(--r-sm); }
 fieldset { margin: var(--s4) 0 0; }
 legend { padding-inline: var(--s2); }
 fieldset > p, .saved-text { margin-block: var(--s2) var(--s3); }
 .check { display: flex; gap: var(--s2); align-items: center; }
</style>
