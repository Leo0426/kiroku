//! Root-bound readonly connections; WAL readers never take the writer's mutex.
use super::{Facets, Library, MovieDetail, Page, Query, Result};
use crate::metadata::SourcedScore;
use rusqlite::{Connection, OpenFlags};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub struct LibraryReadHandle {
    pub(super) root: PathBuf,
    pub(super) root_identity: Arc<same_file::Handle>,
    pub(super) db_identity: Arc<same_file::Handle>,
    pub(super) read_cache: Arc<super::query::ReadCache>,
}

impl LibraryReadHandle {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub(super) fn verify(&self) -> io::Result<()> {
        if same_file::Handle::from_path(&self.root)? != *self.root_identity {
            return Err(io::Error::other("库目录身份已变化，请重新打开媒体库"));
        }
        if !std::fs::symlink_metadata(Library::db_path(&self.root))?.is_file() {
            return Err(io::Error::other("索引文件不是普通文件，请重新打开媒体库"));
        }
        if same_file::Handle::from_path(Library::db_path(&self.root))? != *self.db_identity {
            return Err(io::Error::other("索引文件身份已变化，请重新打开媒体库"));
        }
        Ok(())
    }
    /// Does not create directories, a database, run migration, or rebuild rows.
    pub fn open(&self) -> Result<LibraryReader> {
        self.verify()?;
        let connection = Connection::open_with_flags(
            Library::db_path(&self.root),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.execute_batch("PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA query_only=ON;")?;
        self.verify()?;
        Ok(LibraryReader {
            library: Library {
                root: self.root.clone(),
                root_identity: self.root_identity.clone(),
                db_identity: self.db_identity.clone(),
                conn: connection,
                read_cache: self.read_cache.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn replacement_between_binding_and_open_is_rejected_before_initialization() {
        let root = tempfile::tempdir().unwrap();
        let writer = Library::open(root.path()).unwrap();
        let binding = writer.reader_handle();
        drop(writer);
        let database = Library::db_path(root.path());
        std::fs::rename(&database, root.path().join("original.db")).unwrap();
        std::fs::write(&database, b"").unwrap();
        assert!(Library::open_bound(binding).is_err());
        assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);
    }

    #[test]
    fn readonly_connection_cannot_mutate_even_when_query_only_is_disabled() {
        let root = tempfile::tempdir().unwrap();
        let writer = Library::open(root.path()).unwrap();
        let reader = writer.reader_handle().open().unwrap();
        assert!(reader.library.conn.is_readonly("main").unwrap());
        reader
            .library
            .conn
            .execute_batch("PRAGMA query_only=OFF;")
            .unwrap();
        assert!(reader
            .library
            .conn
            .execute("DELETE FROM movie", [])
            .is_err());
        assert_eq!(reader.list(&Query::default()).unwrap().total, 0);
    }
    #[cfg(unix)]
    #[test]
    fn dropped_or_replaced_roots_and_databases_are_not_recreated_by_reads() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("library");
        let writer = Library::open(&root).unwrap();
        let handle = writer.reader_handle();
        let reader = handle.open().unwrap();
        let offline = parent.path().join("offline");
        std::fs::rename(&root, &offline).unwrap();
        assert!(reader.list(&Query::default()).is_err());
        assert!(handle.open().is_err());
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        assert!(handle.open().is_err());
        assert!(!root.join(".kiroku").exists());
        std::fs::remove_dir(&root).unwrap();
        std::fs::rename(&offline, &root).unwrap();
        assert_eq!(reader.list(&Query::default()).unwrap().total, 0);
        let database = Library::db_path(&root);
        let backup = root.join("old.db");
        std::fs::rename(&database, &backup).unwrap();
        assert!(handle.open().is_err());
        assert!(!database.exists());
        std::fs::copy(&backup, &database).unwrap();
        assert!(reader.facets().is_err());
        assert!(handle.open().is_err());
        assert!(writer.reindex_dir(&root).is_err());
    }
}

/// Deliberately exposes no index writer or raw connection.
pub struct LibraryReader {
    library: Library,
}
impl LibraryReader {
    pub fn root(&self) -> &Path {
        self.library.root()
    }
    fn snapshot<T>(&self, read: impl FnOnce(&Library) -> Result<T>) -> Result<T> {
        self.library.verify_binding()?;
        // A logical query spans one committed snapshot, including COUNT/page,
        // metadata/files, and all facet groups. Drop also rolls back on errors.
        let transaction = self.library.conn.unchecked_transaction()?;
        let value = read(&self.library)?;
        self.library.verify_binding()?;
        transaction.commit()?;
        Ok(value)
    }
    pub fn list(&self, query: &Query) -> Result<Page> {
        self.snapshot(|library| library.list(query))
    }
    pub fn get(&self, code: &str) -> Result<Option<MovieDetail>> {
        self.snapshot(|library| library.get(code))
    }
    pub fn facets(&self) -> Result<Facets> {
        self.snapshot(Library::facets)
    }
    pub fn scores(&self, code: &str) -> Result<Vec<SourcedScore>> {
        self.snapshot(|library| library.scores(code))
    }
}
