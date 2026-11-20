//! Reconcile the index against movie directories without changing media files.
use super::{Library, MovieDirectory, Result};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Default, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshStats {
    pub removed: usize,
    pub relocated: usize,
    pub refreshed: usize,
    pub ambiguous: usize,
    pub missing_codes: Vec<String>,
}

#[derive(Default)]
enum Location {
    #[default]
    Missing,
    Single(PathBuf),
    Ambiguous,
}

impl Library {
    pub fn prune_missing(
        &self,
        cancel: &CancellationToken,
        on_progress: &mut dyn FnMut(usize, usize),
    ) -> Result<usize> {
        Ok(self.refresh_index(None, cancel, on_progress)?.removed)
    }

    /// Reconnect a moved movie only when its on-disk code identifies a single
    /// directory. An empty approval list is a non-destructive preview. Older
    /// explicit callers may omit it to remove all confirmed missing rows.
    pub fn refresh_index(
        &self,
        remove_missing: Option<&[String]>,
        cancel: &CancellationToken,
        on_progress: &mut dyn FnMut(usize, usize),
    ) -> Result<RefreshStats> {
        self.verify_binding()?;
        crate::append::check_cancel(cancel)?;
        let transaction = self.conn.unchecked_transaction()?;
        let total = self
            .conn
            .query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_, i64>(0))? as usize;
        let mut missing = BTreeMap::new();
        let mut stats = RefreshStats::default();
        let mut after: Option<String> = None;
        let mut checked = 0;
        loop {
            crate::append::check_cancel(cancel)?;
            let batch = self.prune_batch(after.as_deref())?;
            if batch.is_empty() {
                break;
            }
            for row in batch {
                crate::append::check_cancel(cancel)?;
                self.verify_binding()?;
                let dir = self.abs(&row.dir);
                crate::append::confined_path(&self.root, &dir)?;
                let contents = MovieDirectory::read(&dir, cancel)?;
                if contents.videos.is_empty() {
                    missing.insert(row.code.clone(), (dir, Location::Missing));
                } else if self.refresh_artwork(&row, &dir, &contents, cancel)? {
                    stats.refreshed += 1;
                }
                after = Some(row.code);
                checked += 1;
                on_progress(checked, total);
            }
        }

