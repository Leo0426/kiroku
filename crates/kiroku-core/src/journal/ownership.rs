//! Filesystem semantics of durable ownership proofs. Journal controls ordering;
//! this module keeps local links, retained copies and remote anchors consistent.
use super::Proof;
use crate::organize::{self, Op};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};
pub(super) fn file_digest_checked(path: &Path, cancel: &dyn Fn() -> bool) -> io::Result<String> {
    if let Some(result) = crate::nas_ops::digest(path, cancel) {
        return result;
    }
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        crate::append::check_stopped(cancel)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        #[cfg(test)]
        crate::append::hash_reads::record(n);
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn verified(path: &Path, proof: &Proof) -> io::Result<bool> {
    verified_checked(path, proof, &|| false)
}
pub(super) fn verified_checked(
    path: &Path,
    proof: &Proof,
    cancel: &dyn Fn() -> bool,
) -> io::Result<bool> {
    crate::append::check_stopped(cancel)?;
    if let Some(source) = &proof.link {
        return Ok(
            fs::symlink_metadata(path)?.file_type().is_symlink() && fs::read_link(path)? == *source
        );
    }
    if proof.published_identity.is_none() {
        if let Some(result) = crate::nas_ops::digest_remote(path, cancel) {
            return result.map(|digest| digest == proof.digest);
        }
    }
    let meta = match crate::nas_ops::metadata(path, cancel) {
        Some(result) => result?,
        None => fs::symlink_metadata(path)?,
    };
    if !meta.file_type().is_file() {
        return Err(conflict(path));
    }
    Ok(file_digest_checked(path, cancel)? == proof.digest)
}

pub(super) fn owned(target: &Path, proof: &Proof) -> bool {
    if let Some(id) = &proof.published_identity {
        return crate::durable_fs::identity(target).is_ok_and(|current| current == *id);
    }
    if proof.link.is_some() {
        crate::placement::same_link(target, &proof.stage).unwrap_or(false)
    } else {
        if let Some(result) = crate::nas_ops::same_file(target, &proof.stage) {
            return result.unwrap_or(false);
        }
        same(target, &proof.stage)
    }
}
pub(super) fn same(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}
pub(super) fn conflict(path: &Path) -> io::Error {
    io::Error::other(format!("恢复需要核对文件,已保留内容: {}", path.display()))
}

impl Proof {
    pub(super) fn remote_for(&self, path: &Path) -> bool {
        self.published_identity.is_none() && crate::nas_ops::mapped_pair(&self.stage, path)
    }
    pub(super) fn occupied(&self, path: &Path) -> io::Result<bool> {
        if self.remote_for(path) {
            return crate::nas_ops::exists(path).ok_or_else(|| conflict(path))?;
        }
        Ok(organize::occupied(path))
    }
    pub(super) fn remove(&self, path: &Path, anchor: &Path) -> io::Result<()> {
        if self.remote_for(path) {
            crate::nas_ops::remove_owned(path, anchor).ok_or_else(|| conflict(path))?
        } else {
            fs::remove_file(path)
        }
    }
    pub(super) fn same_anchor(&self, path: &Path) -> io::Result<bool> {
        if self.remote_for(path) {
            crate::nas_ops::same_file(path, &self.stage).ok_or_else(|| conflict(path))?
        } else {
            Ok(same(path, &self.stage))
        }
    }
    pub(super) fn restore_move(&self, target: &Path, source: &Path) -> io::Result<()> {
        crate::nas_ops::rename(target, source, &self.stage, &|| false)
            .ok_or_else(|| conflict(target))?
    }
    /// Called only after undo progress is durable. Cleanup failure keeps evidence.
    pub(super) fn cleanup(&self, root: &Path, op: &Op) {
        if self.remote_for(op.target()) {
            let anchor = op.source().unwrap_or(&self.stage);
            if self.remove(&self.stage, anchor).is_err() {
                return;
            }
            let mut parent = self.stage.parent();
            while let Some(dir) = parent {
                if dir == root || !dir.starts_with(root) || fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        } else {
            let _ = organize::rollback(
                &[Op::Download {
                    path: self.stage.clone(),
                    url: String::new(),
                    crop: None,
                    stamp: None,
                }],
                Some(root),
            );
        }
    }
}
