//! Durability operations for local disks and macOS SMB mounts.
use std::{fs,io,path::Path};

pub(crate) fn sync_directory(path:&Path)->io::Result<()> {
    let file=fs::File::open(path)?;
    match file.sync_all() {
        Ok(())=>Ok(()),
        #[cfg(target_os="macos")]
        Err(e) if e.raw_os_error()==Some(libc::ENOTSUP)=>{
            // SMB supports fsync, but not the F_FULLFSYNC used by Rust on macOS.
            // Do not suppress permission, disconnect, or I/O errors.
            use std::os::fd::AsRawFd;
            if unsafe {libc::fsync(file.as_raw_fd())}==0 {Ok(())}else{Err(io::Error::last_os_error())}
        },
        Err(e)=>Err(e),
    }
}
pub(crate) fn sync_file(path:&Path)->io::Result<()> {
    // SMB rejects flushes on read-only handles. Preserve readonly local-file support.
    match fs::OpenOptions::new().write(true).open(path) {
        Ok(file)=>file.sync_all(),
        Err(e) if e.kind()==io::ErrorKind::PermissionDenied=>fs::File::open(path)?.sync_all(),
        Err(e)=>Err(e),
    }
}
pub(crate) fn unsupported_link(e:&io::Error)->bool {
    if e.kind()==io::ErrorKind::Unsupported{return true}
    #[cfg(target_os="macos")]
    if e.raw_os_error()==Some(libc::ENOTSUP){return true}
    false
}
/// Birth time prevents an unlinked file's reused inode from authorizing a replacement.
pub(crate) fn identity(path:&Path)->io::Result<String> {
    identity_metadata(&fs::symlink_metadata(path)?)
}
pub(crate) fn identity_file(file:&fs::File)->io::Result<String>{identity_metadata(&file.metadata()?)}
fn identity_metadata(m:&fs::Metadata)->io::Result<String>{
    if !m.is_file(){return Err(io::Error::other("目标不是普通文件"))}
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        let birth=m.created()?.duration_since(std::time::UNIX_EPOCH).map_err(io::Error::other)?;
        Ok(format!("{}:{}:{}:{}",m.dev(),m.ino(),birth.as_secs(),birth.subsec_nanos()))
    }
    #[cfg(not(unix))] {Err(io::Error::new(io::ErrorKind::Unsupported,"没有可持久化的目标身份"))}
}
