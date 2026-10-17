//! 命名模板:目录模板与文件名模板共用,语法对齐 amane `organize/path_templates`。
//!
//! - 占位符 `{code}`/`{number}` `{title}` `{actor}`(首个女优) `{year}` `{studio}` `{publisher}` `{series}` `{content_type}`;
//!   取不到值时,可选组之外用兜底(Unknown / 0000 / 空)。
//! - 可空占位符(文件相位,取自扫描结果):`{cd?}` 分集号、`{sub?}` 中字为 `C`、`{mosaic?}` 为
//!   censored/uncensored/cracked/leaked、`{def?}` 清晰度(4K、1080p…)。没有就是空串,不用兜底。
//! - 值映射 `{name|原值=输出,另一值=输出}`:按规范值改写,`{mosaic?|=未知}` 可给空值指定输出。
//! - 可选组 `[...]`:组内有普通占位符取不到值,或组内的可空占位符(映射后)全为空,整组省略;
//!   `[[...]]` 同样省略,保留时外面包一层方括号(`{code}[[{def?}]]` → `ABC-123[4K]`)。
//! - 未知占位符原样保留。

use std::path::Path;
use crate::{
    code::{Code, ContentType, Mosaic},
    metadata::{Gender, Metadata},
    organize::sanitize,
};

/// 默认文件名模板:保留分集与中字/破解/流出标记,且能被番号解析认回来(`-UC` `-U` `-C` `-LEAK` `-LEAKC`)。
pub const DEFAULT_FILE_TEMPLATE: &str = "{code}[-CD{cd?}][-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}]";

/// 「Plex 友好命名」使用的文件名模板:标题进文件名,分集写成 Plex 能合并的 ` - cdN`。
pub const PLEX_FILE_TEMPLATE: &str = "{code}[-{mosaic?|cracked=U,leaked=LEAK,censored=,uncensored=}{sub?}][ {title}][ - cd{cd?}]";

/// 文件名里的标题最多保留这么多字符。
const FILE_TITLE_MAX: usize = 60;

const REQUIRED: [&str; 22] = ["code", "number", "title", "actor", "year", "studio", "publisher", "series", "content_type", "ext", "prefix", "suffix", "actors", "actress", "actresses", "release", "original_title", "directors", "runtime", "score", "raw_name", "raw_dir"];
const OPTIONAL: [&str; 4] = ["cd", "sub", "mosaic", "def"];

/// 取值上下文。`code` 为当前文件的番号与相位(没有时可空占位符都为空)。
pub struct Ctx<'a> {
    pub meta: &'a Metadata,
    pub code: Option<&'a Code>,
    pub source: Option<&'a Path>,
}

impl<'a> Ctx<'a> {
    pub fn with_source(meta: &'a Metadata, code: Option<&'a Code>, source: &'a Path) -> Self { Self { meta, code, source: Some(source) } }
    pub fn new(meta: &'a Metadata, code: Option<&'a Code>) -> Self {
        Self { meta, code, source: None }
    }
}

enum Value {
    /// 普通占位符取不到值
    Missing(&'static str),
    /// 可空占位符(值可能为空串)
    Optional(String),
    Required(String),
}

fn content_type_name(ct: ContentType) -> &'static str {
    match ct {
        ContentType::Censored => "censored",
        ContentType::Uncensored => "uncensored",
        ContentType::Chinese => "chinese",
        ContentType::Western => "western",
        ContentType::Fc2 => "fc2",
        ContentType::Amateur => "amateur",
        ContentType::Hentai => "hentai",
    }
}

fn mosaic_name(m: Mosaic) -> &'static str {
    match m {
        Mosaic::Censored => "censored",
        Mosaic::Uncensored => "uncensored",
        Mosaic::Cracked => "cracked",
        Mosaic::Leaked => "leaked",
    }
}

