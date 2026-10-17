import type { TemplatePreview } from './types'
const values:Record<string,string>={code:'SMPL-001',number:'SMPL-001',title:'示例标题',original_title:'サンプル',actor:'示例演员',actors:'示例演员',actress:'示例演员',actresses:'示例演员',year:'2024',release:'2024-01-02',runtime:'120',raw_name:'SMPL-001-CD2-C',raw_dir:'示例文件夹',prefix:'SMPL',suffix:'001',ext:'mp4',content_type:'censored',studio:'',publisher:'',series:'',directors:'',score:'',cd:'2',sub:'C',mosaic:'censored',def:''}
const optional=['cd','sub','mosaic','def']
function validate(tpl:string,file:boolean,legacy=false){
 if(tpl.startsWith('/')||tpl.startsWith('\\')||tpl.split('/').includes('..')||tpl.includes(':\\')||/^[A-Za-z]:/.test(tpl))throw '模板不能使用绝对路径或 ..'
 let groups=0,code=false
 for(let i=0;i<tpl.length;i++){const ch=tpl[i];if(ch==='['){if(groups)throw '可选组不能嵌套';groups=tpl[i+1]==='['?2:1;if(groups===2)i++}else if(ch===']'){if(!groups)throw '可选组括号未配对';if(groups===2&&tpl[++i]!==']')throw '双层可选组括号未配对';groups=0}else if(ch==='{'){
 const end=tpl.indexOf('}',i+1);if(end<0)throw '占位符括号未配对';const inner=tpl.slice(i+1,end);if(inner.includes('{'))throw '占位符括号未配对';const [head,map]=inner.split('|');const key=head.replace(/\?$/,'')
 if(!legacy&&!(head.endsWith('?')?optional.includes(key):key in values&&!optional.includes(key)))throw `未知占位符: {${head}}`
 if(file&&key==='ext')throw '文件名模板不含扩展名，请移除 {ext}，整理时会自动附加'
 if(key==='code'||key==='number'){code=true}
 if(map!==undefined&&map.split(',').some(p=>!p.includes('=')))throw '映射格式应为 原值=输出'
 i=end
 }else if(ch==='}')throw '占位符括号未配对'}
 if(groups)throw '可选组括号未配对';if(file&&!code)throw '文件名模板必须包含 {code} 或 {number}'
}
function sanitize(s:string){const out=s.replace(/[\\/:*?"<>|\x00-\x1f]/g,' ').split(/\s+/).filter(Boolean).join(' ').replace(/^[. ]+|[. ]+$/g,'');return /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(out)?`_${out}`:out}
function substitute(s:string,group:boolean,values:Record<string,string>){let missing=false,opts=0,filled=0
 const out=s.replace(/\{([^}]+)\}/g,(_,inner:string)=>{const [head,map]=inner.split('|'),key=head.replace(/\?$/,'');if(!(key in values))return `{${inner}}`;let value=values[key]??'';if(map){const pair=map.split(',').map(p=>p.split('=')).find(([k])=>k.trim()===value);if(pair)value=sanitize(pair[1].trim())}if(head.endsWith('?')){opts++;if(value)filled++}else if(!value){missing=true;value=key==='title'?'':key==='year'?'0000':'Unknown'}return value})
 return group&&(missing||opts>0&&filled===0)?'':out
}
function render(tpl:string,values:Record<string,string>){return substitute(tpl.replace(/\[\[([^\[\]]*)\]\]|\[([^\[\]]*)\]/g,(_,wrapped:string,body:string)=>{const s=substitute(wrapped??body,true,values);return wrapped!==undefined&&s?`[${s}]`:s}),false,values)}
export function previewTemplate(directory:string,file:string,context:Record<string,string>=values):TemplatePreview {
 const warnings:string[]=[]
 function limit(value:string,budget:number){const chars=Array.from(value),protectedIndices=new Set<number>();for(const token of [context.code,context.cd?`-CD${context.cd}`:'',context.cd?`- cd${context.cd}`:''].filter(Boolean)){let index=value.indexOf(token);while(index>=0){const start=Array.from(value.slice(0,index)).length;for(let i=start;i<start+Array.from(token).length;i++)protectedIndices.add(i);index=value.indexOf(token,index+token.length)}}let bytes=new TextEncoder().encode(value).length;for(let i=chars.length-1;i>=0&&bytes>budget;i--){if(!protectedIndices.has(i)){bytes-=new TextEncoder().encode(chars[i]).length;chars[i]=''}}if(bytes>budget)throw '番号和分片标记超过文件名限制';const result=chars.join('').trim();if(result!==value)warnings.push(`名称已截短：${value} → ${result}`);return result}
 try{validate(directory,false,context!==values);validate(file,true,context!==values);const dir=render(directory,context).split('/').map(sanitize).filter(Boolean).map(v=>limit(v,240)).join('/');if(!dir)throw '目录模板结果为空';const rendered=sanitize(render(file,{...context,title:Array.from(context.title??'').slice(0,60).join('')}));const stem=limit(rendered,240-Math.max((context.ext??'mp4').length+1,'-fanart.jpeg'.length));if(!stem.includes(context.code))throw '文件名不能截断或省略番号';return {directory:dir,file:stem+'.'+(context.ext??'mp4'),warnings,error:null}}
 catch(e){return {directory:'',file:'',warnings:[],error:String(e)}}
}

export function validateTemplate(tpl:string,file:boolean){validate(tpl,file)}
