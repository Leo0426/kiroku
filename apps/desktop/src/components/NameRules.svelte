<script lang="ts">
 import {onDestroy} from 'svelte'
 import {api} from '../lib/api'
 import {listen} from '../lib/events'
 import {validateNames,NAME_FIELDS} from '../lib/names'
 import type {NameRules,NameField,CatalogInventory,CatalogPreview,CatalogResult} from '../lib/types'
 let {rules=$bindable(),dirty=false}:{rules:NameRules;dirty?:boolean}=$props()
 const labels:Record<NameField,string>={actors:'演员',tags:'标签',studio:'制作商',publisher:'发行商',series:'系列'}
 let field=$state<NameField>('actors'),canonical=$state(''),aliases=$state(''),error=$state(''),busy=$state(false),inventory=$state<CatalogInventory|null>(null),selected=$state<string[]>([]),preview=$state<CatalogPreview|null>(null),result=$state<CatalogResult|null>(null),message=$state(''),jobId=$state<number|null>(null),progress=$state(''),editIndex=$state<number|null>(null)
 let generation=0;onDestroy(()=>generation++)
 $effect(()=>{rules;preview=null})
 function add(){error='';try{const next=structuredClone($state.snapshot(rules));const rule={canonical:canonical.trim(),aliases:aliases.trim()?aliases.split('\n').map(s=>s.trim()):[]};if(editIndex===null)next[field].push(rule);else next[field][editIndex]=rule;validateNames(next);rules=next;canonical='';aliases='';editIndex=null;preview=null}catch(e){error=String(e)}}
 function remove(index:number){rules={...rules,[field]:rules[field].filter((_,i)=>i!==index)};preview=null}
 async function run(task:()=>Promise<void>){const g=generation;busy=true;error='';message='';jobId=null;const unlisten=await listen('library:catalog',ev=>{if(g===generation){jobId=ev.jobId;progress=ev.total?`${ev.done}/${ev.total} ${ev.code}`:''}});try{await task()}catch(e){if(g===generation)error=String(e)}finally{unlisten();if(g===generation){busy=false;jobId=null}}}
 async function preset(){error='';message='';try{const r=await api.tagPresetMerge($state.snapshot(rules));rules=r.rules;field='tags';editIndex=null;preview=null;message=r.added_rules||r.added_aliases?`已加入 ${r.added_rules} 条标签规则、${r.added_aliases} 个别名，保存设置后生效`:'内置标签译名已全部在规则里'}catch(e){error=String(e)}}
 async function load(){inventory=null;selected=[];preview=null;result=null;await run(async()=>{const next=await api.catalogInventory();inventory=next})}
 async function build(){await run(async()=>{preview=await api.catalogPreview(selected);result=null})}
 async function apply(){if(!preview)return;const token=preview.token;await run(async()=>{result=await api.catalogApply(token);preview=null})}
 async function undo(){if(!result)return;await run(async()=>{for(const item of [...result!.items].reverse()){if(item.journalId){const r=await api.rollback(item.journalId);if(r.failures.length)throw r.failures.join('；')}}result=null;message='已撤销本次修改'})}
