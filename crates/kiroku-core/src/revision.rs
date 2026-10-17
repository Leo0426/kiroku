//! In-place metadata revisions; previews bind disk bytes, library and code. Never touches video.
use crate::{
    journal::{
        replacement::{current, digest, read, within},
        Journal, JournalItem, ReplacementInput,
    },
    organize::SIDECAR,
    Metadata,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub const EDITABLE_FIELDS: &[&str] = &[
    "title",
    "plot",
    "actors",
    "tags",
    "studio",
    "publisher",
    "series",
    "release_date",
    "runtime_min",
    "directors",
];
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub field: String,
    pub before: Value,
    pub after: Value,
}
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub code: String,
    pub before: Metadata,
    pub after: Metadata,
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    inputs: Vec<ReplacementInput>,
    #[serde(skip)]
    media_guards: Vec<(PathBuf,String)>,
    #[serde(skip)]
    root_guard: crate::pipeline::RootGuard,
    #[serde(skip)]
    dir_guard: crate::pipeline::RootGuard,
}
pub struct Outcome {
    pub journal: Journal,
    pub error: Option<String>,
}

fn load(
    root: &Path,
    dir: &Path,
    code: &str,
) -> io::Result<(Metadata, Value, PathBuf, Option<Vec<u8>>, Option<Vec<u8>>)> {
    within(root, &dir.join(SIDECAR))?;
    let mut nfos = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    nfos.retain(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("nfo")));
    if nfos.len() > 1 {
        return Err(io::Error::other(
            "影片目录存在多个 NFO，请先明确保留一个再编辑",
        ));
    }
    let side = read(&dir.join(SIDECAR))?;
    let nfo = match nfos.pop() {
        Some(path) => path,
        None => match &side {
            Some(bytes) => crate::assets::Paths::for_movie(dir, &serde_json::from_slice::<Metadata>(bytes)?)?.nfo,
            None => dir.join("movie.nfo"),
        },
    };
    within(root, &nfo)?;
    let nfo_bytes = read(&nfo)?;
    let (meta, raw) = if let Some(bytes) = &side {
        let raw: Value = serde_json::from_slice(bytes)?;
        let meta: Metadata = serde_json::from_value(raw.clone())?;
        (meta, raw)
    } else {
        let text = nfo_bytes
            .as_ref()
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or_else(|| io::Error::other("无法读取磁盘元数据"))?;
        let mut meta = crate::nfo_read::parse_nfo(text, code)
            .ok_or_else(|| io::Error::other("无法解析 NFO"))?;
        crate::assets::infer_from_nfo(&mut meta, &nfo);
        let raw = serde_json::to_value(&meta)?;
        (meta, raw)
    };
    if meta.code != code {
        return Err(io::Error::other("磁盘番号已变化，请重建索引"));
    }
    Ok((meta, raw, nfo, side, nfo_bytes))
}
/// Capture authoritative disk metadata and all digests before an asynchronous update.
pub fn capture(root: &Path, dir: &Path, code: &str) -> io::Result<Preview> {
    let root = root.canonicalize()?;
    let dir = dir.canonicalize()?;
    let (before, _, _, _, _) = load(&root, &dir, code)?;
    let preview = prepare(&root, &dir, code, before.clone())?;
    if preview.before != before {
        return Err(io::Error::other("捕获期间磁盘内容变化，请重试"));
    }
    Ok(preview)
}
pub fn preview(root: &Path, dir: &Path, code: &str, patch: Value) -> io::Result<Preview> {
    let root = root.canonicalize()?;
    let dir = dir.canonicalize()?;
    let (before, _, _, _, _) = load(&root, &dir, code)?;
    let patch = patch
        .as_object()
        .ok_or_else(|| io::Error::other("编辑内容必须是字段对象"))?;
    let mut value = serde_json::to_value(&before)?;
    for (field, v) in patch {
        if !EDITABLE_FIELDS.contains(&field.as_str()) {
            return Err(io::Error::other(format!("不可人工编辑字段: {field}")));
        }
        value[field] = v.clone();
    }
    let mut after: Metadata = serde_json::from_value(value)?;
    let old = serde_json::to_value(&before)?;
    let new = serde_json::to_value(&after)?;
    for field in EDITABLE_FIELDS {
        if old[field] != new[field] {
            after.name_projection.remove(*field);
            after.locked_fields.insert((*field).into());
            after.provenance.insert((*field).into(), "manual".into());
        }
    }
    let prepared = prepare(&root, &dir, code, after)?;
    if prepared.before != before {
        return Err(io::Error::other("预览期间磁盘内容变化，请重试"));
    }
    Ok(prepared)
}
/// Shared entry for explicit rescrape/lock/crop updates. Unknown sidecar keys are retained.
/// Additional artwork replacements may be attached with `with_file`, with expected digest checks.
pub fn prepare(root: &Path, dir: &Path, code: &str, after: Metadata) -> io::Result<Preview> {
    let root = root.canonicalize()?;
    let dir = dir.canonicalize()?;
    let root_guard=crate::pipeline::RootGuard::capture(&root,false)?;
    let dir_guard=crate::pipeline::RootGuard::capture(&dir,false)?;
    let (before, mut raw, nfo, side, nfo_bytes) = load(&root, &dir, code)?;
    if after.code != code
        || after
            .locked_fields
            .iter()
            .any(|f| !crate::aggregate::FIELDS.contains(&f.as_str()))
    {
        return Err(io::Error::other("无效番号或锁定字段"));
    }
    let new = serde_json::to_value(&after)?;
    let projected_old=serde_json::to_value(crate::localization::project(&before,&before.nfo_language).metadata)?;
    let projected_new=serde_json::to_value(crate::localization::project(&after,&after.nfo_language).metadata)?;
    let mut changes:Vec<Change> = crate::aggregate::FIELDS
        .iter()
        .filter(|f| projected_old[**f] != projected_new[**f])
        .map(|f| Change {
            field: (*f).into(),
            before: projected_old[*f].clone(),
            after: projected_new[*f].clone(),
        })
        .collect();
    if before.favorite != after.favorite {
        changes.push(Change { field: "favorite".into(), before: before.favorite.into(), after: after.favorite.into() });
    }
    let media_guards=if before.placements!=after.placements {changes.push(Change{field:"placements".into(),before:serde_json::to_value(&before.placements)?,after:serde_json::to_value(&after.placements)?});after.placements.iter().filter(|p|p.mode==crate::placement::OrganizeMode::Symlink).map(|p|(p.source.clone(),p.source_proof.clone())).collect()}else{vec![]};
    for (k, v) in new.as_object().expect("metadata object") {
        raw[k] = v.clone();
    }
    let inputs = vec![
        ReplacementInput {
            path: nfo,
            expected: nfo_bytes.as_ref().map(|b| digest(b)),
            bytes: match &nfo_bytes {
                Some(bytes) => crate::nfo::merge_revision(
                    std::str::from_utf8(bytes).map_err(io::Error::other)?,
                    &before,
                    &after,
                )?
                .into_bytes(),
                None => crate::nfo::to_nfo(&after).into_bytes(),
            },
        },
        ReplacementInput {
            path: dir.join(SIDECAR),
            expected: side.as_ref().map(|b| digest(b)),
            bytes: serde_json::to_vec_pretty(&raw)?,
        },
    ];
    root_guard.verify()?;dir_guard.verify()?;
    Ok(Preview {
        root_guard,dir_guard,
        root,
        dir,
        code: code.into(),
        before,
        warnings: crate::localization::project(&after,&after.nfo_language).warnings,
        after,
        changes,
        inputs,
        media_guards,
    })
}
impl Preview {
    /// Explicit export rewrites only language-owned nodes, even if the selected language is unchanged.
    pub fn for_language_export(mut self)->io::Result<Self>{
        let input=self.inputs.first_mut().ok_or_else(||io::Error::other("缺少 NFO 输入"))?;
        let bytes=read(&input.path)?;
        if bytes.as_ref().map(|b|digest(b))!=input.expected {return Err(io::Error::other("预览期间 NFO 已变化"));}
        input.bytes=match bytes {Some(bytes)=>crate::nfo::merge_language_export(std::str::from_utf8(&bytes).map_err(io::Error::other)?,&self.after)?.into_bytes(),None=>crate::nfo::to_nfo(&self.after).into_bytes()};
        Ok(self)
    }

