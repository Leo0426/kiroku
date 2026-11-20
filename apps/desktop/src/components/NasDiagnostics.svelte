<script lang="ts">
 import {onMount,onDestroy} from 'svelte'
 import {api} from '../lib/api'
 import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
 import type {NasStatus} from '../lib/types'
 let {compact=false}:{compact?:boolean}=$props()
 let status=$state<NasStatus|null>(null),error=$state(''),busy=$state(false)
 let alive=true,task:CancellableJob<NasStatus>|null=null
 const labels={disabled:'未启用',invalid:'配置失败',unverified:'待核验',verified:'映射已核验'}
 onMount(()=>{void api.nasStatus().then(s=>{if(alive)status=s}).catch(e=>{if(alive)error=String(e)})})
 onDestroy(()=>{alive=false;void task?.cancel()})
 async function diagnose(){
  if(busy)return
  busy=true;error=''
  const job=cancellableJob(id=>api.nasDiagnose(id),e=>{if(alive)error=`停止失败，请重试：${e}`});task=job
  try{const result=await job.promise;if(alive&&!job.cancelled)status=result}
  catch(e){if(alive)error=String(e)}finally{if(alive){busy=false;task=null}}
 }
</script>
{#if !compact||status?.state!=='disabled'||error}
<section class="nas-check" aria-label="NAS 文件操作">
 <strong>NAS 文件操作{status?` · ${labels[status.state]}`:''}</strong>
 {#if status}<p>{status.message}</p>{/if}
 {#if status?.localRoot}<p class="path">{status.localRoot} → {status.remoteRoot}</p>{/if}
 {#if status?.state==='invalid'}<p role="alert">媒体写入已阻止。修正设置目录的 nas-operations.json 后重启应用。</p>{/if}
 {#if !compact||status?.state==='unverified'||status?.state==='verified'}
 <p>检测会在映射根目录创建并清理中性临时文件，检查连接、映射、硬链接和不覆盖移动；不改动媒体。检测通过不保证所有子目录或跨卷操作可用。</p>
 <button class="btn sm" disabled={busy||!status||status.state==='invalid'||status.state==='disabled'} onclick={diagnose}>{busy?'正在检测…':'检测 NAS 文件操作'}</button>
 {/if}
 {#if busy}<button class="btn sm ghost" onclick={()=>task?.cancel()}>停止检测</button>{/if}
 {#if error}<p role="alert">{error}</p>{/if}
 {#if !compact&&status?.state==='disabled'}<p>可选配置文件：设置目录下 nas-operations.json；填写 host、local_root、remote_root，配置 SSH 免交互认证后重启。应用不保存 SSH 密钥。</p>{/if}
</section>
{/if}
<style>
 .nas-check{padding:16px;margin:12px 0;border:1px solid var(--line);border-radius:var(--r-md);font-size:var(--fs-sm)}
 p{margin:8px 0;color:var(--text-2);overflow-wrap:anywhere}.path{font-family:monospace}button{margin-right:8px}[role=alert]{color:var(--danger)}
</style>
