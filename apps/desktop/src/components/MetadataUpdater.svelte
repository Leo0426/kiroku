<script lang="ts">
 import { onDestroy } from 'svelte'
 import { api } from '../lib/api'
 import { app } from '../lib/state.svelte'
 import { EDITABLE_FIELDS, FIELDS, type Metadata, type MetadataUpdate, type SourceCandidate, type RevisionPreview } from '../lib/types'
 let {meta,onsaved}:{meta:Metadata;onsaved:()=>void}=$props()
 let opened=$state(false),busy=$state(false),error=$state(''),replace=$state(false),force=$state(false),url=$state('')
 let selected=$state<string[]>(EDITABLE_FIELDS.map(f=>f.key)),sources=$state<Record<string,string>>({}),preview=$state<RevisionPreview|null>(null)
 let candidates=$state<Record<string,SourceCandidate[]>>({})
 let generation=0
 onDestroy(()=>{generation++})
 $effect(()=>{meta;app.settings;generation++;opened=false;preview=null;error='';busy=false;candidates={};sources={}})
 function text(value:unknown):string { if(value==null)return '（空）';if(Array.isArray(value))return value.map(v=>typeof v==='string'?v:('path' in v ? `${v.path} → ${v.source}（${v.size} 字节，来源校验 ${String(v.source_proof).slice(-12)}）` : v.name)).join('、')||'（空）';return String(value) }
 async function start(){
  opened=true;error='';preview=null;sources={};candidates={};const current=generation;busy=true
  try{const next=await api.metadataSourceCandidates(meta.code);if(current===generation)candidates=next}
  catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}
 }
 async function inspect(update:MetadataUpdate){
  const current=generation;busy=true;error=''
  try{const next=await api.metadataUpdatePreview(meta.code,update);if(current===generation)preview=next}
  catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}
 }
 async function save(){
  if(!preview)return
  const current=generation,candidate=preview;busy=true;error=''
  try{const saved=await api.revisionSave(candidate.token,candidate.root,candidate.code);if(current!==generation)return;preview=null;
   if(saved.error||saved.indexError)error=[saved.error,saved.indexError&&`索引更新失败：${saved.indexError}`].filter(Boolean).join('；')
   else{opened=false;app.toast('ok','更新已保存，可从历史撤销');onsaved()}
  }catch(e){if(current===generation){error=String(e);preview=null}}finally{if(current===generation)busy=false}
 }