    /// Only content-addressed immutable original images beside this movie. Never arbitrary paths.
    pub fn with_original(mut self, bytes: Vec<u8>) -> io::Result<Self> {
        if crate::artwork::normalize(&bytes)? != bytes {
            return Err(io::Error::other("仅能保留规范静态原图"));
        }
        let hash = digest(&bytes);
        let path = self.dir.join(crate::artwork::original_name(&hash));
        within(&self.root, &path)?;
        let expected = current(&path)?;
        if (self
            .before
            .artwork
            .as_ref()
            .is_some_and(|a| a.digest == hash)
            && expected.is_none())
            || expected.as_ref().is_some_and(|value| value != &hash)
        {
            return Err(io::Error::other("原图资产被外部修改，请检查文件"));
        }
        self.inputs.push(ReplacementInput {
            path,
            expected,
            bytes,
        });
        Ok(self)
    }

    /// Bind a long-running candidate to the disk snapshot captured before network work.
    pub fn prepare_after(&self, after: Metadata) -> io::Result<Preview> {
        self.root_guard.verify()?;self.dir_guard.verify()?;
        for input in &self.inputs {
            if current(&input.path)? != input.expected {
                return Err(io::Error::other("更新期间磁盘内容已变化，请重新预览"));
            }
        }
        let preview = prepare(&self.root, &self.dir, &self.code, after)?;
        if self.inputs.len() != preview.inputs.len()
            || self
                .inputs
                .iter()
                .zip(&preview.inputs)
                .any(|(a, b)| a.path != b.path || a.expected != b.expected)
        {
            return Err(io::Error::other("更新期间磁盘内容已变化，请重新预览"));
        }
        Ok(preview)
    }