fn required(name: &str, ctx: &Ctx, file: bool) -> Option<String> {
    let (m, f) = (ctx.meta, &ctx.meta.fields);
    let non_empty = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(sanitize).filter(|s| !s.is_empty());
    match name {
        "code" | "number" => Some(sanitize(&m.code)),
        "title" => non_empty(f.title.as_deref().or(f.original_title.as_deref()))
            // 文件名不宜过长
            .map(|t| if file { t.chars().take(FILE_TITLE_MAX).collect::<String>().trim().to_string() } else { t }),
        "actor" => non_empty(f.actors.iter().find(|a| a.gender == Gender::Female).or(f.actors.first()).map(|a| a.name.as_str())),
        "prefix" | "suffix" => crate::code::parse_code(&m.code).and_then(|c| c.id.rsplit_once('-').or_else(||c.id.split_once('.')).map(|(p,s)| if name == "prefix" { p.to_string() } else { s.to_string() })),
        "actress" => non_empty(f.actors.iter().find(|a| a.gender == Gender::Female).map(|a| a.name.as_str())),
        "actors" | "actresses" => { let mut names=Vec::new(); for a in &f.actors { if (name=="actors" || a.gender==Gender::Female) && !names.contains(&a.name) { names.push(a.name.clone()); } } non_empty(Some(&names.join("、"))) },
        "release" => non_empty(f.release_date.as_deref()),
        "original_title" => non_empty(f.original_title.as_deref()),
        "directors" => { let mut names=Vec::new();for d in &f.directors { if !names.contains(d) {names.push(d.clone());} } non_empty(Some(&names.join("、"))) },
        "runtime" => f.runtime_min.map(|v| v.to_string()),
        "score" => m.mean_score().map(|v| v.to_string()),
        "ext" => ctx.source.and_then(Path::extension).and_then(|v| non_empty(v.to_str())),
        "raw_name" => ctx.source.and_then(Path::file_stem).and_then(|v| non_empty(v.to_str())),
        "raw_dir" => ctx.source.and_then(Path::parent).and_then(Path::file_name).and_then(|v| non_empty(v.to_str())),
        "year" => m.year().map(String::from),
        "studio" => non_empty(f.studio.as_deref()),
        "publisher" => non_empty(f.publisher.as_deref()),
        "series" => non_empty(f.series.as_deref()),
        "content_type" => ctx.code.map(|c| content_type_name(c.content_type).to_string()),
        _ => None,
    }
}

fn optional(name: &str, ctx: &Ctx) -> String {
    let Some(c) = ctx.code else { return String::new() };
    match name {
        "cd" => c.part.map(|p| p.to_string()),
        "sub" => c.subtitled.then(|| "C".to_string()),
        "mosaic" => c.mosaic.map(|m| mosaic_name(m).to_string()),
        "def" => c.definition.clone(),
        _ => None,
    }
    .unwrap_or_default()
}

/// 缺失时的兜底值(只在可选组之外使用)。
fn fallback_of(name: &str) -> &'static str {
    match name {
        "year" => "0000",
        "title" => "",
        _ => "Unknown",
    }
}

/// 解析 `{...}` 内部;未知占位符返回 None。
fn lookup(inner: &str, ctx: &Ctx, file: bool) -> Option<Value> {
    let (head, map) = match inner.split_once('|') {
        Some((h, m)) => (h, Some(m)),
        None => (inner, None),
    };
    let mapped = |v: String| -> String {
        let Some(map) = map else { return v };
        map.split(',')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| k.trim() == v)
            .map(|(_, out)| sanitize(out.trim()))
            .unwrap_or(v)
    };
    if let Some(name) = head.strip_suffix('?') {
        let name = OPTIONAL.iter().find(|n| **n == name)?;
        return Some(Value::Optional(mapped(optional(name, ctx))));
    }
    let name = REQUIRED.iter().find(|n| **n == head)?;
    Some(match required(name, ctx, file) {
        Some(v) => Value::Required(mapped(v)),
        None => Value::Missing(name),
    })
}

/// 替换一段文字里的占位符。`group` 时按可选组规则返回 None 表示整组省略。
fn substitute(text: &str, ctx: &Ctx, group: bool, file: bool) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let (mut optionals, mut filled) = (0, 0);
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let found = after.find('}').and_then(|j| lookup(&after[..j], ctx, file).map(|v| (j, v)));
        let Some((j, value)) = found else {
            out.push('{');
            rest = after;
            continue;
        };
        match value {
            Value::Required(v) => out.push_str(&v),
            Value::Optional(v) => {
                optionals += 1;
                filled += usize::from(!v.is_empty());
                out.push_str(&v);
            }
            Value::Missing(_) if group => return None,
            Value::Missing(name) => out.push_str(fallback_of(name)),
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    if group && optionals > 0 && filled == 0 {
        return None;
    }
    Some(out)
}

