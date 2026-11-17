//! Opt-in SSH filesystem acceleration. Paths travel as JSON, never shell input.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub host: String,
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
}
struct Backend {
    cfg: Config,
    verified: Mutex<Option<same_file::Handle>>,
}
static BACKEND: OnceLock<Mutex<Option<Arc<Backend>>>> = OnceLock::new();
pub fn configure(config: Option<Config>) -> io::Result<()> {
    if let Some(c) = &config {
        if c.host.is_empty()
            || c.host.starts_with('-')
            || !c
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b))
            || !c.local_root.is_absolute()
            || !c.remote_root.is_absolute()
        {
            return Err(io::Error::other("NAS SSH 配置无效"));
        }
    }
    *BACKEND.get_or_init(Default::default).lock().unwrap() = config.map(|cfg| {
        Arc::new(Backend {
            cfg,
            verified: Mutex::new(None),
        })
    });
    Ok(())
}
pub fn configure_file(settings: &Path) -> io::Result<()> {
    let p = settings.with_file_name("nas-operations.json");
    match fs::read(p) {
        Ok(b) => configure(Some(serde_json::from_slice(&b)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => configure(None),
        Err(e) => Err(e),
    }
}
fn backend(path: &Path) -> Option<Arc<Backend>> {
    BACKEND
        .get()?
        .lock()
        .ok()?
        .clone()
        .filter(|b| path.starts_with(&b.cfg.local_root))
}
/// macOS SMB exposes remotely-created names in decomposed form.
pub(crate) fn local_name(root: &Path, name: String) -> String {
    #[cfg(target_os = "macos")]
    if backend(root).is_some() {
        use unicode_normalization::UnicodeNormalization;
        return name.nfd().collect();
    }
    name
}
impl Backend {
    fn mapped(&self, path: &Path) -> io::Result<PathBuf> {
        let relative = path
            .strip_prefix(&self.cfg.local_root)
            .map_err(io::Error::other)?;
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(io::Error::other("NAS 路径不允许跳转"));
        }
        Ok(self.cfg.remote_root.join(relative))
    }
    fn call(
        &self,
        request: serde_json::Value,
        cancel: &dyn Fn() -> bool,
    ) -> io::Result<serde_json::Value> {
        let command = format!("python3 -c '{}'", SCRIPT.replace('\'', "'\\''"));
        let mut child = Command::new("/usr/bin/ssh")
            .args([
                "-T",
                "-oBatchMode=yes",
                "-oConnectTimeout=8",
                "-oServerAliveInterval=10",
                "-oServerAliveCountMax=2",
                "--",
                &self.cfg.host,
                &command,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let input = serde_json::to_vec(&request)?;
        let sent = child.stdin.take().unwrap().write_all(&input);
        if let Err(e) = sent {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let started = Instant::now();
        loop {
            if cancel() || started.elapsed() > Duration::from_secs(600) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    if cancel() {
                        io::ErrorKind::Interrupted
                    } else {
                        io::ErrorKind::TimedOut
                    },
                    "NAS 操作已停止或超时",
                ));
            }
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        let out = child.wait_with_output()?;
        if cancel() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "NAS 操作已停止"));
        }
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "NAS SSH 操作失败：{}",
                String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(400)
                    .collect::<String>()
            )));
        }
        let value: serde_json::Value = serde_json::from_slice(&out.stdout)?;
        if let Some(n) = value.get("errno").and_then(|n| n.as_i64()) {
            let kind = match n {
                17 => io::ErrorKind::AlreadyExists,
                18 => io::ErrorKind::CrossesDevices,
                2 => io::ErrorKind::NotFound,
                13 => io::ErrorKind::PermissionDenied,
                _ => io::ErrorKind::Other,
            };
            return Err(io::Error::new(
                kind,
                format!("NAS 文件操作失败（服务器错误 {n}）"),
            ));
        }
        Ok(value)
    }
    fn verify(&self, cancel: &dyn Fn() -> bool) -> io::Result<()> {
        let current = same_file::Handle::from_path(&self.cfg.local_root)?;
        let mut verified = self.verified.lock().unwrap();
        if verified.as_ref() == Some(&current) {
            return Ok(());
        }
        let mut probe = tempfile::Builder::new()
            .prefix(".kiroku-ssh-probe-")
            .tempfile_in(&self.cfg.local_root)?;
        let token = format!("{}:{:?}", std::process::id(), std::time::SystemTime::now());
        probe.write_all(token.as_bytes())?;
        probe.as_file().sync_all()?;
        let result=self.call(serde_json::json!({"op":"probe","root":self.cfg.remote_root,"from":self.mapped(probe.path())?,"token":token}),cancel)?;
        if result["ok"] != true {
            return Err(io::Error::other("NAS 路径映射校验失败"));
        }
        *verified = Some(current);
        Ok(())
    }
}
pub(crate) fn link(from: &Path, to: &Path, cancel: &dyn Fn() -> bool) -> Option<io::Result<()>> {
    let b = backend(from)?;
    if !to.starts_with(&b.cfg.local_root) {
        return None;
    }
    Some((|| {
        b.verify(cancel)?;
        b.call(serde_json::json!({"op":"link","root":b.cfg.remote_root,"from":b.mapped(from)?,"to":b.mapped(to)?}),cancel)?;
        // This is a recovery anchor, never open it through SMB: macOS SMB cannot
        // reliably stat two remote hardlink names for the same file identifier.
        Ok(())
    })())
}
pub(crate) fn mapped_pair(from: &Path, to: &Path) -> bool {
    backend(from).is_some_and(|b| to.starts_with(&b.cfg.local_root))
}
fn operation(
    path: &Path,
    op: &str,
    other: Option<&Path>,
    anchor: Option<&Path>,
    cancel: &dyn Fn() -> bool,
) -> Option<io::Result<serde_json::Value>> {
    let b = backend(path)?;
    Some((|| {
        b.verify(cancel)?;
        let mut q = serde_json::json!({"op":op,"root":b.cfg.remote_root,"from":b.mapped(path)?});
        if let Some(other) = other {
            q["to"] = serde_json::json!(b.mapped(other)?);
        }
        if let Some(anchor) = anchor {
            q["anchor"] = serde_json::json!(b.mapped(anchor)?);
        }
        b.call(q, cancel)
    })())
}
pub fn exists(path: &Path) -> Option<io::Result<bool>> {
    operation(path, "stat", None, None, &|| false).map(|r| match r {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    })
}
pub fn same_file(from: &Path, to: &Path) -> Option<io::Result<bool>> {
    operation(from, "same", Some(to), None, &|| false).map(|r| match r {
        Ok(v) => Ok(v["same"] == true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    })
}
pub(crate) fn rename(
    from: &Path,
    to: &Path,
    anchor: &Path,
    cancel: &dyn Fn() -> bool,
) -> Option<io::Result<()>> {
    operation(from, "rename", Some(to), Some(anchor), cancel).map(|r| r.map(|_| ()))
}
pub(crate) fn remove_owned(path: &Path, anchor: &Path) -> Option<io::Result<()>> {
    operation(path, "unlink", None, Some(anchor), &|| false).map(|r| r.map(|_| ()))
}
pub(crate) fn digest_remote(path: &Path, cancel: &dyn Fn() -> bool) -> Option<io::Result<String>> {
    let _ = backend(path)?;
    let started = Instant::now();
    log::info!(target:"kiroku::organize","正在 NAS 内部校验文件（不经本机传输）");
    operation(path,"hash",None,None,cancel).map(|r|{
        let v=r?;
        let digest=v["digest"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit())).ok_or_else(||io::Error::other("NAS 校验结果无效"))?.to_owned();
        log::info!(target:"kiroku::organize","NAS 校验完成，用时 {} 秒",started.elapsed().as_secs());
        Ok(digest)
    })
}
fn refresh_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries {
                let _ = entry;
            }
        }
    }
}
fn retry_missing<T>(
    cancel: &dyn Fn() -> bool,
    mut action: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let started = Instant::now();
    let mut announced = false;
    loop {
        if cancel() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "NAS 操作已停止"));
        }
        match action() {
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && started.elapsed() < Duration::from_secs(75) =>
            {
                if !announced {
                    log::info!(target:"kiroku::organize","等待 NAS 挂载同步目录信息…");
                    announced = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}
pub(crate) fn metadata(path: &Path, cancel: &dyn Fn() -> bool) -> Option<io::Result<fs::Metadata>> {
    backend(path)?;
    Some(retry_missing(cancel, || {
        refresh_parent(path);
        fs::symlink_metadata(path)
    }))
}
pub(crate) fn digest(path: &Path, cancel: &dyn Fn() -> bool) -> Option<io::Result<String>> {
    backend(path)?;
    let size = match metadata(path, cancel)? {
        Ok(meta) => meta.len(),
        Err(e) => return Some(Err(e)),
    };
    if size < 8 * 1024 * 1024 {
        return None;
    }
    digest_remote(path, cancel)
}
const SCRIPT: &str = r#"
import os,sys,json,hashlib,stat,unicodedata,ctypes
q=json.load(sys.stdin)
root=os.path.realpath(q['root'])
def safe(p,missing=False):
 if not os.path.isabs(p) or os.path.commonpath([root,os.path.abspath(p)])!=root: raise OSError(13,'outside root')
 relative=os.path.relpath(p,root)
 if relative=='.' or '..' in relative.split(os.sep): raise OSError(13,'invalid path')
 cur=root
 parts=relative.split(os.sep)
 for i,part in enumerate(parts):
  candidate=os.path.join(cur,part)
  if not os.path.lexists(candidate):
   # SMB/macOS may expose decomposed names for composed NAS directory entries.
   normalized=unicodedata.normalize('NFC',part)
   matches=[name for name in os.listdir(cur) if unicodedata.normalize('NFC',name)==normalized]
   if len(matches)>1:raise OSError(17,'ambiguous normalized name')
   if matches:candidate=os.path.join(cur,matches[0])
   elif missing and i==len(parts)-1:candidate=os.path.join(cur,normalized)
  try:
   if stat.S_ISLNK(os.lstat(candidate).st_mode): raise OSError(13,'symlink')
  except FileNotFoundError:
   if not missing or i!=len(parts)-1: raise
  cur=candidate
 return cur
def identity(s):return (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns)
try:
 src=safe(q['from'])
 if q['op']=='probe':
  with open(src,'rb') as f: ok=f.read(1024)==q['token'].encode()
  result={'ok':ok}
 elif q['op']=='stat':
  result={'size':os.stat(src).st_size}
 elif q['op']=='same':
  result={'same':os.path.samefile(src,safe(q['to']))}
 elif q['op']=='rename':
  dst=safe(q['to'],True);anchor=safe(q['anchor'])
  if not stat.S_ISREG(os.lstat(src).st_mode) or not os.path.samefile(src,anchor):raise OSError(16,'ownership changed')
  libc=ctypes.CDLL(None,use_errno=True)
  rename=libc.renameat2
  rename.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint]
  rename.restype=ctypes.c_int
  if rename(-100,os.fsencode(src),-100,os.fsencode(dst),1)!=0:raise OSError(ctypes.get_errno(),'exclusive rename failed')
  for parent in set([os.path.dirname(src),os.path.dirname(dst)]):
   fd=os.open(parent,os.O_RDONLY)
   try:os.fsync(fd)
   finally:os.close(fd)
  result={'ok':True}
 elif q['op']=='unlink':
  anchor=safe(q['anchor'])
  if not os.path.samefile(src,anchor):raise OSError(16,'ownership changed')
  os.unlink(src)
  fd=os.open(os.path.dirname(src),os.O_RDONLY)
  try:os.fsync(fd)
  finally:os.close(fd)
  result={'ok':True}
 elif q['op']=='link':
  dst=safe(q['to'],True)
  if not stat.S_ISREG(os.lstat(src).st_mode):raise OSError(13,'not regular')
  os.link(src,dst,follow_symlinks=False)
  fd=os.open(src,os.O_RDONLY)
  try:os.fsync(fd)
  finally:os.close(fd)
  fd=os.open(os.path.dirname(dst),os.O_RDONLY)
  try:os.fsync(fd)
  finally:os.close(fd)
  result={'ok':True}
 elif q['op']=='hash':
  if not stat.S_ISREG(os.lstat(src).st_mode):raise OSError(13,'not regular')
  with open(src,'rb') as f:
   before=identity(os.fstat(f.fileno()));h=hashlib.sha256()
   while True:
    data=f.read(1024*1024)
    if not data:break
    h.update(data)
   if before!=identity(os.fstat(f.fileno())) or before!=identity(os.stat(src)):raise OSError(16,'changed during hash')
  result={'digest':h.hexdigest()}
 else:raise OSError(22,'unknown operation')
 print(json.dumps(result))
except OSError as e:print(json.dumps({'errno':e.errno or 5}))
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transient_missing_is_retried_but_permission_and_cancellation_stop() {
        let mut attempts = 0;
        let result = retry_missing(&|| false, || {
            attempts += 1;
            if attempts < 3 {
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else {
                Ok(42)
            }
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 3);
        let mut attempts = 0;
        let result = retry_missing::<()>(&|| false, || {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts, 1);
        let result = retry_missing::<()>(&|| true, || panic!("cancelled operation must not run"));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
}