</script>
<section class="card" aria-label="名称规则管理">
 <p>规则保存后用于新整理。已有影片需明确选择并预览；人工锁定字段优先。</p>
 <div class="preset"><button class="btn" onclick={preset} disabled={busy||editIndex!==null}>导入内置标签译名</button><span class="muted">把常见的繁体、日文标签统一成简体，不改动已有规则。</span></div>
 <label class="toggle"><input type="checkbox" checked={!!rules.strip_actor_names_from_title} disabled={busy} onchange={e=>{rules={...rules,strip_actor_names_from_title:e.currentTarget.checked};preview=null}} /><span>标题剔除演员名<small>在 AI 汉化之前，从标题里删掉本片演员的规范名和别名；原题保留来源原文。只有一个字的名字不处理，删完为空时保留原题。</small></span></label>
 <label>名称类别 <select class="input" bind:value={field} disabled={busy||editIndex!==null}>{#each NAME_FIELDS as key}<option value={key}>{labels[key]}</option>{/each}</select></label>
 <label>规范名 <input class="input" bind:value={canonical} disabled={busy} list="catalog-names" /></label>
 <datalist id="catalog-names">{#each Object.keys(inventory?.names[field]??{}) as name}<option value={name}></option>{/each}</datalist>
 <label>别名（每行一个） <textarea class="input" bind:value={aliases} disabled={busy}></textarea></label>
 <button class="btn" onclick={add} disabled={busy||!canonical.trim()}>{editIndex===null?'添加规则':'保存规则'}</button>{#if editIndex!==null}<button class="btn" onclick={()=>{editIndex=null;canonical='';aliases=''}}>取消编辑</button>{/if}
 {#each rules[field] as rule,index}<div class="rule"><strong>{rule.canonical}</strong><span>{rule.aliases.join('、')}</span><span class="muted">{inventory?`引用 ${[rule.canonical,...rule.aliases].reduce((sum,name)=>sum+(inventory?.names[field]?.[name]??0),0)} 次`:'引用数未读取'}</span><button class="btn sm" disabled={busy} onclick={()=>{canonical=rule.canonical;aliases=rule.aliases.join('\n');editIndex=index}}>编辑规则</button><button class="btn sm" disabled={busy} onclick={()=>{remove(index);if(editIndex===index){editIndex=null;canonical='';aliases=''}}}>删除规则</button></div>{/each}
 <hr />
 <button class="btn" onclick={load} disabled={busy}>读取库名称</button>
 {#if dirty}<p class="muted">请先保存设置，再预览已有影片。</p>{/if}
 {#if inventory}
 {#each inventory.errors as e}<p role="alert">{e}</p>{/each}
 {#if inventory.cancelled}<p>读取已取消，列表可能不完整。</p>{/if}
 <div class="movies">{#each inventory.movies as movie}<label data-catalog-movie><input type="checkbox" bind:group={selected} value={movie.dir} disabled={busy}/><span>{movie.code} · <span data-actors>{movie.names.actors.join('、')}</span><small>{movie.dir}</small></span></label>{/each}</div>
 <button class="btn" onclick={build} disabled={busy||dirty||!selected.length}>预览所选影片</button>{/if}
 {#if busy}<p role="status">处理中… {progress}</p><button class="btn" disabled={jobId===null} onclick={()=>jobId!==null&&api.jobCancel(jobId)}>停止当前任务</button>{/if}
 {#if preview}<p>原有影片位置保持不变</p>{#each preview.warnings as warning}<p>{warning}</p>{/each}{#each preview.items as item}<div data-name-diff><strong>{item.code}</strong>{#each item.changes as change}<p>{change.field}：{JSON.stringify(change.before)} → {JSON.stringify(change.after)}</p>{/each}{#if !item.changes.length}<p>没有名称变化</p>{/if}</div>{/each}<button class="btn primary" disabled={busy||dirty} onclick={apply}>应用名称规则</button>{/if}
 {#if result}<p role="status">已完成 {result.items.filter(i=>!i.error).length} 项{result.cancelled?' · 已取消':''}</p>{#each result.items.filter(i=>i.error) as item}<p role="alert">{item.code}：{item.error}</p>{/each}{#if result.indexError}<p role="alert">索引更新失败：{result.indexError}</p>{/if}{#if result.items.some(i=>i.journalId)}<button class="btn" onclick={undo} disabled={busy}>撤销本次修改</button>{/if}{/if}
 {#if message}<p role="status">{message}</p>{/if}{#if error}<p role="alert">{error}</p>{/if}
</section>
<style>section{display:grid;gap:12px;padding:var(--s4)}label{display:grid;gap:6px}.toggle{display:flex;align-items:flex-start;gap:10px}textarea{min-height:80px}.preset{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.rule{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.movies{max-height:300px;overflow:auto}.movies label{display:flex;align-items:center;padding:8px;gap:10px}small{display:block;color:var(--text-3);overflow-wrap:anywhere}[data-name-diff]{padding:12px;background:var(--surface-2);border-radius:8px}p{overflow-wrap:anywhere}</style>
