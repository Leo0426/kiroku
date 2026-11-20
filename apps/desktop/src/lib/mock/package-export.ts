import type { PackageExportApi, PackageExportPreview, PackageFile, PackageManifest } from '../package-export'
import type { Metadata, MovieDetail } from '../types'
import { emitMock } from '../events'
import { baseName } from '../format'

export interface MockPackage {
  manifest:PackageManifest
  metadata:Record<string,Metadata>
  assets:Record<string,string>
}
const packages=new Map<string,MockPackage>()
export function readMockPackage(directory:string):MockPackage|null {
  const item=packages.get(directory.replace(/\/$/,''))
  return item ? structuredClone(item) : null
}
type Context={getMovie:(code:string)=>Promise<MovieDetail|null>;root:()=>string|null;begin:(requestId?:string)=>number;end:()=>void;cancelled:()=>boolean}

function avatarFile(name:string,used:Set<string>) {
  let stem=name.replace(/[\\/:*?"<>|\x00-\x1f]/g,' ').trim().replace(/\s+/g,' ').replace(/^[. ]+|[. ]+$/g,'')
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(stem)) stem='_'+stem
  let ordinal=used.size+1
  while (!stem || new TextEncoder().encode(stem).length>240 || used.has(stem.normalize().toLowerCase())) {
    stem='kiroku-actor-'+String(ordinal++).padStart(5,'0')
  }
  used.add(stem.normalize().toLowerCase())
  return '.actors/'+stem+'.png'
}
export function mockPackageExport(ctx:Context):PackageExportApi {
  let pending:{preview:PackageExportPreview; pkg:MockPackage; root:string}|null=null
  const check=(root:string)=>{if(ctx.cancelled())throw '导出已停止';if(root!==ctx.root())throw '媒体库已变化'}
  return {
    async packagePreview(codes,kinds,directory,requestId){
      ctx.begin(requestId)
      try{
        pending=null
        const root=ctx.root()
        if(!root||!codes.length||!kinds.length)throw '请选择影片及文件'
        const parent=directory.replace(/\/$/,'')
        if(parent===root||parent.startsWith(root+'/'))throw '请选择当前媒体库之外的导出目录'
        const token=crypto.randomUUID()
        const preview:PackageExportPreview={token,destination:parent+'/kiroku-'+token,movies:0,files:0,bytes:0,examples:[],warnings:[],warningCount:0}
        const pkg:MockPackage={manifest:{version:1,movies:[]},metadata:{},assets:{}}
        for(const code of new Set(codes)){
          check(root)
          const movie=await ctx.getMovie(code)
          if(!movie)throw '库中已找不到 '+code
          const prefix='movies/'+String(preview.movies+1).padStart(5,'0')+'-'+code.replace(/[\\/:]/g,'_')+'/'
          const files:PackageFile[]=[]
          async function add(kind:PackageFile['kind'],name:string,data:string,actor?:string,source?:string){
            if(!kinds.includes(kind))return
            const path=prefix+name
            const bytes=new TextEncoder().encode(data)
            const hash=await crypto.subtle.digest('SHA-256',bytes)
            files.push({kind,path,bytes:bytes.length,sha256:[...new Uint8Array(hash)].map(v=>v.toString(16).padStart(2,'0')).join(''),actor,source})
            pkg.assets[path]=data
            preview.files++;preview.bytes+=bytes.length
            if(preview.examples.length<12)preview.examples.push(path)
          }
          const meta={...movie.meta,favorite:movie.favorite}
          const escape=(s:string)=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;')
          await add('nfo',code+'.nfo','<movie><num>'+escape(code)+'</num><title>'+escape(meta.title??'')+'</title>'+meta.tags.map(t=>'<tag>'+escape(t)+'</tag>').join('')+'</movie>')
          await add('sidecar','kiroku.json',JSON.stringify(meta))
          if(movie.poster)await add('poster',code+'-poster.jpg',movie.poster)
          if(movie.fanart)await add('fanart',code+'-fanart.jpg',movie.fanart)
          if(kinds.includes('sidecar'))pkg.metadata[code]=meta
          else if(kinds.includes('nfo')){const {favorite:_,locked_fields,placements,hosted_actor_images,raw_sources,...publicMeta}=meta;pkg.metadata[code]=publicMeta}
          const avatarNames=new Set<string>()
          if(kinds.includes('avatar'))for(const actor of meta.actors){
            if(movie.localActorNames?.includes(actor.name) || (actor.thumb && meta.hosted_actor_images?.[actor.thumb])) {
              await add('avatar',avatarFile(actor.name,avatarNames),'mock-avatar:'+actor.name,actor.name,actor.thumb??undefined)
            } else {preview.warningCount++;if(preview.warnings.length<30)preview.warnings.push(code+' · '+actor.name+' 缺少本地头像')}
          }
          if(kinds.includes('video'))for(const file of movie.files)await add('video',baseName(file.path),'mock-video:'+baseName(file.path))
          if(files.length){preview.movies++;pkg.manifest.movies.push({code,files})}
          else {preview.warningCount++;if(preview.warnings.length<30)preview.warnings.push(code+' 没有可导出文件，已跳过')}
        }
        check(root)
        if(!preview.files)throw '所选影片没有可导出的本地文件'
        pending={preview,pkg,root}
        return structuredClone(preview)
      }finally{ctx.end()}
    },
    async packageExport(token,requestId){
      const id=ctx.begin(requestId)
      try{
        if(!pending||pending.preview.token!==token)throw '资料包预览已失效'
        const {preview,pkg,root}=pending;pending=null
        for(let n=1;n<=4;n++){
          await new Promise(resolve=>setTimeout(resolve,12));check(root)
          emitMock('package:progress',{jobId:id,done:Math.ceil(preview.bytes*n/4),total:preview.bytes})
        }
        if(packages.has(preview.destination))throw '目标已存在，请重新预览'
        packages.set(preview.destination,pkg)
        return preview
      }finally{ctx.end()}
    },
  }
}
