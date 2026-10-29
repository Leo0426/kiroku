//! Reconcile only indexed directories, without watching files or touching media.
use super::{Library, Result, VIDEO_EXT};
use std::{fs, io, path::Path};
use tokio_util::sync::CancellationToken;

impl Library {
    /// Remove entries whose directory has no video left. A remaining part/version,
    /// or even a dangling media link, keeps the entry. Never deletes disk files.
    /// All index changes roll back on cancellation, I/O error or lost root binding.
    pub fn prune_missing(&self, cancel: &CancellationToken, on_progress: &mut dyn FnMut(usize, usize)) -> Result<usize> {
        self.verify_binding()?;
        crate::append::check_cancel(cancel)?;
        let transaction = self.conn.unchecked_transaction()?;
        let total: usize = self.conn.query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_, i64>(0))? as usize;
        let mut after: Option<String> = None;
        let mut checked = 0;
        loop {
            crate::append::check_cancel(cancel)?;
            // Bound resident memory even for large libraries. Drop the statement
            // before calling the sole index writer (reindex_dir).
            let batch = self.prune_batch(after.as_deref())?;
            if batch.is_empty() { break; }
            for (code, relative) in batch {
                crate::append::check_cancel(cancel)?;
                self.verify_binding()?;
                let dir = self.abs(&relative);
                crate::append::confined_path(&self.root, &dir)?;
                if !has_video(&dir, cancel)? {
                    self.reindex_dir_cancellable(&dir, cancel)?;
                }
                after = Some(code);
                checked += 1;
                on_progress(checked, total);
            }
        }
        self.verify_binding()?;
        crate::append::check_cancel(cancel)?;
        let remaining: usize = self.conn.query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_, i64>(0))? as usize;
        transaction.commit()?;
        Ok(total.saturating_sub(remaining))
    }

    fn prune_batch(&self, after: Option<&str>) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(prune_page_sql(after.is_some()))?;
        let args = after.into_iter().collect::<Vec<_>>();
        let batch = statement.query_map(rusqlite::params_from_iter(args), |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(batch)
    }
}

fn prune_page_sql(after: bool) -> &'static str {
    if after {
        "SELECT code,dir FROM movie WHERE code>?1 ORDER BY code LIMIT 256"
    } else {
        "SELECT code,dir FROM movie ORDER BY code LIMIT 256"
    }
}

fn has_video(dir: &Path, cancel: &CancellationToken) -> io::Result<bool> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        crate::append::check_cancel(cancel)?;
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("._") { continue; }
        let extension = Path::new(name.as_ref()).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        if VIDEO_EXT.contains(&extension.as_str()) {
            let kind = entry.file_type()?;
            // Do not follow a link: its source may just be on an offline volume.
            if kind.is_file() || kind.is_symlink() { return Ok(true); }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_prune_pages_seek_and_keep_complete_ordered_membership() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open(root.path()).unwrap();
        let transaction = library.conn.unchecked_transaction().unwrap();
        for i in 0..1000 {
            library.conn.execute("INSERT INTO movie(code,dir,added_at,meta_json) VALUES(?1,?1,0,'{}')", [format!("SMPL-{i:06}")]).unwrap();
        }
        transaction.commit().unwrap();
        let plan = library.conn.prepare(&format!("EXPLAIN QUERY PLAN {}", prune_page_sql(true))).unwrap()
            .query_map(["SMPL-000800"], |r| r.get::<_, String>(3)).unwrap().collect::<std::result::Result<Vec<_>, _>>().unwrap();
        assert!(plan.iter().any(|p| p.contains("SEARCH movie") && p.contains("code>?")), "{plan:?}");
        let mut after = None;
        let mut codes = vec![];
        loop {
            let page = library.prune_batch(after.as_deref()).unwrap();
            if page.is_empty() { break; }
            assert!(page.len() <= 256);
            after = page.last().map(|p| p.0.clone());
            codes.extend(page.into_iter().map(|p| p.0));
        }
        assert_eq!(codes, (0..1000).map(|i| format!("SMPL-{i:06}")).collect::<Vec<_>>());
    }
}