/// 展开可选组,得到未清洗的结果。
fn expand(tpl: &str, ctx: &Ctx, file: bool) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find('[') {
        out.push_str(&substitute(&rest[..i], ctx, false, file).unwrap_or_default());
        let after = &rest[i + 1..];
        // `[[...]]`:保留时外包方括号
        let (body, close, wrap) = match after.strip_prefix('[') {
            Some(inner) => (inner, "]]", true),
            None => (after, "]", false),
        };
        let Some(j) = body.find(close) else {
            // 括号不配对:按普通文字处理
            out.push('[');
            rest = after;
            continue;
        };
        if let Some(group) = substitute(&body[..j], ctx, true, file) {
            if wrap { out.push('['); }
            out.push_str(&group);
            if wrap { out.push(']'); }
        }
        rest = &body[j + close.len()..];
    }
    out.push_str(&substitute(rest, ctx, false, file).unwrap_or_default());
    out
}

/// 目录模板:用 `/` 分层,每一级单独清洗,空的层级去掉。
pub fn render_dir(tpl: &str, ctx: &Ctx) -> String {
    expand(tpl, ctx, false).split('/').map(sanitize).map(|v| limit_component(&v, 240, &[&ctx.meta.code]).unwrap_or(v)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

/// 文件名模板(不含扩展名):整体清洗,`/` 不会产生子目录;结果为空时退回番号。
pub fn render_file(tpl: &str, ctx: &Ctx) -> String {
    let name = sanitize(&expand(tpl, ctx, true));
    let name=if name.is_empty(){sanitize(&ctx.meta.code)}else{name};
    let ext=ctx.source.and_then(Path::extension).and_then(|e|e.to_str()).unwrap_or("mp4");
    let markers=protected_markers(tpl,ctx);let protected:Vec<_>=markers.iter().map(String::as_str).collect();
    limit_component(&name,240usize.saturating_sub(ext.len()+1),&protected).unwrap_or(name)

}

/// 文件名模板必须含番号,否则同批影片会互相冲突。
pub fn validate_file_template(tpl: &str) -> Result<(), String> {
    validate_template(tpl, true)

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        code::{parse_code, parse_file_info},
        metadata::{Actor, PartialMetadata},
    };

    fn meta() -> Metadata {
        Metadata {
            code: "SMPL-001".into(),
            fields: PartialMetadata { title: Some("T: x/y".into()), actors: vec![Actor::new("Foo", Gender::Female)], ..Default::default() },
            ..Default::default()
        }
    }

    fn file(tpl: &str, name: &str) -> String {
        let (m, c) = (meta(), parse_code(name).unwrap());
        render_file(tpl, &Ctx::new(&m, Some(&c)))
    }

    #[test]
    fn default_file_template_keeps_markers() {
        let cases = [
            ("SMPL-001.mp4", "SMPL-001"),
            ("SMPL-001-C.mp4", "SMPL-001-C"),
            ("smpl001-uc.mp4", "SMPL-001-UC"),
            ("SMPL-001-U-CD2.mp4", "SMPL-001-CD2-U"),
            ("SMPL-001-UMR.mp4", "SMPL-001-U"),
            ("SMPL-001 流出.mp4", "SMPL-001-LEAK"),
            ("SMPL-001-C 流出.mp4", "SMPL-001-LEAKC"),
            ("HEYZO-1234-C.mp4", "SMPL-001-C"), // 无码类型不标(meta 的番号固定为 SMPL-001)
        ];
        for (src, want) in cases {
            assert_eq!(file(DEFAULT_FILE_TEMPLATE, src), want, "{src}");
        }
    }

    /// 整理后的文件名再解析,相位不变(重建索引、再次整理都不丢中字/破解/流出)。
    #[test]
    fn default_names_round_trip_through_the_parser() {
        for src in ["SMPL-001-C.mp4", "SMPL-001-UC.mp4", "SMPL-001-U.mp4", "SMPL-001-leak.mp4", "SMPL-001-C-leak.mp4", "SMPL-001-UC-CD2.mp4", "SMPL-001.mp4"] {
            let before = parse_file_info(src);
            let after = parse_file_info(&format!("{}.mp4", file(DEFAULT_FILE_TEMPLATE, src)));
            assert_eq!(
                (after.number, after.cd, after.has_subtitle, after.mosaic),
                (before.number, before.cd, before.has_subtitle, before.mosaic),
                "{src}"
            );
        }
    }

    #[test]
    fn plex_template_and_title_limit() {
        assert_eq!(file(PLEX_FILE_TEMPLATE, "SMPL-001-UC-CD2.mp4"), "SMPL-001-UC T x y - cd2");
        let mut m = meta();
        m.fields.title = Some("长".repeat(100));
        let c = parse_code("SMPL-001.mp4").unwrap();
        assert_eq!(render_file(PLEX_FILE_TEMPLATE, &Ctx::new(&m, Some(&c))).chars().count(), "SMPL-001 ".len() + FILE_TITLE_MAX);
        m.fields.title = None;
        assert_eq!(render_file(PLEX_FILE_TEMPLATE, &Ctx::new(&m, Some(&c))), "SMPL-001");
    }

    #[test]
    fn groups_mapping_and_brackets() {
        // amane 文档里的例子
        let t = "{code}[-CD{cd?}][-{mosaic?|cracked=U,censored=}{sub?}]";
        assert_eq!(file(t, "SMPL-001-U-C-CD2.mp4"), "SMPL-001-CD2-UC");
        assert_eq!(file(t, "SMPL-001-U.mp4"), "SMPL-001-U");
        assert_eq!(file(t, "SMPL-001.mp4"), "SMPL-001", "不残留 -");
        assert_eq!(file("{code}[[{def?}]]", "SMPL-001-4K.mp4"), "SMPL-001[4K]");
        assert_eq!(file("{code}[[{def?}]]", "SMPL-001.mp4"), "SMPL-001");
        assert_eq!(file("{code} {mosaic?|=未知}", "FC2-PPV-1234567.mp4"), "SMPL-001 未知");
        assert_eq!(file("{code} {content_type|censored=有码}", "SMPL-001.mp4"), "SMPL-001 有码");
        // 普通占位符缺失仍按原规则整组省略
        assert_eq!(file("{code}[ - {series}{sub?}]", "SMPL-001-C.mp4"), "SMPL-001");
        // 文件名里的 / 不会产生子目录;未知占位符原样保留
        assert_eq!(file("{code}/{nope?}", "SMPL-001.mp4"), "SMPL-001 {nope }");
    }

    #[test]
    fn dir_templates_unchanged_without_a_code() {
        let m = meta();
        let ctx = Ctx::new(&m, None);
        assert_eq!(render_dir("{actor}/{code} {title}", &ctx), "Foo/SMPL-001 T x y");
        assert_eq!(render_dir("{actor}/{code}[ - {series}]", &ctx), "Foo/SMPL-001");
        assert_eq!(render_dir("{code}[-{sub?}]", &ctx), "SMPL-001");
        assert_eq!(render_dir("{content_type}/{code}", &ctx), "Unknown/SMPL-001");
    }

    #[test]
    fn file_template_must_name_the_code() {
        assert!(validate_file_template(DEFAULT_FILE_TEMPLATE).is_ok());
        assert!(validate_file_template("{number}-x").is_ok());
        assert!(validate_file_template("{title}").is_err());
    }
}

/// Strict diagnostics for newly saved templates. Rendering old templates stays permissive.
pub fn validate_template(tpl: &str, file: bool) -> Result<(), String> { validate_template_inner(tpl,file,false) }
pub fn validate_legacy_template(tpl:&str,file:bool)->Result<(),String>{validate_template_inner(tpl,file,true)}
fn validate_template_inner(tpl:&str,file:bool,legacy:bool)->Result<(),String>{
    if tpl.starts_with('/') || tpl.starts_with('\\') || tpl.split('/').any(|s| s == "..") || tpl.contains(":\\") || tpl.as_bytes().get(1)==Some(&b':') { return Err("模板不能使用绝对路径或 ..".into()); }
    let mut code = false; let mut groups = 0; let mut chars = tpl.chars().peekable();
    while let Some(ch)=chars.next() { match ch {
      '[' => { if groups>0{return Err("可选组不能嵌套".into());}groups=if chars.peek()==Some(&'['){chars.next();2}else{1}; },
      ']' => {if groups==0{return Err("可选组括号未配对".into());}if groups==2 && chars.next()!=Some(']'){return Err("双层可选组括号未配对".into());}groups=0;},
      '{' => {let mut token=String::new();let mut closed=false;for c in chars.by_ref(){if c=='}'{closed=true;break;} if c=='{'{return Err("占位符括号未配对".into());}token.push(c);}if !closed{return Err("占位符括号未配对".into());}
       let (head,map)=token.split_once('|').map_or((token.as_str(),None),|(h,m)|(h,Some(m)));
       if !legacy && !REQUIRED.contains(&head) && !head.strip_suffix('?').is_some_and(|h|OPTIONAL.contains(&h)) {return Err(format!("未知占位符: {{{head}}}"));}
       if file && head=="ext" {return Err("文件名模板不含扩展名，请移除 {ext}，整理时会自动附加".into());}
       if matches!(head,"code"|"number") {code=true;}
       if let Some(map)=map {if map.split(',').any(|p|!p.contains('=')){return Err("映射格式应为 原值=输出".into());}}
      },
      '}' => return Err("占位符括号未配对".into()), _=>()
    }}
    if groups!=0{return Err("可选组括号未配对".into());}
    if file && !code{return Err("文件名模板必须包含 {code} 或 {number}".into());} Ok(())
}
fn protected_markers(tpl:&str,ctx:&Ctx)->Vec<String>{
    let mut markers=vec![ctx.meta.code.clone()];
    let Some(part)=ctx.code.and_then(|c|c.part) else{return markers;};
    markers.push(format!("-CD{part}"));markers.push(format!("- cd{part}"));
    let mut rest=tpl;while let Some(start)=rest.find('{') {let after=&rest[start+1..];let Some(end)=after.find('}') else{break;};let token=&after[..end];
        if token.split('|').next()==Some("cd?") {if let Some(Value::Optional(value))=lookup(token,ctx,true){
            let prefix=rest[..start].rsplit(['}', '[', ']', '/']).next().unwrap_or_default();
            markers.push(sanitize(&format!("{prefix}{value}")));
        }}rest=&after[end+1..];
    } markers
}
/// UTF-8 byte budget, retaining protected code and part markers wherever they occur.
pub fn limit_component(value: &str, budget: usize, protected: &[&str]) -> Result<String,String> {
    if value.len()<=budget {return Ok(value.into());}
    let mut chars:Vec<(usize,char,bool)>=value.char_indices().map(|(i,c)|(i,c,false)).collect();
    for token in protected.iter().filter(|t|!t.is_empty()) {for (start,_) in value.match_indices(token) {for (i,_,keep) in &mut chars {if *i>=start && *i<start+token.len(){*keep=true;}}}}
    if chars.iter().filter(|(_,_,k)|*k).map(|(_,c,_)|c.len_utf8()).sum::<usize>()>budget{return Err("番号和分片标记超过文件名限制".into());}
    let mut bytes=value.len();for (_,c,keep) in chars.iter_mut().rev(){if bytes<=budget{break;}if !*keep {bytes-=c.len_utf8();*c='\0';}}
    Ok(chars.into_iter().filter_map(|(_,c,_)|(c!='\0').then_some(c)).collect::<String>().trim().to_string())
}

/// Validate a concrete plan and report every truncation before applying it.
pub fn preview_paths(dir_tpl: &str, file_tpl: &str, ctx: &Ctx) -> Result<(String,String,Vec<String>),String> {
    let extension=ctx.source.and_then(Path::extension).and_then(|e|e.to_str()).unwrap_or("mp4");
    preview_paths_with_suffix(dir_tpl,file_tpl,ctx,(extension.len()+1).max("-fanart.jpeg".len()))
}

/// Share one filename budget across a video and its companion suffixes.
pub(crate) fn preview_paths_with_suffix(dir_tpl: &str, file_tpl: &str, ctx: &Ctx, suffix_bytes: usize) -> Result<(String,String,Vec<String>),String> {
    validate_template_inner(dir_tpl,false,true)?;validate_template_inner(file_tpl,true,true)?;
    let mut markers=protected_markers(file_tpl,ctx);markers.extend(protected_markers(dir_tpl,ctx));let protected:Vec<_>=markers.iter().map(String::as_str).collect();
    let mut warnings=Vec::new();let mut components=Vec::new();
    for original in expand(dir_tpl,ctx,false).split('/') {let value=sanitize(original);if value.is_empty(){continue;}let limited=limit_component(&value,240,&protected)?;if limited!=value{warnings.push(format!("目录名称已截短：{value} → {limited}"));}components.push(limited);}
    if components.is_empty(){return Err("目录模板结果为空".into());}
    let raw=sanitize(&expand(file_tpl,ctx,true));let value=if raw.is_empty(){sanitize(&ctx.meta.code)}else{raw};
    let budget=240usize.checked_sub(suffix_bytes).ok_or("扩展名超过文件名限制")?;
    let limited=limit_component(&value,budget,&protected)?;
    if !limited.contains(&ctx.meta.code) {return Err("文件名不能截断或省略番号".into());}
    if value!=limited {warnings.push(format!("文件名称已截短：{value} → {limited}"));}
    Ok((components.join("/"),limited,warnings))
}
