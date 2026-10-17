//! Explicit name rules and reversible projections. Raw source snapshots are never edited.
use crate::{metadata::Actor, Metadata};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NameRule {
    pub canonical: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NameRules {
    #[serde(default)]
    pub actors: Vec<NameRule>,
    #[serde(default)]
    pub tags: Vec<NameRule>,
    #[serde(default)]
    pub studio: Vec<NameRule>,
    #[serde(default)]
    pub publisher: Vec<NameRule>,
    #[serde(default)]
    pub series: Vec<NameRule>,
    /// 标题剔除演员名(ADR-0011)。关闭时不序列化,保持旧规则指纹。
    #[serde(default, skip_serializing_if = "is_false")]
    pub strip_actor_names_from_title: bool,
}
fn is_false(b: &bool) -> bool {
    !*b
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NameProjectionRecord {
    pub original: Value,
    pub applied: Value,
    pub fingerprint: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}
pub struct NameProjection {
    pub metadata: Metadata,
    pub warnings: Vec<String>,
}
fn key(s: &str) -> String {
    s.trim().nfc().collect()
}
impl NameRules {
    pub fn fields(&self) -> [(&'static str, &[NameRule]); 5] {
        [
            ("actors", &self.actors),
            ("tags", &self.tags),
            ("studio", &self.studio),
            ("publisher", &self.publisher),
            ("series", &self.series),
        ]
    }
    pub fn validate(&self) -> Result<(), String> {
        for (field, rules) in self.fields() {
            let mut canonical = BTreeSet::new();
            for rule in rules {
                let name = key(&rule.canonical);
                if name.is_empty() || !canonical.insert(name) {
                    return Err(format!("{field}: 规范名不能为空或重复"));
                }
            }
            let mut aliases = BTreeSet::new();
            for rule in rules {
                for alias in &rule.aliases {
                    let alias = key(alias);
                    if alias.is_empty() || canonical.contains(&alias) || !aliases.insert(alias) {
                        return Err(format!("{field}: 别名不能为空、重复或同时作为规范名"));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("serializable name rules"))
        )
    }
}
/// Store original only while the current field still belongs to this projection. An
/// external/manual edit severs ownership instead of restoring a stale original value.
pub fn project(meta: &Metadata, rules: &NameRules) -> Result<NameProjection, String> {
    project_fields(
        meta,
        rules,
        &["actors", "tags", "studio", "publisher", "series", "title"],
    )
}
pub fn project_fields(
    meta: &Metadata,
    rules: &NameRules,
    selected: &[&str],
) -> Result<NameProjection, String> {
    rules.validate()?;
    let mut value = serde_json::to_value(meta).map_err(|e| e.to_string())?;
    let mut records = meta.name_projection.clone();
    let mut warnings = Vec::new();
    let fingerprint = rules.fingerprint();
    for (field, entries) in rules.fields() {
        if !selected.contains(&field) || meta.locked_fields.contains(field) {
            continue;
        }
        let original = records
            .get(field)
            .filter(|r| r.applied == value[field])
            .map(|r| r.original.clone())
            .unwrap_or_else(|| value[field].clone());
        let mut map = BTreeMap::new();
        for rule in entries {
            map.insert(key(&rule.canonical), rule.canonical.trim());
            for alias in &rule.aliases {
                map.insert(key(alias), rule.canonical.trim());
            }
        }
        let name = |s: &str| {
            map.get(&key(s))
                .map(|s| s.to_string())
                .unwrap_or_else(|| s.to_string())
        };
        let warning_start = warnings.len();
        let applied = match field {
            "actors" => {
                let actors: Vec<Actor> =
                    serde_json::from_value(original.clone()).map_err(|e| e.to_string())?;
                let mut out: Vec<Actor> = Vec::new();
                for mut actor in actors {
                    actor.name = name(&actor.name);
                    if let Some(previous) = out.iter_mut().find(|a| key(&a.name) == key(&actor.name)) {
                        if previous.gender != actor.gender {
                            warnings.push(format!("演员 {} 的性别信息冲突，保留首项", actor.name));
                        }
                        if previous.thumb.is_none() {
                            previous.thumb = actor.thumb;
                        }
                    } else {
                        out.push(actor);
                    }
                }
                serde_json::to_value(out).map_err(|e| e.to_string())?
            }
            "tags" => {
                let tags: Vec<String> =
                    serde_json::from_value(original.clone()).map_err(|e| e.to_string())?;
                let mut seen = BTreeSet::new();
                let out: Vec<String> = tags
                    .iter()
                    .map(|s| name(s))
                    .filter(|s| seen.insert(key(s)))
                    .collect();
                serde_json::to_value(out).map_err(|e| e.to_string())?
            }
            _ => match original.as_str() {
                Some(s) => Value::String(name(s)),
                None => original.clone(),
            },
        };
        value[field] = applied.clone();
        if original != applied {
            records.insert(
                field.into(),
                NameProjectionRecord {
                    original,
                    applied,
                    fingerprint: fingerprint.clone(),
                    warnings: warnings[warning_start..].to_vec(),
                },
            );
        } else {
            records.remove(field);
        }
    }
    if selected.contains(&"title") && !meta.locked_fields.contains("title") {
        let current = value["title"].clone();
        let original = records
            .get("title")
            .filter(|r| r.applied == current)
            .map(|r| r.original.clone())
            .unwrap_or(current);
        let applied = match original.as_str() {
            Some(title) if rules.strip_actor_names_from_title => {
                Value::String(strip_actor_names(title, &actor_names(&value, &records, rules)?))
            }
            _ => original.clone(),
        };
        value["title"] = applied.clone();
        if original != applied {
            records.insert(
                "title".into(),
                NameProjectionRecord { original, applied, fingerprint: fingerprint.clone(), warnings: vec![] },
            );
        } else {
            records.remove("title");
        }
    }
    let mut metadata: Metadata = serde_json::from_value(value).map_err(|e| e.to_string())?;
    metadata.name_projection = records;
    Ok(NameProjection { metadata, warnings })
}

/// 本片演员的规范名、规则里的别名,以及投影前的来源原名。
fn actor_names(
    value: &Value,
    records: &BTreeMap<String, NameProjectionRecord>,
    rules: &NameRules,
) -> Result<Vec<String>, String> {
    let parse = |v: &Value| serde_json::from_value::<Vec<Actor>>(v.clone()).map_err(|e| e.to_string());
    let mut names: Vec<String> = parse(&value["actors"])?.into_iter().map(|a| a.name).collect();
    if let Some(r) = records.get("actors") {
        names.extend(parse(&r.original)?.into_iter().map(|a| a.name));
    }
    let current: BTreeSet<String> = names.iter().map(|n| key(n)).collect();
    for rule in rules.actors.iter().filter(|r| current.contains(&key(&r.canonical))) {
        names.extend(rule.aliases.iter().cloned());
    }
    Ok(names)
}

const OPEN: &str = "(（[【「『〔<〈《";
const CLOSE: &str = ")）]】」』〕>〉》";
const SEPARATORS: &[char] = &['-', '–', '—', '_', '・', '･', '/', '／', '|', '｜', ',', '，', '、', ':', '：', '~', '～', '&', '＆', '+', '＋'];

/// 从标题里删掉演员名:NFC 比较;少于 2 个字符的名字不处理;长名优先;CJK 按字面子串,
/// ASCII 名不区分大小写且两侧须为词边界。删除后清掉空括号、合并空白、去首尾分隔符。
/// 没有命中或清理后为空时原样返回。
pub fn strip_actor_names(title: &str, names: &[String]) -> String {
    let mut names: Vec<String> = names.iter().map(|n| key(n)).filter(|n| n.chars().count() >= 2).collect();
    names.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    names.dedup();
    let mut text: String = title.nfc().collect();
    let mut removed = false;
    for name in &names {
        let ascii = name.is_ascii();
        let mut from = 0;
        loop {
            let hay = if ascii { text.to_ascii_lowercase() } else { text.clone() };
            let needle = if ascii { name.to_ascii_lowercase() } else { name.clone() };
            let Some(at) = hay[from..].find(&needle).map(|i| i + from) else { break };
            let end = at + needle.len();
            let bounded = !ascii
                || (!text[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric())
                    && !text[end..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric()));
            if bounded {
                text.replace_range(at..end, " ");
                removed = true;
                from = at;
            } else {
                from = end;
            }
        }
    }
    if !removed {
        return title.to_string();
    }
    loop {
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut next = words.join(" ");
        for (o, c) in OPEN.chars().zip(CLOSE.chars()) {
            next = next.replace(&format!("{o}{c}"), "").replace(&format!("{o} {c}"), "");
        }
        if next == text {
            break;
        }
        text = next;
    }
    let cleaned = text.trim_matches(|c: char| c.is_whitespace() || SEPARATORS.contains(&c)).to_string();
    if cleaned.is_empty() { title.to_string() } else { cleaned }
}

/// 内置标签译名:简体规范名 + 繁体(JavDB/JavBus)和日文(DMM/XCity)别名。
pub fn tag_preset() -> Vec<NameRule> {
    serde_json::from_str(include_str!("../assets/tag_preset.json")).expect("valid tag preset")
}

/// 预设并入标签规则的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetMerge {
    pub rules: NameRules,
    pub added_rules: usize,
    pub added_aliases: usize,
}

impl NameRules {
    /// 把预设并入标签规则,不改动用户已有的任何规则:
    /// 规范名已存在时只补充未占用的别名;规范名被用户用作别名时整条跳过;
    /// 已被占用的别名丢弃,丢完没有别名的预设不加。
    pub fn merge_tags(&self, preset: &[NameRule]) -> PresetMerge {
        let mut rules = self.clone();
        let (mut added_rules, mut added_aliases) = (0, 0);
        let mut used: BTreeSet<String> = rules
            .tags
            .iter()
            .flat_map(|r| std::iter::once(&r.canonical).chain(&r.aliases))
            .map(|s| key(s))
            .collect();
        for p in preset {
            let canonical = key(&p.canonical);
            let existing = rules.tags.iter().position(|r| key(&r.canonical) == canonical);
            if existing.is_none() && used.contains(&canonical) {
                continue;
            }
            let aliases: Vec<String> = p
                .aliases
                .iter()
                .filter(|a| used.insert(key(a)))
                .cloned()
                .collect();
            if aliases.is_empty() {
                continue;
            }
            added_aliases += aliases.len();
            match existing {
                Some(i) => rules.tags[i].aliases.extend(aliases),
                None => {
                    used.insert(canonical);
                    added_rules += 1;
                    rules.tags.push(NameRule { canonical: p.canonical.clone(), aliases });
                }
            }
        }
        PresetMerge { rules, added_rules, added_aliases }
    }
}

/// Original-film revision bound to authoritative disk metadata and exact file bytes.
pub fn catalog_preview(
    root: &std::path::Path,
    dir: &std::path::Path,
    code: &str,
    rules: &NameRules,
) -> std::io::Result<crate::revision::Preview> {
    let captured = crate::revision::capture(root, dir, code)?;
    let projected = project(&captured.before, rules).map_err(std::io::Error::other)?;
    captured.prepare_after(projected.metadata)
}

/// Only diagnostics whose projection still owns the current field are presented.
pub fn warnings(meta: &Metadata) -> Vec<String> {
    let Ok(value) = serde_json::to_value(meta) else {
        return vec![];
    };
    meta.name_projection
        .iter()
        .filter(|(field, r)| value[*field] == r.applied)
        .flat_map(|(_, r)| r.warnings.clone())
        .collect()
}
