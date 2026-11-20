import { EDITABLE_FIELDS, type Metadata, type MovieDetail, type RestoreReview, type RestoreRow, type RestoreRules } from '../types'
import { readMockPackage, type MockPackage } from './package-export'
import type { PackageManifest } from '../package-export'
type PackageMovie = PackageManifest['movies'][number]

type Changes = RestoreRow['changes']
type Asset = { kind:'poster'|'fanart'|'avatar'; name?:string; data:string; path:string }
export interface RestoreCandidate { before: MovieDetail; after: Metadata; assets:Asset[]; changes:Changes; warnings:string[] }
const eq=(left:unknown,right:unknown)=>JSON.stringify(left)===JSON.stringify(right)
const present=(v:unknown)=>Array.isArray(v)?v.length>0:typeof v==='string'?!!v.trim():v!=null
const take=(rule:'keep'|'fill'|'replace'|undefined,empty:boolean)=>rule==='replace'||((rule??'fill')==='fill' && empty)
function prepare(before: MovieDetail, pkg: MockPackage, movie: PackageMovie,rules:RestoreRules):RestoreCandidate {
  const after=structuredClone(before.meta), src=pkg.metadata[movie.code], assets:Asset[]=[], warnings:string[]=[]
  const sidecar=movie.files.some(f=>f.kind==='sidecar')
  if(src){
    for(const field of [...EDITABLE_FIELDS.map(f=>f.key),'original_title']){
      const rule=['title','tags','actors'].includes(field)?rules[field as 'title'|'tags'|'actors']:rules.metadata
      if(rule==='keep'||((rule??'fill')==='fill' && before.meta.locked_fields?.includes(field)))continue
      const importedLock=sidecar&&src.locked_fields?.includes(field)
      const value=Reflect.get(src,field)
      if(!present(value) && !importedLock)continue
      if(take(rule,!present(Reflect.get(after,field)))&&!eq(value,Reflect.get(after,field))){
        Reflect.set(after,field,structuredClone(value))
        if(after.name_projection)delete after.name_projection[field]
        after.provenance ??= {}
        after.provenance[field]=src.provenance?.[field]??'package'
      }
      if(importedLock&&eq(Reflect.get(after,field),value)&&!after.locked_fields?.includes(field)) (after.locked_fields??=[]).push(field)
    }
    if(sidecar&&take(rules.favorite,!before.favorite)&&!!src.favorite!==before.favorite)after.favorite=!!src.favorite
    if(take(rules.metadata,!before.meta.scores.length)&&src.scores.length)after.scores=structuredClone(src.scores)
    if(sidecar&&src.text_versions&&take(rules.metadata,!before.meta.text_versions))after.text_versions=structuredClone(src.text_versions)
  }
  const changes=Object.keys({...before.meta,...after}).filter(f=>!eq(Reflect.get(before.meta,f),Reflect.get(after,f))).map(field=>({field,before:Reflect.get(before.meta,field),after:Reflect.get(after,field)}))
  for(const file of movie.files){
    const data=pkg.assets[file.path]
    if(file.kind==='poster'||file.kind==='fanart'){
      if(take(rules[file.kind],!before[file.kind])&&before[file.kind]!==data) assets.push({kind:file.kind,data,path:before.dir+'/'+(before.meta.asset_stem ? before.meta.asset_stem+'-' : '')+file.kind+'.jpg'})
    }else if(file.kind==='avatar'&&rules.avatars!=='keep'){
      const actor=after.actors.find(a=>a.name===file.actor || (a.thumb && a.thumb===file.source))
      if(!actor){warnings.push('一张头像未匹配到本片演员');continue}
      if(take(rules.avatars,!before.localActorNames?.includes(actor.name)))assets.push({kind:'avatar',name:actor.name,data,path:before.dir+'/.actors/'+actor.name+'.png'})
    }
  }
  return {before,after,assets,changes,warnings}
}
let pending:{ review:RestoreReview; candidates:Map<string,RestoreCandidate>; settings:string; generation:number }|null=null
export async function restoreReview(root:string,generation:number,settings:string,token:string,directory:string,rules:RestoreRules,get:(code:string)=>Promise<MovieDetail|null>,cancelled:()=>boolean) {
  const pkg=readMockPackage(directory)
  if(!pkg)throw '所选目录没有可读取的 kiroku-package.json'
  const items:RestoreRow[]=[], candidates=new Map<string,RestoreCandidate>()
  for(const movie of pkg.manifest.movies){
    if(cancelled())throw '导回预览已停止'
    const before=await get(movie.code)
    if(!before){items.push({code:movie.code,status:'unmatched',destination:null,changes:[],files:[],warnings:[]});continue}
    const prepared=prepare(before,pkg,movie,rules)
    const changed=prepared.changes.length>0||prepared.assets.length>0
    if(changed)candidates.set(movie.code,prepared)
    const files=prepared.assets.map(a=>a.path)
    if(prepared.changes.length)files.push(before.dir+'/'+(before.meta.asset_stem??'movie')+'.nfo',before.dir+'/kiroku.json')
    items.push({code:movie.code,status:changed?'ready':'unchanged',changes:prepared.changes,files,destination:before.dir,warnings:prepared.warnings})
  }
  const review={root,token,items}
  pending={review,candidates,settings,generation}
  return structuredClone(review)
}
export function takeRestore(root:string,generation:number,settings:string,token:string,codes:string[]) {
  if(!pending||pending.review.root!==root||pending.review.token!==token||pending.settings!==settings||pending.generation!==generation)throw '导回预览已过期，请重新检查'
  const p=pending
  pending=null
  if(!codes.length||codes.some(code=>!p.candidates.has(code)))throw '只能导回已预览且匹配的影片'
  return [...new Set(codes)].map(code=>p.candidates.get(code)!)
}