    pub fn with_file(
        mut self,
        path: PathBuf,
        expected: Option<String>,
        bytes: Vec<u8>,
    ) -> io::Result<Self> {
        within(&self.root, &path)?;
        let assets = crate::assets::Paths::for_movie(&self.dir, &self.before)?;
        if path.parent() != Some(self.dir.as_path())
            || (path != assets.poster && path != assets.fanart)
        {
            return Err(io::Error::other("仅支持影片目录内的海报修订"));
        }
        self.inputs.push(ReplacementInput {
            path,
            expected,
            bytes,
        });
        Ok(self)
    }
}
pub fn apply(preview: &Preview) -> io::Result<Outcome> {
    preview.root_guard.verify()?;preview.dir_guard.verify()?;
    for (source,proof) in &preview.media_guards {if crate::placement::source_proof(source)?!=*proof{return Err(io::Error::other("预览后链接来源已变化，请重新验证"))}}
    preview.root_guard.verify()?;preview.dir_guard.verify()?;
    // Reject stale previews before creating a journal. All targets are checked again by transaction.
    for input in &preview.inputs {
        within(&preview.root, &input.path)?;
        if current(&input.path)? != input.expected {
            return Err(io::Error::other("预览后磁盘内容已变化，请重新预览"));
        }
    }
    let mut journal = Journal::new(&preview.root);
    journal.items.push(JournalItem {
        code: preview.code.clone(),
        dir: preview.dir.clone(),
    });
    let changed = preview
        .inputs
        .iter()
        .filter(|input| input.expected.as_deref() != Some(digest(&input.bytes).as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Err(io::Error::other("没有需要保存的内容变化"));
    }
    let error = journal.replace_files_observed(&changed,&mut |_| {
        for (source,proof) in &preview.media_guards {if crate::placement::source_proof(source)?!=*proof{return Err(io::Error::other("链接来源已变化，请重新验证"))}}Ok(())
    }).err().map(|e| e.to_string());
    Ok(Outcome { journal, error })
}

/// Capture bytes and expected digest for an artwork replacement before presenting its preview.
pub fn snapshot_file(root: &Path, path: &Path) -> io::Result<(Option<String>, Option<Vec<u8>>)> {
    within(&root.canonicalize()?, path)?;
    let bytes = read(path)?;
    Ok((bytes.as_ref().map(|b| digest(b)), bytes))
}
