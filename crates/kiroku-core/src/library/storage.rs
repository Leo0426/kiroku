//! Network-volume indexes are local, disposable caches. Media and journals stay on the volume.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn db_path(root: &Path) -> PathBuf {
    let Some(volume) = network_identity(root) else { return crate::journal::dir(root).join("library.db") };
    let normalized = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hash = Sha256::new();
    hash.update(normalized.as_os_str().as_encoded_bytes());
    hash.update(volume);
    #[cfg(unix)]
    if let Ok(metadata) = fs::metadata(root) {
        use std::os::unix::fs::MetadataExt;
        hash.update(metadata.ino().to_le_bytes());
    }
    crate::settings::Settings::default_path().parent().unwrap().join("library-index")
        .join(format!("{:x}", hash.finalize())).join("library.db")
}

#[cfg(target_os = "macos")]
fn network_identity(root: &Path) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(root.as_os_str().as_bytes()).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // statfs writes the complete struct only on success. Mount source is stable
    // across remounts; the transient device number is deliberately not a key.
    let info = unsafe {
        if libc::statfs(path.as_ptr(), info.as_mut_ptr()) != 0 { return None; }
        info.assume_init()
    };
    if info.f_flags & libc::MNT_LOCAL as u32 != 0 { return None; }
    Some(info.f_mntfromname.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect())
}
#[cfg(not(target_os = "macos"))]
fn network_identity(_: &Path) -> Option<Vec<u8>> { None }

/// Copy a committed SQLite snapshot, never raw database bytes. Publish atomically
/// only after completion. A busy/corrupt legacy index stays intact and can retry.
pub(super) fn prepare(root: &Path, path: &Path) -> Result<()> {
    let legacy = crate::journal::dir(root).join("library.db");
    if path == legacy { return Ok(()); }
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    if network_identity(parent).is_some() { return Err(io::Error::other("网络盘索引缓存目录必须位于本机磁盘").into()); }
    if path.try_exists()? { return Ok(()); }
    match fs::symlink_metadata(&legacy) {
        Ok(m) if !m.is_file() => return Err(io::Error::other("旧索引不是普通文件").into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        _ => {}
    }
    let source_identity = same_file::Handle::from_path(&legacy)?;
    let source = Connection::open_with_flags(&legacy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(std::time::Duration::from_millis(250))?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut target = Connection::open(temp.path())?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
        let started = std::time::Instant::now();
        loop {
            match backup.step(256)? {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More if started.elapsed() < std::time::Duration::from_secs(10) => {},
                _ => return Err(io::Error::other("旧索引正在使用或快照超时，请稍后重新打开媒体库").into()),
            }
        }
    }
    drop(target);
    if same_file::Handle::from_path(&legacy)? != source_identity {
        return Err(io::Error::other("复制期间旧索引身份变化，请重新打开媒体库").into());
    }
    temp.as_file().sync_all()?;
    match temp.persist_noclobber(path) {
        Ok(_) => {},
        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(e.error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_snapshot_includes_committed_wal_and_is_not_reimported() {
        let source_root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let source = Library::open(source_root.path()).unwrap();
        let movie = source_root.path().join("TEST-001");
        fs::create_dir(&movie).unwrap();
        fs::write(movie.join("TEST-001.mp4"), b"neutral video").unwrap();
        fs::write(movie.join("movie.nfo"), "<movie><num>TEST-001</num><title>Neutral example</title></movie>").unwrap();
        source.reindex_dir(&movie).unwrap();
        let dest = cache.path().join("library.db");
        prepare(source_root.path(), &dest).unwrap();
        let copied = Connection::open(&dest).unwrap();
        assert_eq!(copied.query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
        source.conn.execute("DELETE FROM movie", []).unwrap();
        prepare(source_root.path(), &dest).unwrap();
        assert_eq!(copied.query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
    }

    #[test]
    fn corrupt_legacy_does_not_publish_an_empty_cache() {
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        fs::create_dir(crate::journal::dir(root.path())).unwrap();
        let legacy = crate::journal::dir(root.path()).join("library.db");
        fs::write(&legacy, b"not a database").unwrap();
        let dest = cache.path().join("library.db");
        assert!(prepare(root.path(), &dest).is_err());
        assert!(!dest.exists());
        assert_eq!(fs::read(legacy).unwrap(), b"not a database");
    }
}
