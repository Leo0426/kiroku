import type {Metadata, NameField, NameRule, NameRules, PresetMerge} from './types'
export const NAME_FIELDS:NameField[]=['actors','tags','studio','publisher','series']
export const EMPTY_RULES:NameRules={actors:[],tags:[],studio:[],publisher:[],series:[]}
const key=(s:string)=>s.trim().normalize('NFC')
export function validateNames(rules:NameRules){
 for(const field of NAME_FIELDS){
  const canonical=new Set<string>(),aliases=new Set<string>()
  for(const rule of rules[field]??[]){const k=key(rule.canonical);if(!k||canonical.has(k))throw '规范名不能为空或重复';canonical.add(k)}
  for(const rule of rules[field]??[])for(const alias of rule.aliases){const k=key(alias);if(!k||canonical.has(k)||aliases.has(k))throw '别名不能为空、重复或同时作为规范名';aliases.add(k)}
 }
}
/** Browser facade mirrors core reversible rules; source snapshots remain untouched. */
export function projectNames(meta:Metadata,rules:NameRules,selected:readonly string[]=[...NAME_FIELDS,'title']){
 validateNames(rules)
 const after=structuredClone(meta),warnings:string[]=[],records=after.name_projection??{}
 const fields=after as unknown as Record<string,unknown>
 for(const field of NAME_FIELDS){
  if(!selected.includes(field)||meta.locked_fields?.includes(field))continue
  const record=records[field],original=record&&JSON.stringify(record.applied)===JSON.stringify(fields[field])?structuredClone(record.original):structuredClone(fields[field])
  const map=new Map<string,string>();for(const r of rules[field]??[]){map.set(key(r.canonical),r.canonical.trim());for(const a of r.aliases)map.set(key(a),r.canonical.trim())}
  const name=(v:string)=>map.get(key(v))??v
  const warningStart=warnings.length
  let applied:unknown
  if(field==='actors'){
   const actors:Metadata['actors']=[]
   for(const actor of original as Metadata['actors']){const a={...actor,name:name(actor.name)},old=actors.find(v=>key(v.name)===key(a.name));if(old){if(old.gender!==a.gender)warnings.push(`演员 ${a.name} 的性别信息冲突，保留首项`)}else actors.push(a)}
   applied=actors
  }else if(field==='tags') {const seen=new Set<string>();applied=(original as string[]).map(name).filter(v=>{const k=key(v);if(seen.has(k))return false;seen.add(k);return true})}
  else applied=typeof original==='string'?name(original):original
  fields[field]=applied
  if(JSON.stringify(original)!==JSON.stringify(applied))records[field]={original,applied:structuredClone(applied),fingerprint:JSON.stringify(rules),warnings:warnings.slice(warningStart)}
  else delete records[field]
 }
 if(selected.includes('title')&&!meta.locked_fields?.includes('title')){
  const record=records.title,current=fields.title,original=record&&JSON.stringify(record.applied)===JSON.stringify(current)?record.original:current
  const applied=typeof original==='string'&&rules.strip_actor_names_from_title?stripActorNames(original,actorNames(after,records,rules)):original
  fields.title=applied
  if(JSON.stringify(original)!==JSON.stringify(applied))records.title={original,applied,fingerprint:JSON.stringify(rules),warnings:[]}
  else delete records.title
 }
 after.name_projection=records;return {metadata:after,warnings}
}

function actorNames(meta:Metadata,records:NonNullable<Metadata['name_projection']>,rules:NameRules){
 const names=meta.actors.map(a=>a.name)
 const original=records.actors?.original as Metadata['actors']|undefined
 if(original)names.push(...original.map(a=>a.name))
 const current=new Set(names.map(key))
 for(const r of rules.actors??[])if(current.has(key(r.canonical)))names.push(...r.aliases)
 return names
}
const PAIRS=['()','（）','[]','【】','「」','『』','〔〕','<>','〈〉','《》']
const SEPARATORS=new Set('-–—_・･/／|｜,，、:：~～&＆+＋')
const isWord=(c:string|undefined)=>!!c&&/[0-9A-Za-z]/.test(c)
/** Mirror of core names::strip_actor_names. */
export function stripActorNames(title:string,names:string[]){
 const list=[...new Set(names.map(key).filter(n=>[...n].length>=2))].sort((a,b)=>[...b].length-[...a].length||(a<b?-1:a>b?1:0))
 let text=title.normalize('NFC'),removed=false
 for(const name of list){
  const ascii=/^[\x00-\x7f]*$/.test(name),needle=ascii?name.toLowerCase():name
  let from=0
  for(;;){
   const hay=ascii?text.replace(/[A-Z]/g,c=>c.toLowerCase()):text,at=hay.indexOf(needle,from)
   if(at<0)break
   const end=at+needle.length
   if(!ascii||(!isWord(text[at-1])&&!isWord(text[end]))){text=text.slice(0,at)+' '+text.slice(end);removed=true;from=at}else from=end
  }
 }
 if(!removed)return title
 for(;;){let next=text.split(/\s+/).filter(Boolean).join(' ');for(const [o,c] of PAIRS)next=next.split(o+c).join('').split(`${o} ${c}`).join('');if(next===text)break;text=next}
 const chars=[...text];let a=0,b=chars.length
 while(a<b&&(/\s/.test(chars[a])||SEPARATORS.has(chars[a])))a++
 while(b>a&&(/\s/.test(chars[b-1])||SEPARATORS.has(chars[b-1])))b--
 const cleaned=chars.slice(a,b).join('')
 return cleaned||title
}

/** Browser facade mirror of core NameRules::merge_tags: existing user rules are never changed. */
export function mergeTags(rules:NameRules,preset:NameRule[]):PresetMerge{
 const next=structuredClone(rules),used=new Set(next.tags.flatMap(r=>[r.canonical,...r.aliases]).map(key))
 let added_rules=0,added_aliases=0
 for(const p of preset){
  const c=key(p.canonical),existing=next.tags.find(r=>key(r.canonical)===c)
  if(!existing&&used.has(c))continue
  const aliases=p.aliases.filter(a=>{const k=key(a);if(used.has(k))return false;used.add(k);return true})
  if(!aliases.length)continue
  added_aliases+=aliases.length
  if(existing)existing.aliases.push(...aliases)
  else{used.add(c);added_rules++;next.tags.push({canonical:p.canonical,aliases})}
 }
 return {rules:next,added_rules,added_aliases}
}
