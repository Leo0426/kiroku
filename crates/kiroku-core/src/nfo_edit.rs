//! Byte-span edits keep all unselected XML exactly as found, including CDATA/comments/attributes.
use crate::Metadata;
use quick_xml::{events::Event, Reader};
use std::{collections::BTreeSet, io};

struct Node {
    name: String,
    start: usize,
    open_end: usize,
    close_start: usize,
    end: usize,
    parent: Option<usize>,
    empty: bool,
    aspect: Option<String>,
}
struct Document {
    nodes: Vec<Node>,
}
fn invalid(reason: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("无法安全局部更新 NFO：{reason}"))
}
fn parse(xml: &str) -> io::Result<Document> {
    let mut r = Reader::from_str(xml);
    let mut nodes: Vec<Node> = vec![];
    let mut stack = vec![];
    let mut roots = 0;
    loop {
        let start = r.buffer_position() as usize;
        let event = r.read_event().map_err(invalid)?;
        let end = r.buffer_position() as usize;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = xml[..end].ends_with("/>");
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(invalid)?
                    .to_owned();
                let mut aspect = None;
                for attribute in e.attributes() {
                    let a = attribute.map_err(invalid)?;
                    if a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:") {
                        return Err(invalid("命名空间文档尚不支持，请先转换为普通 movie NFO"));
                    }
                    if a.key.as_ref() == b"aspect" {
                        aspect = Some(a.unescape_value().map_err(invalid)?.into_owned());
                    }
                }
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || name != "movie" || empty {
                        return Err(invalid("需要一个非空 movie 根节点"));
                    }
                }
                let index = nodes.len();
                nodes.push(Node {
                    name,
                    start,
                    open_end: end,
                    close_start: end,
                    end,
                    parent: stack.last().copied(),
                    empty,
                    aspect,
                });
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                let index = stack.pop().ok_or_else(|| invalid("节点未配对"))?;
                nodes[index].close_start = start;
                nodes[index].end = end;
            }
            Event::Text(t) => {
                let text = t.unescape().map_err(invalid)?;
                if stack.is_empty() && !text.trim().is_empty() {
                    return Err(invalid("根节点以外含文本"));
                }
            }
            Event::CData(_) if stack.is_empty() => return Err(invalid("根节点以外含 CDATA")),
            Event::DocType(_) => return Err(invalid("不支持带 DTD 的文档")),
            Event::Decl(d) => {
                if let Some(encoding) = d.encoding() {
                    let encoding = encoding.map_err(invalid)?;
                    if !encoding.eq_ignore_ascii_case(b"utf-8") {
                        return Err(invalid("仅支持 UTF-8 编码"));
                    }
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    if roots != 1 || !stack.is_empty() {
        return Err(invalid("movie 文档不完整"));
    }
    Ok(Document { nodes })
}
fn tags(field: &str) -> &'static [&'static str] {
    match field {
        "title" => &["title", "sorttitle"],
        "original_title" => &["originaltitle"],
        "plot" => &["plot", "outline"],
        "actors" => &["actor"],
        "tags" => &["tag", "genre"],
        "directors" => &["director"],
        "studio" => &["studio", "maker"],
        "publisher" => &["publisher", "label"],
        "series" => &["series", "set"],
        "release_date" => &["premiered", "releasedate", "year"],
        "runtime_min" => &["runtime"],
        "poster_urls" => &["poster"],
        "thumb_urls" => &["cover"],
        "trailer_urls" => &["trailer"],
        _ => &[],
    }
}
fn replace_content(xml: &str, node: &Node, content: &str) -> String {
    if node.empty {
        format!(
            "{}>{content}</{}>",
            &xml[node.start..node.open_end - 2],
            node.name
        )
    } else {
        format!(
            "{}{content}{}",
            &xml[node.start..node.open_end],
            &xml[node.close_start..node.end]
        )
    }
}
/// Merge only Metadata fields that actually changed. Untouched XML is copied byte-for-byte.
/// Unsupported XML is rejected before any write. Used by manual edits and `revision::prepare`.
pub fn merge_revision(xml: &str, before: &Metadata, after: &Metadata) -> io::Result<String> { merge(xml,before,after,false) }
pub fn merge_language_export(xml:&str,after:&Metadata)->io::Result<String>{merge(xml,after,after,true)}
fn merge(xml:&str,before:&Metadata,after:&Metadata,language_export:bool)->io::Result<String>{
    let document = parse(xml)?;
    let generated = super::to_nfo(after);
    let projection = parse(&generated)?;
    let old = serde_json::to_value(crate::localization::project(before,&before.nfo_language).metadata)?;
    let new = serde_json::to_value(crate::localization::project(after,&after.nfo_language).metadata)?;
    let mut selected = BTreeSet::new();
    for field in crate::aggregate::FIELDS {
        if old[*field] != new[*field] {
            selected.extend(tags(field).iter().copied());
        }
    }
    if language_export || before.nfo_language!=after.nfo_language {selected.extend(["title","sorttitle","plot","outline","originaltitle"]);}
    if before.scores != after.scores {
        selected.extend(["rating", "criticrating"]);
    }
    if selected.is_empty() {
        return Ok(xml.into());
    }
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let mut additions = String::new();
    for tag in selected {
        let matches = |node: &Node| {
            node.parent == Some(0)
                && (node.name == tag
                    || (node.name == "thumb"
                        && ((tag == "poster" && node.aspect.as_deref() == Some("poster"))
                            || (tag == "cover" && node.aspect.as_deref() != Some("poster")))))
        };
        let targets: Vec<_> = document
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches(n))
            .collect();
        let replacements: Vec<_> = projection
            .nodes
            .iter()
            .filter(|n| n.parent == Some(0) && n.name == tag)
            .collect();
        if matches!(tag, "actor" | "tag" | "genre" | "director") {
            let rendered = replacements
                .iter()
                .map(|n| &generated[n.start..n.end])
                .collect::<Vec<_>>()
                .join("\n");
            if let Some((_, first)) = targets.first() {
                edits.push((first.start, first.end, rendered));
                for (_, n) in targets.iter().skip(1) {
                    edits.push((n.start, n.end, String::new()));
                }
            } else if !rendered.is_empty() {
                additions.push_str(&rendered);
                additions.push('\n');
            }
            continue;
        }
        if targets.is_empty() {
            for n in replacements {
                additions.push_str(&generated[n.start..n.end]);
                additions.push('\n');
            }
            continue;
        }
        for (position,(index, node)) in targets.into_iter().enumerate() {
            if position>0 && matches!(tag,"title"|"sorttitle"|"plot"|"outline"|"originaltitle") {edits.push((node.start,node.end,String::new()));continue;}
            if tag == "set" && replacements.is_empty() {
                let children: Vec<_> = document
                    .nodes
                    .iter()
                    .filter(|n| n.parent == Some(index))
                    .collect();
                if children.is_empty() {
                    edits.push((node.start, node.end, String::new()));
                } else {
                    for name in children.iter().filter(|n| n.name == "name") {
                        edits.push((name.start, name.end, String::new()));
                    }
                }
                continue;
            }
            let Some(replacement) = replacements.first() else {
                edits.push((node.start, node.end, String::new()));
                continue;
            };
            if tag == "set" {
                let names: Vec<_> = document
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.parent == Some(index) && n.name == "name")
                    .collect();
                let desired = projection
                    .nodes
                    .iter()
                    .find(|n| {
                        n.name == "name"
                            && n.parent.is_some_and(|p| projection.nodes[p].name == "set")
                    })
                    .ok_or_else(|| invalid("系列投影没有 name"))?;
                let text = &generated[desired.open_end..desired.close_start];
                if names.is_empty() {
                    if document.nodes.iter().any(|n| n.parent == Some(index)) {
                        return Err(invalid("set 节点没有可更新的 name"));
                    }
                    edits.push((node.start, node.end, replace_content(xml, node, text)));
                } else {
                    for (name_index, name) in names {
                        if document.nodes.iter().any(|n| n.parent == Some(name_index)) {
                            return Err(invalid("系列名称包含嵌套节点"));
                        }
                        edits.push((name.start, name.end, replace_content(xml, name, text)));
                    }
                }
            } else {
                if document.nodes.iter().any(|n| n.parent == Some(index)) {
                    return Err(invalid(format!("待修改的 {tag} 含嵌套节点")));
                }
                edits.push((
                    node.start,
                    node.end,
                    replace_content(
                        xml,
                        node,
                        &generated[replacement.open_end..replacement.close_start],
                    ),
                ));
            }
        }
    }
    if !additions.is_empty() {
        let close = document.nodes[0].close_start;
        edits.push((close, close, additions));
    }
    edits.sort_by_key(|(start, _, _)| *start);
    let mut result = String::new();
    let mut cursor = 0;
    for (start, end, text) in edits {
        if start < cursor {
            return Err(invalid("更新节点重叠"));
        }
        result.push_str(&xml[cursor..start]);
        result.push_str(&text);
        cursor = end;
    }
    result.push_str(&xml[cursor..]);
    Ok(result)
}
