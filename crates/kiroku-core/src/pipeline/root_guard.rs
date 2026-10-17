//! Root identities survive preview; explicit creation atomically binds its new root.
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[derive(Debug)]
struct PendingRoot {
    path: PathBuf,
    identity: Mutex<Option<same_file::Handle>>,
}
#[derive(Debug, Clone)]
pub struct RootGuard {
    path: PathBuf,
    identity: Arc<same_file::Handle>,
    pending: Option<Arc<PendingRoot>>,
}
impl RootGuard {
    pub fn capture(root: &Path, create: bool) -> io::Result<Self> {
        let requested = std::path::absolute(root)?;
        let mut path = requested.clone();
        let missing = match fs::symlink_metadata(&path) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(error) => return Err(error),
        };
        if missing && !create {
            return Err(io::Error::other("媒体库目录不可用；新建库需明确选择创建"));
        }
        if !missing && !requested.is_dir() {
            return Err(io::Error::other("库路径不是可用目录"));
        }
        while !path.exists() {
            path = path
                .parent()
                .ok_or_else(|| io::Error::other("无可用库父目录"))?
                .to_path_buf();
        }
        if !path.is_dir() {
            return Err(io::Error::other("库路径不是目录"));
        }
        let pending = missing.then(|| {
            Arc::new(PendingRoot {
                path: requested,
                identity: Mutex::new(None),
            })
        });
        let guard = Self {
            identity: Arc::new(same_file::Handle::from_path(&path)?),
            path,
            pending,
        };
        guard.verify()?;
        Ok(guard)
    }
    fn verify_parent(&self) -> io::Result<()> {
        if same_file::Handle::from_path(&self.path)? != *self.identity {
            return Err(io::Error::other("库目录身份已变化，请重新预览"));
        }
        Ok(())
    }
    pub fn verify(&self) -> io::Result<()> {
        self.verify_parent()?;
        if let Some(pending) = &self.pending {
            let identity = pending
                .identity
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(identity) = identity.as_ref() {
                if same_file::Handle::from_path(&pending.path)? != *identity {
                    return Err(io::Error::other("新库目录身份已变化，请重新预览"));
                }
            } else {
                match fs::symlink_metadata(&pending.path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                    Ok(_) => return Err(io::Error::other("新库路径已出现，请重新预览")),
                }
            }
        }
        Ok(())
    }
    /// Runs on the blocking worker after image preparation and before media I/O.
    pub(crate) fn initialize(&self) -> io::Result<()> {
        self.verify_parent()?;
        let Some(pending) = &self.pending else {
            return self.verify();
        };
        let mut identity = pending
            .identity
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(bound) = identity.as_ref() {
            if same_file::Handle::from_path(&pending.path)? != *bound {
                return Err(io::Error::other("新库目录身份已变化，请重新预览"));
            }
            return Ok(());
        }
        let path = crate::append::confined_path(&self.path, &pending.path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.verify_parent()?;
        crate::append::confined_path(&self.path, &pending.path)?;
        fs::create_dir(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::other("新库路径已出现，请重新预览")
            } else {
                error
            }
        })?;
        let bound = same_file::Handle::from_path(&path)?;
        self.verify_parent()?;
        crate::append::confined_path(&self.path, &pending.path)?;
        if same_file::Handle::from_path(&pending.path)? != bound {
            return Err(io::Error::other("新库目录身份已变化，请重新预览"));
        }
        *identity = Some(bound);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn dangling_library_alias_is_not_a_missing_library_creation_intent() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("alias");
        std::os::unix::fs::symlink(parent.path().join("offline"), &root).unwrap();
        assert!(RootGuard::capture(&root, false).is_err());
        assert!(RootGuard::capture(&root, true).is_err());
    }

    #[test]
    fn explicit_creation_is_exclusive_and_then_bound_to_its_own_root() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("nested/library");
        let guard = RootGuard::capture(&root, true).unwrap();
        let clone = guard.clone();
        guard.verify().unwrap();
        guard.initialize().unwrap();
        clone.initialize().unwrap();
        clone.verify().unwrap();
        fs::rename(&root, parent.path().join("offline")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(guard.verify().is_err());
        assert!(clone.initialize().is_err());
        assert!(!root.join(".kiroku").exists());
        let other = parent.path().join("other");
        let guard = RootGuard::capture(&other, true).unwrap();
        fs::create_dir(&other).unwrap();
        assert!(guard.initialize().is_err());
    }
}
