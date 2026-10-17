//! Selective, non-destructive metadata updates. Persistence belongs to revision.
use crate::{revision::EDITABLE_FIELDS, Metadata};
use serde_json::Value;
use std::io;
pub fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(v) => !v.is_empty(),
        _ => true,
    }
}
pub fn set_lock(before: &Metadata, field: &str, locked: bool) -> io::Result<Metadata> {
    if !crate::aggregate::FIELDS.contains(&field) {
        return Err(io::Error::other("未知锁定字段"));
    }
    let mut after = before.clone();
    if locked {
        after.locked_fields.insert(field.into());
    } else {
        after.locked_fields.remove(field);
    }
    Ok(after)
}
pub fn select_source(before: &Metadata, field: &str, source: &str) -> io::Result<Metadata> {
    if !EDITABLE_FIELDS.contains(&field) {
        return Err(io::Error::other("该字段不支持来源重选"));
    }
    let snapshot = before
        .raw_sources
        .get(source)
        .filter(|s| s.code == before.code)
        .ok_or_else(|| io::Error::other("没有该来源的有效原始快照，请先更新缓存"))?;
    let raw = serde_json::to_value(&snapshot.fields)?;
    if !nonempty(&raw[field]) {
        return Err(io::Error::other("该来源没有这个字段的非空候选"));
    }
    let mut value = serde_json::to_value(before)?;
    value[field] = raw[field].clone();
    let mut after: Metadata = serde_json::from_value(value)?;
    crate::localization::invalidate(&mut after,field);
    after.provenance.insert(field.into(), source.into());
    crate::localization::recover_original(&mut after);
    after.locked_fields.insert(field.into());
    if let Some(url) = &snapshot.fields.source_url {
        after
            .source_urls
            .insert(source.into(), crate::metadata::public_url(url));
    } else {
        after.source_urls.remove(source);
    }
    Ok(after)
}
pub fn merge_selected(
    before: &Metadata,
    candidate: &Metadata,
    fields: &[String],
    replace: bool,
) -> io::Result<Metadata> {
    let mut old = serde_json::to_value(before)?;
    let new = serde_json::to_value(candidate)?;
    let mut replaced=vec![];
    let mut provenance = before.provenance.clone();
    let mut source_urls = before.source_urls.clone();
    for field in fields {
        if !EDITABLE_FIELDS.contains(&field.as_str()) {
            return Err(io::Error::other("不可更新字段"));
        }
        if before.locked_fields.contains(field)
            || !nonempty(&new[field])
            || (!replace && nonempty(&old[field]))
        {
            continue;
        }
        old[field] = new[field].clone();
        if let Some(record) = candidate.name_projection.get(field) {
            old["name_projection"][field] = serde_json::to_value(record)?;
        } else if let Some(records) = old["name_projection"].as_object_mut() { records.remove(field); }
        replaced.push(field.clone());
        if let Some(source) = candidate.provenance.get(field) {
            provenance.insert(field.clone(), source.clone());
            if let Some(url) = candidate.source_urls.get(source) {
                source_urls.insert(source.clone(), crate::metadata::public_url(url));
            } else {
                source_urls.remove(source);
            }
        } else {
            provenance.remove(field);
        }
    }
    let mut after: Metadata = serde_json::from_value(old)?;
    after.provenance = provenance;
    after.source_urls = source_urls;
    after.raw_sources = candidate.raw_sources.clone();
    for field in replaced {crate::localization::replace_field(&mut after,candidate,&field);}
    Ok(after)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCandidate {
    pub source: String,
    pub value: Value,
    pub fetched_at: u64,
}
/// Derived choices use the same eligibility rules as fetch/cache and never expose credentials.
pub fn source_candidates(
    meta: &Metadata,
    settings: &crate::settings::Settings,
) -> std::collections::BTreeMap<String, Vec<SourceCandidate>> {
    let mut out = EDITABLE_FIELDS
        .iter()
        .map(|f| ((*f).into(), vec![]))
        .collect::<std::collections::BTreeMap<String, Vec<SourceCandidate>>>();
    let Some(code) = crate::code::parse_code(&meta.code) else {
        return out;
    };
    let route = settings.chains_for(code.content_type).route;
    let sources = settings.build_sources();
    for site in route {
        let Some(cfg) = settings.sources.iter().find(|s| s.enabled && s.id == site) else {
            continue;
        };
        if !sources.iter().any(|s| s.id() == site && s.supports(&code)) {
            continue;
        }
        let Some(snapshot) = meta
            .raw_sources
            .get(&site)
            .filter(|s| s.matches_config(&code, cfg, settings.proxy_for(cfg)))
        else {
            continue;
        };
        let Ok(fields) = serde_json::to_value(&snapshot.fields) else {
            continue;
        };
        for field in EDITABLE_FIELDS {
            if nonempty(&fields[*field]) {
                out.get_mut(*field).unwrap().push(SourceCandidate {
                    source: site.clone(),
                    value: fields[*field].clone(),
                    fetched_at: snapshot.fetched_at,
                });
            }
        }
    }
    out
}

/// Explicit source selection applies current rules before locking the selected value.
pub fn select_source_named(before: &Metadata, field: &str, source: &str, rules: &crate::names::NameRules) -> io::Result<Metadata> {
    let mut selected = select_source(before, field, source)?;
    selected.name_projection.remove(field);
    selected.locked_fields.remove(field);
    let mut after = crate::names::project_fields(&selected, rules, &[field]).map_err(io::Error::other)?.metadata;
    after.locked_fields.insert(field.into());
    Ok(after)
}
