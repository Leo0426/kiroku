//! Reliable source text and explicit NFO language projection. Never guesses legacy languages.
use crate::Metadata;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::Path};
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextVersion {
    pub title: Option<String>,
    pub plot: Option<String>,
    pub provenance: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextVersions {
    pub original: TextVersion,
    pub localized: BTreeMap<String, TextVersion>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NfoLanguage {
    #[default]
    #[serde(rename = "current")]
    Current,
    #[serde(rename = "zh-Hans")]
    ZhHans,
    #[serde(rename = "original")]
    Original,
}
pub struct LocalizedProjection {
    pub metadata: Metadata,
    pub warnings: Vec<String>,
}
/// Only call on freshly aggregated source values before AI (not arbitrary legacy sidecars).
pub fn capture_original(m: &mut Metadata) {
    for field in ["title", "plot"] {
        let current = if field == "title" {
            m.fields
                .original_title
                .clone()
                .or_else(|| m.fields.title.clone())
        } else {
            m.fields.plot.clone()
        };
        let target = if field == "title" {
            &mut m.text_versions.original.title
        } else {
            &mut m.text_versions.original.plot
        };
        if target.is_none()
            && !m.provenance.get(field).is_some_and(|s| {
                (matches!(s.as_str(), "ai" | "manual") || s.starts_with("localized:"))
            })
            && !m.locked_fields.contains(field)
        {
            *target = current;
            let key = if field == "title" && m.fields.original_title.is_some() {
                "original_title"
            } else {
                field
            };
            if let Some(source) = m.provenance.get(key).or_else(|| m.provenance.get(field)) {
                m.text_versions
                    .original
                    .provenance
                    .insert(field.into(), source.clone());
            }
        }
    }
}
/// Recover old originals only from an explicit original title or the adopted raw source.
pub fn recover_original(m: &mut Metadata) {
    for field in ["title", "plot"] {
        let target = if field == "title" {
            &mut m.text_versions.original.title
        } else {
            &mut m.text_versions.original.plot
        };
        if target.is_some() {
            continue;
        }
        if field == "title" && m.fields.original_title.is_some() {
            *target = m.fields.original_title.clone();
            if let Some(source) = m.provenance.get("original_title") {
                m.text_versions
                    .original
                    .provenance
                    .insert("title".into(), source.clone());
            }
            continue;
        }
        let Some(source) = m
            .provenance
            .get(field)
            .filter(|s| !(matches!(s.as_str(), "ai" | "manual") || s.starts_with("localized:")))
        else {
            continue;
        };
        let Some(snapshot) = m.raw_sources.get(source).filter(|s| s.code == m.code) else {
            continue;
        };
        *target = if field == "title" {
            snapshot
                .fields
                .original_title
                .clone()
                .or_else(|| snapshot.fields.title.clone())
        } else {
            snapshot.fields.plot.clone()
        };
        if target.is_some() {
            m.text_versions
                .original
                .provenance
                .insert(field.into(), source.clone());
        }
    }
}
/// Invalidate exactly one field after its source or manual value changes.
pub fn invalidate(m: &mut Metadata, field: &str) {
    match field {
        "title" => m.text_versions.original.title = None,
        "plot" => m.text_versions.original.plot = None,
        _ => return,
    }
    m.text_versions.original.provenance.remove(field);
    for version in m.text_versions.localized.values_mut() {
        if field == "title" {
            version.title = None;
        } else {
            version.plot = None;
        }
        version.provenance.remove(field);
    }
    if field == "title" {
        m.fields.original_title = None;
    }
}
pub fn replace_field(after: &mut Metadata, candidate: &Metadata, field: &str) {
    if !matches!(field, "title" | "plot") {
        return;
    }
    invalidate(after, field);
    let mut raw = candidate.clone();
    capture_original(&mut raw);
    if field == "title" {
        after.text_versions.original.title = raw.text_versions.original.title;
        after.fields.original_title = raw.fields.original_title;
    } else {
        after.text_versions.original.plot = raw.text_versions.original.plot;
    }
    if let Some(p) = raw.text_versions.original.provenance.get(field) {
        after
            .text_versions
            .original
            .provenance
            .insert(field.into(), p.clone());
    }
    for (lang, version) in raw.text_versions.localized {
        let target = after.text_versions.localized.entry(lang).or_default();
        if field == "title" {
            target.title = version.title;
        } else {
            target.plot = version.plot;
        }
        if let Some(p) = version.provenance.get(field) {
            target.provenance.insert(field.into(), p.clone());
        }
    }
}
pub fn project(m: &Metadata, language: &NfoLanguage) -> LocalizedProjection {
    let mut metadata = m.clone();
    recover_original(&mut metadata);
    if metadata.fields.original_title.is_none() {
        metadata.fields.original_title = metadata.text_versions.original.title.clone();
    }
    let mut warnings = vec![];
    for field in ["title", "plot"] {
        if m.locked_fields.contains(field) || *language == NfoLanguage::Current {
            continue;
        }
        let version = match language {
            NfoLanguage::Original => Some(&metadata.text_versions.original),
            NfoLanguage::ZhHans => metadata
                .text_versions
                .localized
                .iter()
                .find(|(tag, _)| tag.eq_ignore_ascii_case("zh-Hans"))
                .map(|(_, v)| v),
            _ => None,
        };
        let value = version.and_then(|v| {
            if field == "title" {
                v.title.clone()
            } else {
                v.plot.clone()
            }
        });
        match value {
            Some(v) => {
                if field == "title" {
                    metadata.fields.title = Some(v);
                } else {
                    metadata.fields.plot = Some(v);
                }
            }
            None => warnings.push(format!(
                "{}{}未保存，回退当前值",
                if *language == NfoLanguage::Original {
                    "原文"
                } else {
                    "简体中文"
                },
                if field == "title" { "标题" } else { "简介" }
            )),
        }
    }
    LocalizedProjection { metadata, warnings }
}
pub fn export_preview(
    root: &Path,
    dir: &Path,
    code: &str,
    language: NfoLanguage,
) -> io::Result<crate::revision::Preview> {
    let captured = crate::revision::capture(root, dir, code)?;
    let mut after = captured.before.clone();
    recover_original(&mut after);
    after.nfo_language = language;
    captured.prepare_after(after)?.for_language_export()
}
/// Save supplied text (no new translation service) and optionally select it as current text.
pub fn set_localized(
    m: &Metadata,
    language: &str,
    text: TextVersion,
    select: bool,
) -> io::Result<Metadata> {
    let parts: Vec<_> = language.split('-').collect();
    if parts.is_empty()
        || !(2..=8).contains(&parts[0].len())
        || !parts[0].bytes().all(|c| c.is_ascii_alphabetic())
        || parts.iter().skip(1).any(|part| {
            part.is_empty() || part.len() > 8 || !part.bytes().all(|c| c.is_ascii_alphanumeric())
        })
    {
        return Err(io::Error::other(
            "语言标签须为 BCP 47，如 zh-Hans、en 或 ja",
        ));
    }
    let mut after = m.clone();
    recover_original(&mut after);
    let canonical = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                p.to_ascii_lowercase()
            } else if p.len() == 4 {
                let lower = p.to_ascii_lowercase();
                format!("{}{}", lower[..1].to_ascii_uppercase(), &lower[1..])
            } else if p.len() == 2 {
                p.to_ascii_uppercase()
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("-");
    let version = after
        .text_versions
        .localized
        .entry(canonical.clone())
        .or_default();
    for (field, value) in [("title", text.title), ("plot", text.plot)] {
        if let Some(value) = value {
            if field == "title" {
                version.title = Some(value.clone());
            } else {
                version.plot = Some(value.clone());
            }
            version.provenance.insert(field.into(), "manual".into());
            if select && !m.locked_fields.contains(field) {
                if field == "title" {
                    after.fields.title = Some(value);
                } else {
                    after.fields.plot = Some(value);
                }
                after
                    .provenance
                    .insert(field.into(), format!("localized:{canonical}"));
            }
        }
    }
    Ok(after)
}
