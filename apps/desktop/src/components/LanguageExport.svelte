<script lang="ts">
 import { onDestroy } from 'svelte'
 import { api } from '../lib/api'
 import { app } from '../lib/state.svelte'
 import type { Metadata, NfoLanguage, RevisionPreview } from '../lib/types'
 let {meta,onsaved}:{meta:Metadata;onsaved:()=>void}=$props()
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
</script>
<section class:expanded={opened} aria-label="NFO 语言导出">
 <button class="btn" type="button" aria-expanded={opened} disabled={busy} onclick={()=>opened=!opened}>语言与 NFO</button>
 <div class="export-body" hidden={!opened}>
 <p class="muted">原文标题：{meta.text_versions?.original.title??meta.original_title??'原文未保存'}<br/>原文简介：{meta.text_versions?.original.plot??'原文未保存'}</p>
 {#each Object.entries(meta.text_versions?.localized??{}) as [tag,text]}<div><strong>{tag} 已保存文本</strong><p>{text.title??'标题未保存'}</p><p>{text.plot??'简介未保存'}</p><button class="btn" disabled={busy} onclick={()=>selectSaved(tag)}>预览选择 {tag} 为当前文本</button></div>{/each}
 <button class="btn" aria-expanded={editVersions} onclick={()=>editVersions=!editVersions}>保存已有语言文本</button>{#if editVersions}<div><p>这里保存您提供的文本，AI 汉化仍只生成 zh-Hans；勾选字段后空白也会作为明确版本保存。</p><label>语言标签 <input aria-label="文本语言标签" bind:value={languageTag}/></label><label><input type="checkbox" bind:checked={includeTitle}/>保存标题 <input aria-label="语言版本标题" bind:value={versionTitle}/></label><label><input type="checkbox" bind:checked={includePlot}/>保存简介 <textarea aria-label="语言版本简介" bind:value={versionPlot}></textarea></label><label><input type="checkbox" bind:checked={selectVersion}/>同时选为当前文本（锁定字段优先）</label><button class="btn" disabled={busy} onclick={inspectText}>预览保存语言版本</button></div>{/if}
 <label>导出语言 <select aria-label="NFO 导出语言" bind:value={language} disabled={busy}><option value="current">当前文本</option><option value="zh-Hans">简体中文</option><option value="original">来源原文</option></select></label>
 <button class="btn" disabled={busy} onclick={inspect}>预览 NFO 导出</button>
 {#if preview}<div aria-label="NFO 语言差异">{#each preview.changes as change}<p>{change.field}：<del>{String(change.before??'（空）')}</del> → {String(change.after??'（空）')}</p>{/each}{#each preview.warnings??[] as warning}<p role="status">{warning}</p>{/each}<p>只更新 NFO 与文本版本记录，影片路径保持原位。人工锁定字段（包括空值）优先。</p><button class="btn primary" disabled={busy} onclick={save}>确认导出 NFO</button></div>{/if}
 {#if journalId}<button class="btn" disabled={busy} onclick={undo}>撤销 NFO 导出</button>{/if}
 {#if error}<p role="alert">{error}</p>{/if}
</div>
</section>

<style>
 .export-body { margin-top: var(--s3); }
 p { font-size: var(--fs-sm); overflow-wrap: anywhere; }
 label { display: flex; flex-wrap: wrap; gap: var(--s2); align-items: center; margin: var(--s3) 0; }
 input:not([type=checkbox]), select, textarea { min-width: 0; max-width: 100%; }
 textarea { width: 100%; }
 button { margin: var(--s1) var(--s1) var(--s1) 0; }
</style>