</script>
<section class:expanded={opened} aria-label="元数据更新">
 {#if !opened}<button class="btn" onclick={start}>更新与来源</button>{:else}
 <h3>更新与来源</h3>
 {#if preview}
 <div aria-label="更新差异预览">
 {#each preview.changes as change}<div class="change"><strong>{FIELDS.find(f=>f.key===change.field)?.label??(change.field==='placements'?'媒体链接来源记录':change.field)} · 来源：{preview.after.provenance?.[change.field]??'未知'}</strong><del>{text(change.before)}</del><ins>{text(change.after)}</ins></div>{/each}
 {#each FIELDS.filter(f=>Boolean(preview!.before.locked_fields?.includes(f.key))!==Boolean(preview!.after.locked_fields?.includes(f.key))) as field}<p>{field.label}：{preview.after.locked_fields?.includes(field.key)?'锁定（包括空值）':'解锁'}</p>{/each}
 {#each EDITABLE_FIELDS.filter(f=>preview!.before.provenance?.[f.key]!==preview!.after.provenance?.[f.key]) as field}<p>{field.label}来源：{preview.before.provenance?.[field.key]??'未知'} → {preview.after.provenance?.[field.key]??'未知'}</p>{/each}
 <p>原始来源缓存将随本次确认保存；未列出的字段保持原值。</p>
 <button class="btn primary" disabled={busy} onclick={save}>确认更新</button><button class="btn" disabled={busy} onclick={()=>preview=null}>返回更新</button>
 </div>
 {:else}
 <p class="muted">默认补充空字段，复用匹配的原站缓存，只联网补齐缺失快照。替换已有值需要确认差异；锁定字段始终保留，包括空值。</p>
 <fieldset disabled={busy}><legend>所选字段</legend>
 {#each EDITABLE_FIELDS as field}<label class="inline"><input aria-label="更新字段" type="checkbox" value={field.key} bind:group={selected} />{field.label}{meta.locked_fields?.includes(field.key)?'（已锁定）':''}</label>{/each}
 </fieldset>
 <label class="inline"><input type="checkbox" bind:checked={replace} disabled={busy}/>替换已有值</label>
 <label class="inline"><input type="checkbox" bind:checked={force} disabled={busy||!!url.trim()}/>强制刷新原站缓存（重新联网）</label>
 <label class="urlrow">详情页地址（可选）<input class="input" type="url" placeholder="https://javdb.com/v/…" bind:value={url} disabled={busy}/></label>
 <p class="muted">填写后，该站使用这一页的结果（搜索找错时使用），路线上的其他站点照常查询。</p>
 <button class="btn primary" disabled={busy} onclick={()=>inspect({kind:'refresh',fields:selected,replace,force,...(url.trim()?{url:url.trim()}:{})})}>{busy?'更新中…':'预览更新'}</button>
 {#if busy}<button class="btn" onclick={()=>api.jobCancel()}>取消更新任务</button>{/if}
 {#if Object.keys(meta.raw_sources??{}).length}<div aria-label="原站缓存">{#each Object.entries(meta.raw_sources??{}) as [site,snapshot]}<p>{site} · {snapshot.base} · 采集于 {new Date(snapshot.fetched_at*1000).toLocaleString()}</p>{/each}</div>{/if}
 <p class="muted">来源列表仅含当前设置下有效且非空的候选；过期缓存需重新抓取。</p>
 {#if meta.placements?.some(p=>p.mode==='symlink')}<p>媒体软链接依赖来源文件。来源恢复或经确认修改后，可预览更新身份记录；被替换的链接仍会拒绝登记。</p><button class="btn" disabled={busy} onclick={()=>inspect({kind:'revalidateLinks'})}>重新验证媒体链接</button>{/if}
 <h4>锁定与来源候选</h4>
 {#each FIELDS as field}
 <div class="field"><strong>{field.label}</strong><button class="btn sm" disabled={busy} onclick={()=>inspect({kind:'lock',field:field.key,locked:!meta.locked_fields?.includes(field.key)})}>{meta.locked_fields?.includes(field.key)?'解锁':'锁定'}{field.label}</button>
 {#if EDITABLE_FIELDS.some(f=>f.key===field.key)}
 <select aria-label={`${field.label}来源`} bind:value={sources[field.key]} disabled={busy}><option value="">选择原始来源</option>{#each candidates[field.key]??[] as candidate}<option value={candidate.source}>{candidate.source} · {new Date(candidate.fetchedAt*1000).toLocaleString()} · {text(candidate.value)}</option>{/each}</select>
 <button class="btn sm" disabled={busy||!sources[field.key]} onclick={()=>inspect({kind:'source',field:field.key,source:sources[field.key]})}>预览{field.label}来源</button>
 {/if}</div>
 {/each}
 {#if !Object.keys(meta.raw_sources??{}).length}<p>尚无原始来源快照，请先预览更新并确认保存缓存。</p>{/if}
 {/if}
 <button class="btn ghost" disabled={busy} onclick={()=>{opened=false;preview=null}}>取消更新</button>
 {/if}
 {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>.inline{display:inline-flex;gap:6px;align-items:center;margin:6px}.field{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:12px 0}.field strong{min-width:70px}select{max-width:100%}.change{display:flex;flex-direction:column;gap:6px;margin:16px 0}del{color:var(--text-3);white-space:pre-wrap}ins{text-decoration:none;color:var(--accent);white-space:pre-wrap} .urlrow { display:flex; flex-direction:column; gap:4px; }
</style>
