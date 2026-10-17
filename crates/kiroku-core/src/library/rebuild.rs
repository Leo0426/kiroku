//! A job-private disk manifest; collecting movie directories never retains a Vec per movie.
use super::{Result, SIDECAR};
use rusqlite::{params, Connection};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

pub(super) struct Directories {
    connection: Connection,
    _scratch: tempfile::TempDir,
}
impl Directories {
    pub fn capture(root: &Path, cancel: &CancellationToken) -> Result<Self> {
        let scratch = tempfile::Builder::new()
            .prefix("kiroku-rebuild-")
            .tempdir()?;
        let connection = Connection::open(scratch.path().join("directories.db"))?;
        // Disposable staging only: no durability requirement, 1 MiB page cache,
        // 512 MiB disk ceiling. SQLITE_FULL is a normal failed rebuild, never truncation.
        connection.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-1024; PRAGMA max_page_count=131072; CREATE TABLE directory(sort_key BLOB PRIMARY KEY, path BLOB NOT NULL) WITHOUT ROWID;")?;
        let manifest = Self {
            connection,
            _scratch: scratch,
        };
        let transaction = manifest.connection.unchecked_transaction()?;
        {
            let mut insert = manifest
                .connection
                .prepare("INSERT OR IGNORE INTO directory VALUES(?1,?2)")?;
            for entry in WalkDir::new(root)
                .into_iter()
                .filter_entry(|entry| entry.file_name() != ".kiroku")
            {
                if cancel.is_cancelled() {
                    break;
                }
                let entry = entry.map_err(|error| io::Error::other(error.to_string()))?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy();
                if name == SIDECAR || name.to_ascii_lowercase().ends_with(".nfo") {
                    if let Some(directory) = entry.path().parent() {
                        insert.execute(params![sort_key(directory),encode(directory)])?;
                    }
                }
            }
        }
        transaction.commit()?;
        Ok(manifest)
    }
    #[cfg(test)]
    pub fn measurement(&self) -> (u64, i64) {
        (
            std::fs::metadata(self._scratch.path().join("directories.db"))
                .unwrap()
                .len(),
            self.connection
                .query_row("PRAGMA cache_size", [], |row| row.get(0))
                .unwrap(),
        )
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM directory", [], |row| {
                row.get::<_, i64>(0)
            })? as usize)
    }
    pub fn visit(
        &self,
        cancel: &CancellationToken,
        action: &mut dyn FnMut(usize, &Path) -> Result<bool>,
    ) -> Result<()> {
        let mut statement = self
            .connection
            .prepare("SELECT path FROM directory ORDER BY sort_key")?;
        let mut rows = statement.query([])?;
        let mut index = 0;
        while !cancel.is_cancelled() {
            let Some(row) = rows.next()? else {
                break;
            };
            let directory = decode(row.get(0)?)?;
            index += 1;
            if !action(index, &directory)? {
                break;
            }
        }
        Ok(())
    }
}

// Components, rather than whole path bytes, determine PathBuf::Ord. Filename
// components cannot contain NUL, so a zero terminator preserves prefix ordering.
// as_encoded_bytes follows the native OsStr representation, including Windows WTF-8.
fn sort_key(path:&Path)->Vec<u8> {
    let mut key=Vec::new();
    for component in path.components() {
        key.extend_from_slice(component.as_os_str().as_encoded_bytes());key.push(0);
    }
    key
}

#[cfg(unix)]
fn encode(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(unix)]
fn decode(bytes: Vec<u8>) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(bytes).into())
}
#[cfg(windows)]
fn encode(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
#[cfg(windows)]
fn decode(bytes: Vec<u8>) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    if bytes.len() % 2 != 0 {
        return Err(io::Error::other("invalid staged path"));
    }
    Ok(std::ffi::OsString::from_wide(
        &bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .into())
}
#[cfg(not(any(unix, windows)))]
fn encode(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}
#[cfg(not(any(unix, windows)))]
fn decode(bytes: Vec<u8>) -> io::Result<PathBuf> {
    Ok(String::from_utf8(bytes).map_err(io::Error::other)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sorting_keys_match_native_path_component_order() {
        let mut paths=["a/movie","a-dup","a","a.b","z","Ā","é","🙂"].map(|name|Path::new("root").join(name)).to_vec();
        let mut expected=paths.clone();expected.sort();paths.sort_by_key(|path|sort_key(path));assert_eq!(paths,expected);
    }

    #[cfg(unix)]
    #[test]
    fn staged_path_codec_preserves_non_utf8_names() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'd', 0xff]));
        assert_eq!(decode(encode(&path)).unwrap(), path);
    }

    #[test]
    fn disk_manifest_deduplicates_and_releases_scratch_and_preserves_raw_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let name = std::ffi::OsString::from("目录");
        let directory = temporary.path().join(name);
        std::fs::create_dir(&directory).unwrap();
        for name in ["kiroku.json", "movie.nfo", "part.nfo"] {
            std::fs::write(directory.join(name), b"fixture").unwrap();
        }
        let cancel = CancellationToken::new();
        let manifest = Directories::capture(temporary.path(), &cancel).unwrap();
        assert_eq!(manifest.len().unwrap(), 1);
        let mut visited = vec![];
        manifest
            .visit(&cancel, &mut |_, path| {
                visited.push(path.to_path_buf());
                Ok(true)
            })
            .unwrap();
        assert_eq!(visited, vec![directory]);
        let scratch = manifest._scratch.path().to_path_buf();
        drop(manifest);
        assert!(!scratch.exists());
    }
}
