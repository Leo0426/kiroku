<script lang="ts">
  import { untrack } from 'svelte'
  import { app } from '../lib/state.svelte'
  import { readScopePage, type ExportScope } from '../lib/export-scope'
  import type { MovieCard, Query } from '../lib/types'
  import LoadingStatus from './LoadingStatus.svelte'

  let {query,scope=$bindable(),disabled=false,limit=50,labelPrefix=''}:{
    query:Query; scope:ExportScope; disabled?:boolean; limit?:number; labelPrefix?:string
  }=$props()
  let movies=$state.raw<MovieCard[]>([])
  let total=$state(0), offset=$state(0), error=$state(''), loading=$state(false), retry=$state(0)
  const pageSize=$derived(Math.min(200,Math.max(1,limit)))
  const scopeKind=$derived(scope.kind)
  let savedSelection:string[]=[]
  let previousRoot=untrack(() => app.settings?.library_root)
  const selected=$derived(scope.kind==='selection' ? scope.codes : [])
  const chosen=$derived(new Set(selected))

  $effect.pre(()=>{
    JSON.stringify(query)
    const root=app.settings?.library_root
    if(root!==previousRoot){
      previousRoot=root
      savedSelection=[]
      if(untrack(()=>scope.kind)==='selection')scope={kind:'selection',codes:[]}
    }
    offset=0
  })
  $effect(()=>{
    const q=$state.snapshot(query), current=offset, kind=scopeKind, size=pageSize
    retry; app.settings?.library_root; app.libraryVersion
    let alive=true
    const controller=new AbortController()
    loading=true; error=''; movies=[]
    void readScopePage({...(kind==='library'?{}:q),offset:kind==='selection'?current:0,limit:kind==='selection'?size:1},controller.signal)
      .then(result=>{if(alive){total=result.total;movies=result.items; if(current>=total&&current>0)offset=Math.max(0,Math.floor((total-1)/size)*size)}})
      .catch(e=>{if(alive){error=String(e);total=0}})
      .finally(()=>{if(alive)loading=false})
    return ()=>{alive=false;controller.abort()}
  })
  function change(kind:ExportScope['kind']) {
    if(scope.kind==='selection')savedSelection=[...scope.codes]
    scope=kind==='selection'?{kind,codes:savedSelection}:{kind}
    offset=0
  }
  function toggle(codes:string[], add:boolean) {
    const set=new Set(untrack(()=>selected))
    for(const code of codes){if(add)set.add(code);else set.delete(code)}
    scope={kind:'selection',codes:[...set]}
  }
</script>

<fieldset class="scope" {disabled}>
  <legend>影片范围</legend>
  <div class="line">
    <select aria-label="{labelPrefix}影片范围" class="input" value={scope.kind} onchange={e=>change(e.currentTarget.value as ExportScope['kind'])}>
      <option value="selection">手动勾选</option><option value="filtered">当前筛选全部</option><option value="library">整个媒体库</option>
    </select>
    {#if scope.kind==='selection'}<span class="muted">已选 {selected.length} 部</span>
    {:else if !loading && !error}<span class="muted">共 {total} 部</span>{/if}
  </div>
  {#if error}<p role="alert">{error}</p><button class="btn sm" onclick={()=>retry++}>重新读取范围</button>
  {:else if loading}<LoadingStatus label="正在读取影片范围…" />
  {:else if scope.kind==='selection'}
    {#if movies.length}
      {@const allSelected=movies.every(movie=>chosen.has(movie.code))}
      <button class="btn sm ghost all" onclick={()=>toggle(movies.map(movie=>movie.code),!allSelected)}>{allSelected?'取消本页选择':'选择本页'}</button>
      <div class="movies">
        {#each movies as movie (movie.code)}
          <label><input class="checkbox" type="checkbox" checked={chosen.has(movie.code)} aria-label={labelPrefix ? labelPrefix+movie.code : undefined} onchange={e=>toggle([movie.code],e.currentTarget.checked)}/>
            <span class="mono">{movie.code}</span><span>{movie.title??''}</span></label>
        {/each}
      </div>
      {#if total>pageSize}<div class="line pages"><button class="btn sm" disabled={offset===0} onclick={()=>offset=Math.max(0,offset-pageSize)}>上一页</button>
        <span>{Math.floor(offset/pageSize)+1} / {Math.ceil(total/pageSize)}</span><button class="btn sm" disabled={offset+pageSize>=total} onclick={()=>offset+=pageSize}>下一页</button></div>{/if}
    {:else}<p class="muted">没有符合条件的影片。</p>{/if}
  {/if}
</fieldset>

<style>
.scope { min-width:0; border:0; margin:0; padding:0; }
legend { font-weight:600; padding:0; margin-bottom:var(--s2); }
.line { display:flex; align-items:center; flex-wrap:wrap; gap:var(--s2); }
.all, .movies label { display:flex; align-items:center; gap:var(--s2); min-height:32px; }
.all { margin-top:var(--s2); }
.movies { max-height:210px; overflow:auto; border-block:1px solid var(--line); }
.movies label { padding:2px var(--s1); }
.movies label:hover { background:var(--surface-2); }
.mono { white-space:nowrap; }
.pages { margin-top:var(--s2); }
p { margin-block:var(--s2); }
</style>