        if !missing.is_empty() {
            self.verify_binding()?;
            let directories = super::rebuild::Directories::capture(&self.root, cancel)?;
            directories.visit(cancel, &mut |_, path| {
                let relative = super::rel_str(
                    path.strip_prefix(&self.root)
                        .expect("captured directory within library"),
                );
                let occupied = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM movie WHERE dir=?1)",
                    [&relative],
                    |row| row.get::<_, bool>(0),
                )?;
                if occupied {
                    return Ok(true);
                }
                crate::append::confined_path(&self.root, path)?;
                let contents = MovieDirectory::read(path, cancel)?;
                if !contents.videos.is_empty() {
                    if let Some(meta) = contents.metadata(path)? {
                        if let Some((_, location)) = missing.get_mut(&meta.code) {
                            *location = match location {
                                Location::Missing => Location::Single(path.to_owned()),
                                _ => Location::Ambiguous,
                            };
                        }
                    }
                }
                Ok(true)
            })?;
        }

        let approved = remove_missing.map(|codes| codes.iter().collect::<HashSet<_>>());
        for (code, (dir, location)) in missing {
            crate::append::check_cancel(cancel)?;
            self.verify_binding()?;
            crate::append::confined_path(&self.root, &dir)?;
            // A concurrent restore wins, including a linked video on an offline volume.
            if !MovieDirectory::read(&dir, cancel)?.videos.is_empty() {
                continue;
            }
            match location {
                Location::Single(new_dir) => {
                    self.reindex_expected(&new_dir, &code, cancel)?;
                    stats.relocated += 1;
                }
                Location::Ambiguous => stats.ambiguous += 1,
                Location::Missing if approved.as_ref().is_none_or(|codes| codes.contains(&code)) => {
                    let result = self.reindex_dir_cancellable(&dir, cancel)?;
                    if result.is_none() {
                        stats.removed += 1;
                    } else if result.as_deref() != Some(&code) {
                        return Err(io::Error::other("影片目录已变化，请重新刷新").into());
                    }
                }
                Location::Missing => stats.missing_codes.push(code),
            }
        }
        self.verify_binding()?;
        crate::append::check_cancel(cancel)?;
        transaction.commit()?;
        Ok(stats)
    }

    fn reindex_expected(&self, dir: &Path, code: &str, cancel: &CancellationToken) -> Result<()> {
        if self.reindex_dir_cancellable(dir, cancel)?.as_deref() != Some(code) {
            return Err(io::Error::other("影片目录已变化，请重新刷新").into());
        }
        Ok(())
    }

    fn refresh_artwork(
        &self,
        row: &IndexedMovie,
        dir: &Path,
        contents: &MovieDirectory,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let Some(meta) = contents.metadata(dir)? else {
            return Ok(false);
        };
        if meta.code != row.code {
            return Ok(false);
        }
        let (poster, fanart) = contents.artwork(&meta);
        if poster == row.poster.as_ref().map(|rel| self.abs(rel)).as_deref()
            && self.image_version(poster, cancel)? == row.poster_version
            && self.image_version(fanart, cancel)? == row.fanart_version
        {
            return Ok(false);
        }
        self.reindex_expected(dir, &row.code, cancel)?;
        Ok(true)
    }

    fn prune_batch(&self, after: Option<&str>) -> Result<Vec<IndexedMovie>> {
        let mut statement = self.conn.prepare(prune_page_sql(after.is_some()))?;
        let args = after.into_iter().collect::<Vec<_>>();
        let batch = statement
            .query_map(rusqlite::params_from_iter(args), |r| {
                Ok(IndexedMovie {
                    code: r.get(0)?,
                    dir: r.get(1)?,
                    poster: r.get(2)?,
                    poster_version: r.get(3)?,
                    fanart_version: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(batch)
    }
}

struct IndexedMovie {
    code: String,
    dir: String,
    poster: Option<String>,
    poster_version: Option<String>,
    fanart_version: Option<String>,
}

fn prune_page_sql(after: bool) -> &'static str {
    if after {
        "SELECT code,dir,poster,poster_version,fanart_version FROM movie WHERE code>?1 ORDER BY code LIMIT 256"
    } else {
        "SELECT code,dir,poster,poster_version,fanart_version FROM movie ORDER BY code LIMIT 256"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(dir: &Path, code: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let meta = crate::Metadata {
            code: code.into(),
            asset_stem: Some(format!("{code}-original")),
            ..Default::default()
        };
        std::fs::write(dir.join(crate::organize::SIDECAR), serde_json::to_vec(&meta).unwrap()).unwrap();
        std::fs::write(dir.join(format!("{code}.mp4")), b"synthetic video").unwrap();
    }

    #[test]
    fn refresh_reconnects_moved_movies_repairs_local_art_and_removes_only_reviewed_missing_codes() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open(root.path()).unwrap();
        let codes = ["AAA-111", "BBB-222", "CCC-333", "DDD-444", "EEE-555"];
        for code in codes {
            let path = root.path().join(code);
            place(&path, code);
            library.reindex_dir(&path).unwrap();
        }
        let relocated = root.path().join("renamed/edition");
        std::fs::create_dir(relocated.parent().unwrap()).unwrap();
        std::fs::rename(root.path().join(codes[0]), &relocated).unwrap();
        std::fs::write(
            relocated.join("MOVIE.NFO"),
            b"<movie><num>AAA-111</num><title>Example</title></movie>",
        )
        .unwrap();
        std::fs::write(relocated.join(crate::organize::SIDECAR), b"invalid sidecar").unwrap();
        std::fs::write(relocated.join("FOLDER.JPEG"), b"synthetic poster").unwrap();
        std::fs::write(relocated.join("AAA-111-backdrop.PNG"), b"synthetic background").unwrap();
        std::fs::write(root.path().join("BBB-222/BBB-222-cover.WEBP"), b"cover").unwrap();
        for code in [codes[2], codes[3]] {
            std::fs::remove_file(root.path().join(code).join(format!("{code}.mp4"))).unwrap();
        }
        place(&root.path().join("unindexed"), "ZZZ-999");
        let refresh = |approval| {
            library
                .refresh_index(approval, &CancellationToken::new(), &mut |_, _| {})
                .unwrap()
        };
        let preview = refresh(Some(&[]));
        assert_eq!((preview.relocated, preview.refreshed, preview.removed), (1, 1, 0));
        assert_eq!(preview.missing_codes, ["CCC-333", "DDD-444"]);
        let detail = library.get(codes[0]).unwrap().unwrap();
        assert_eq!(Path::new(&detail.dir), relocated);
        assert_eq!(
            detail.card.poster.as_deref().map(Path::new),
            Some(relocated.join("FOLDER.JPEG").as_path())
        );
        assert_eq!(
            detail.fanart.as_deref().map(Path::new),
            Some(relocated.join("AAA-111-backdrop.PNG").as_path())
        );
        assert!(library.get("ZZZ-999").unwrap().is_none());
        // One reviewed movie returns and another disappears before confirmation.
        std::fs::write(root.path().join("DDD-444/DDD-444.mp4"), b"returned").unwrap();
        std::fs::remove_file(root.path().join("EEE-555/EEE-555.mp4")).unwrap();
        let confirmed = refresh(Some(&preview.missing_codes));
        assert_eq!(confirmed.removed, 1);
        assert_eq!(confirmed.missing_codes, ["EEE-555"]);
        assert!(library.get(codes[2]).unwrap().is_none());
        assert!(library.get(codes[3]).unwrap().is_some());
        assert!(library.get(codes[4]).unwrap().is_some());
        assert!(root.path().join("CCC-333/kiroku.json").is_file());
    }

    #[test]
    fn refresh_keeps_ambiguous_editions_and_rechecks_the_match_on_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open(root.path()).unwrap();
        let original = root.path().join("old");
        place(&original, "AAA-111");
        library.reindex_dir(&original).unwrap();
        std::fs::remove_dir_all(&original).unwrap();
        let preview = library
            .refresh_index(Some(&[]), &CancellationToken::new(), &mut |_, _| {})
            .unwrap();
        assert_eq!(preview.missing_codes, ["AAA-111"]);
        for name in ["new-a", "new-b"] {
            place(&root.path().join(name), "AAA-111");
        }
        let ambiguous = library
            .refresh_index(Some(&preview.missing_codes), &CancellationToken::new(), &mut |_, _| {})
            .unwrap();
        assert_eq!((ambiguous.ambiguous, ambiguous.removed, ambiguous.relocated), (1, 0, 0));
        assert!(ambiguous.missing_codes.is_empty());
        assert_eq!(library.get("AAA-111").unwrap().unwrap().dir, original.to_string_lossy());
        std::fs::remove_dir_all(root.path().join("new-b")).unwrap();
        let reconnected = library
            .refresh_index(Some(&preview.missing_codes), &CancellationToken::new(), &mut |_, _| {})
            .unwrap();
        assert_eq!((reconnected.relocated, reconnected.removed), (1, 0));
        assert_eq!(
            library.get("AAA-111").unwrap().unwrap().dir,
            root.path().join("new-a").to_string_lossy()
        );
    }

    #[test]
    fn refresh_resolves_artwork_by_role_and_owner_in_the_movie_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sample");
        let library = Library::open(root.path()).unwrap();
        place(&dir, "AAA-111");
        library.reindex_dir(&dir).unwrap();
        for name in ["BBB-222-poster.jpg", "still-01.jpg", "actor.png", "fanart.jpg"] {
            std::fs::write(dir.join(name), b"fixture").unwrap();
        }
        let poster = || {
            library
                .refresh_index(Some(&[]), &CancellationToken::new(), &mut |_, _| {})
                .unwrap();
            library.get("AAA-111").unwrap().unwrap().card.poster.map(PathBuf::from)
        };
        assert_eq!(poster(), None);
        std::fs::write(dir.join("AAA-111.PNG"), b"same stem").unwrap();
        assert_eq!(poster(), Some(dir.join("AAA-111.PNG")));
        std::fs::write(dir.join("Cover.JPEG"), b"generic").unwrap();
        assert_eq!(poster(), Some(dir.join("Cover.JPEG")));
        std::fs::write(dir.join("BBB-222.mp4"), b"another movie").unwrap();
        assert_eq!(poster(), Some(dir.join("AAA-111.PNG")));
        std::fs::write(dir.join("AAA-111-folder.JPG"), b"named").unwrap();
        assert_eq!(poster(), Some(dir.join("AAA-111-folder.JPG")));
        std::fs::create_dir(dir.join(".actors")).unwrap();
        std::fs::write(dir.join(".actors/poster.jpg"), b"portrait").unwrap();
        std::fs::remove_file(dir.join("AAA-111.PNG")).unwrap();
        std::fs::remove_file(dir.join("AAA-111-folder.JPG")).unwrap();
        assert_eq!(poster(), None);
        // An excluded generic cover must not contribute an image revision.
        let meta = crate::Metadata {
            code: "AAA-111".into(),
            ..Default::default()
        };
        std::fs::write(dir.join(crate::organize::SIDECAR), serde_json::to_vec(&meta).unwrap()).unwrap();
        std::fs::write(dir.join("poster.jpg"), b"unowned generic").unwrap();
        library.reindex_dir(&dir).unwrap();
        let detail = library.get("AAA-111").unwrap().unwrap();
        assert!(detail.card.poster.is_none() && detail.card.poster_version.is_none());
        assert!(detail.fanart.is_none() && detail.fanart_version.is_none());
        assert_eq!(
            library
                .refresh_index(Some(&[]), &CancellationToken::new(), &mut |_, _| {})
                .unwrap()
                .refreshed,
            0
        );
    }
    #[test]
    fn later_prune_pages_seek_and_keep_complete_ordered_membership() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open(root.path()).unwrap();
        let transaction = library.conn.unchecked_transaction().unwrap();
        for i in 0..1000 {
            library
                .conn
                .execute(
                    "INSERT INTO movie(code,dir,added_at,meta_json) VALUES(?1,?1,0,'{}')",
                    [format!("SMPL-{i:06}")],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        let plan = library
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", prune_page_sql(true)))
            .unwrap()
            .query_map(["SMPL-000800"], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plan.iter().any(|p| p.contains("SEARCH movie") && p.contains("code>?")),
            "{plan:?}"
        );
        let mut after = None;
        let mut codes = vec![];
        loop {
            let page = library.prune_batch(after.as_deref()).unwrap();
            if page.is_empty() {
                break;
            }
            assert!(page.len() <= 256);
            after = page.last().map(|p| p.code.clone());
            codes.extend(page.into_iter().map(|p| p.code));
        }
        assert_eq!(codes, (0..1000).map(|i| format!("SMPL-{i:06}")).collect::<Vec<_>>());
    }
}
