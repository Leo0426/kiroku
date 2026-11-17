<script lang="ts">
  import Icon from './Icon.svelte'
 import {onDestroy,untrack} from 'svelte'
 import {api} from '../lib/api'
 import {app} from '../lib/state.svelte'
 import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
 import type {Metadata,RevisionPreview} from '../lib/types'
 let {meta,onsaved,onactive}:{meta:Metadata;onsaved:()=>void;onactive?:(active:boolean)=>void}=$props()
 let opened=$state(false),busy=$state(false),error=$state(''),message=$state('')
 let preview=$state<RevisionPreview|null>(null)
 let journal=$state<string|null>(null)
 let links=$state<string[]>([])
 let task=$state<CancellableJob<RevisionPreview>|null>(null)
 let generation=0, identity=''
 onDestroy(()=>{generation++;void task?.cancel()})
 $effect(()=>{const next=JSON.stringify([meta.code,app.settings]);if(next===identity)return;identity=next;untrack(()=>{generation++;void task?.cancel();task=null;preview=null;busy=false;opened=false;error='';message='';journal=null;links=[]})})
 $effect(()=>onactive?.(opened||busy))
 async function inspect(){
  const g=generation;busy=true;error='';message='';preview=null
  const request=cancellableJob(id=>api.metadataUpdatePreview(meta.code,{kind:'actorImages'},id),e=>{if(g===generation)error=String(e)});task=request
  try{const next=await request.promise;if(g===generation&&!request.cancelled)preview=next}catch(e){if(g===generation)error=String(e)}finally{if(g===generation){busy=false;task=null}}
 }
 async function save(){
  if(!preview)return
  const p=preview,g=generation;busy=true;error=''
  try{const r=await api.revisionSave(p.token,p.root,p.code);if(g!==generation)return;preview=null;journal=r.journalId;if(r.error||r.indexError)throw [r.error,r.indexError].filter(Boolean).join('；');links=[...new Set(Object.values(p.after.hosted_actor_images??{}).map(v=>v.url))];message='头像与 NFO 已保存。请检查头像链接，再在 Plex 中刷新本片元数据。';onsaved()}
  catch(e){if(g===generation){preview=null;error=String(e)}}finally{if(g===generation)busy=false}
 }
 async function undo(){if(!journal)return;const g=generation;busy=true;error='';try{const r=await api.rollback(journal);if(g!==generation)return;if(r.failures.length||r.indexError)throw [...r.failures,r.indexError].filter(Boolean).join('；');journal=null;links=[];message='NFO 和元数据已恢复，头像缓存保留。请在 Plex 刷新元数据。';onsaved()}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function openLink(url:string){try{await api.openSourceUrl(url)}catch(e){error=String(e)}}
</script>
<section class:expanded={opened} aria-label="演员头像发布">
 <button class="btn" aria-expanded={opened} disabled={busy} onclick={()=>opened=!opened}><Icon name="image" size={15}/>演员头像<Icon name="chevron-down" size={16}/></button>
 {#if opened}
 <div class="tool-content">
 <p>为 Plex NFO 发布可直接读取的头像地址，保留原站头像记录。</p>
 {#if !app.settings?.actor_hosting.enabled}<p>请先到设置 → 媒体服务器 → 演员头像托管，填写保存目录与 NAS 主机名 URL 前缀并保存。</p>{:else}
 <p class="path">保存目录：{app.settings.actor_hosting.directory}<br/>URL 前缀：{app.settings.actor_hosting.base_url}</p>
 <p>预览只下载到内存。确认后写入缓存及 NFO；没有头像的演员会跳过，下载失败时不保存。</p>
 <button class="btn" disabled={busy} onclick={inspect}>{busy?'处理中…':'预览头像发布'}</button>
 {#if busy&&task}<button class="btn" onclick={()=>task?.cancel()}>取消头像下载</button>{/if}
 {/if}
 {#if preview}
 <div aria-label="头像发布预览">
 {#each preview.after.actors as actor}{#if actor.thumb && preview.after.hosted_actor_images?.[actor.thumb]}<p class="path">{actor.name} → {preview.after.hosted_actor_images[actor.thumb].url}</p>{/if}{/each}
 {#each preview.warnings??[] as warning}<p>{warning}</p>{/each}
 <p>头像缓存可被多部影片共用，撤销时不会删除。请确保 URL 前缀对应保存目录。</p>
 <button class="btn primary" disabled={busy} onclick={save}>确认发布头像</button>
 <button class="btn" disabled={busy} onclick={()=>preview=null}>放弃预览</button>
 </div>
 {/if}
 {#if message}<p role="status">{message}</p>{/if}
 {#each links as url,i}<button class="btn sm" onclick={()=>openLink(url)}>打开头像链接 {i+1}</button>{/each}
 {#if journal}<button class="btn" disabled={busy} onclick={undo}>撤销头像 NFO 更新</button>{/if}
 {#if error}<p role="alert">{error}</p>{/if}
 </div>
 {/if}
</section>
<style>p{font-size:var(--fs-sm);margin:12px 0}.path{overflow-wrap:anywhere}button{margin:4px 4px 4px 0}</style>
