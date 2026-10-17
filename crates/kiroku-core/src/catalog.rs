//! Explicit existing-film name projections; no moves, network requests, or index writes.
use crate::{
    names::{self, NameRules},
    pipeline::RootGuard,
    revision,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMovie {
    pub code: String,
    pub dir: PathBuf,
    pub names: BTreeMap<String, Vec<String>>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub movies: Vec<CatalogMovie>,
    pub names: BTreeMap<String, BTreeMap<String, usize>>,
    pub errors: Vec<String>,
    pub cancelled: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub root: PathBuf,
    pub fingerprint: String,
    pub items: Vec<revision::Preview>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    guard: RootGuard,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemOutcome {
    pub code: String,
    pub dir: PathBuf,
    pub journal_id: Option<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub items: Vec<ItemOutcome>,
    pub cancelled: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub code: String,
}

pub fn inventory(root: &Path, cancel: &CancellationToken) -> io::Result<Inventory> {
    let root = root.canonicalize()?;
    let guard = RootGuard::capture(&root, false)?;
    let mut result = Inventory {
        movies: vec![],
        names: BTreeMap::new(),
        errors: vec![],
        cancelled: false,
    };
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name() != ".kiroku")
    {
        if cancel.is_cancelled() {
            result.cancelled = true;
            break;
        }
        guard.verify()?;
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(e.to_string());
                continue;
            }
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        if !dir.join("kiroku.json").exists() {
            match crate::assets::nfo_path(dir) {
                Ok(path) if path.exists() => {},
                Ok(_) => continue,
                Err(error) => { result.errors.push(format!("{}: {error}", dir.display())); continue; }
            }
        }
        crate::append::confined_path(&root, dir)?;
        let meta = match crate::append::read_meta(dir) {
            Ok(m) => m,
            Err(e) => {
                result.errors.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let names: BTreeMap<String, Vec<String>> = BTreeMap::from([
            (
                "actors".into(),
                meta.fields.actors.iter().map(|a| a.name.clone()).collect(),
            ),
            ("tags".into(), meta.fields.tags.clone()),
            ("studio".into(), meta.fields.studio.into_iter().collect()),
            (
                "publisher".into(),
                meta.fields.publisher.into_iter().collect(),
            ),
            ("series".into(), meta.fields.series.into_iter().collect()),
        ]);
        for (field, values) in &names {
            for name in values.iter().collect::<BTreeSet<_>>() {
                *result
                    .names
                    .entry(field.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default() += 1;
            }
        }
        result.movies.push(CatalogMovie {
            code: meta.code,
            dir: dir.to_path_buf(),
            names,
        });
    }
    guard.verify()?;
    result.movies.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(result)
}
pub fn preview(
    root: &Path,
    dirs: &[PathBuf],
    rules: &NameRules,
    cancel: &CancellationToken,
) -> io::Result<Preview> {
    rules.validate().map_err(io::Error::other)?;
    let original_root = std::path::absolute(root)?;
    let root = root.canonicalize()?;
    let guard = RootGuard::capture(&root, false)?;
    let mut items = vec![];
    let mut warnings = vec![];
    let mut seen = BTreeSet::new();
    for dir in dirs {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消"));
        }
        guard.verify()?;
        let absolute_dir = std::path::absolute(dir)?;
        let normalized = absolute_dir
            .strip_prefix(&original_root)
            .map(|p| root.join(p))
            .unwrap_or(absolute_dir.clone());
        let dir = crate::append::confined_path(&root, &normalized)?;
        if !seen.insert(dir.clone()) {
            continue;
        }
        let m = crate::append::read_meta(&dir)?;
        let captured = revision::capture(&root, &dir, &m.code)?;
        let projected = names::project(&captured.before, rules).map_err(io::Error::other)?;
        warnings.extend(
            projected
                .warnings
                .into_iter()
                .map(|w| format!("{}: {w}", m.code)),
        );
        items.push(captured.prepare_after(projected.metadata)?);
    }
    guard.verify()?;
    Ok(Preview {
        root,
        fingerprint: rules.fingerprint(),
        items,
        warnings,
        guard,
    })
}
pub fn apply(
    preview: &Preview,
    rules: &NameRules,
    cancel: &CancellationToken,
    on_progress: &dyn Fn(Progress),
) -> io::Result<Outcome> {
    rules.validate().map_err(io::Error::other)?;
    if preview.fingerprint != rules.fingerprint() {
        return Err(io::Error::other("名称规则已变化，请重新预览"));
    }
    preview.guard.verify()?;
    let mut outcome = Outcome {
        items: vec![],
        cancelled: false,
    };
    for item in &preview.items {
        if cancel.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        if let Err(e) = preview.guard.verify() {
            outcome.items.push(ItemOutcome {
                code: item.code.clone(),
                dir: item.dir.clone(),
                journal_id: None,
                error: Some(e.to_string()),
            });
            break;
        }
        let result = if item.before == item.after {
            ItemOutcome {
                code: item.code.clone(),
                dir: item.dir.clone(),
                journal_id: None,
                error: None,
            }
        } else {
            match revision::apply(item) {
                Ok(out) => ItemOutcome {
                    code: item.code.clone(),
                    dir: item.dir.clone(),
                    journal_id: Some(out.journal.id),
                    error: out.error,
                },
                Err(e) => ItemOutcome {
                    code: item.code.clone(),
                    dir: item.dir.clone(),
                    journal_id: None,
                    error: Some(e.to_string()),
                },
            }
        };
        outcome.items.push(result);
        on_progress(Progress {
            done: outcome.items.len(),
            total: preview.items.len(),
            code: item.code.clone(),
        });
    }
    Ok(outcome)
}
