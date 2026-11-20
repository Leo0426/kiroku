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
#[derive(Default)]
struct Configuration {
    backend: Option<Arc<Backend>>,
    error: Option<String>,
}
static CONFIGURATION: OnceLock<Mutex<Configuration>> = OnceLock::new();
fn configuration() -> std::sync::MutexGuard<'static, Configuration> {
    CONFIGURATION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
fn validate(c: &Config) -> io::Result<()> {
    if c.host.is_empty()
        || c.host.starts_with('-')
        || !c
            .host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b))
        || !c.local_root.is_absolute()
        || !c.remote_root.is_absolute()
        || c.local_root
            .components()
            .chain(c.remote_root.components())
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::other(
            "NAS SSH 配置无效，请检查主机与绝对路径映射",
        ));
    }
    Ok(())
}
pub fn configure(config: Option<Config>) -> io::Result<()> {
    let result = config.as_ref().map(validate).transpose();
    let mut state = configuration();
    state.backend = None;
    state.error = result.as_ref().err().map(ToString::to_string);
    result?;
    state.backend = config.map(|cfg| {
        Arc::new(Backend {
            cfg,
            verified: Mutex::new(None),
        })
    });
    Ok(())
}
pub fn configure_file(settings: &Path) -> io::Result<()> {
    let path = settings.with_file_name("nas-operations.json");
    let parsed = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| {
            io::Error::other("NAS 配置文件格式无效，请修正 nas-operations.json 后重启")
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(io::Error::other("无法读取 NAS 配置文件，请检查权限后重启")),
    };
    match parsed {
        Ok(config) => configure(config),
        Err(error) => {
            *configuration() = Configuration {
                backend: None,
                error: Some(error.to_string()),
            };
            Err(error)
        }
    }
}
/// Invalid opt-in configuration must never silently enable the local fallback.
pub fn ensure_writes_allowed() -> io::Result<()> {
    match &configuration().error {
        Some(error) => Err(io::Error::other(format!("{error}；媒体写入已阻止"))),
        None => Ok(()),
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub message: String,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
}
pub fn status() -> Status {
    let state = configuration();
    if let Some(error) = &state.error {
        return Status {
            state: "invalid".into(),
            message: error.clone(),
            local_root: None,
            remote_root: None,
        };
    }
    match &state.backend {
        None => Status {
            state: "disabled".into(),
            message: "未配置 NAS SSH，使用普通文件操作".into(),
            local_root: None,
            remote_root: None,
        },
        Some(b) => {
            // A previously checked mount can disappear or be replaced.
            let current = same_file::Handle::from_path(&b.cfg.local_root).ok();
            let verified = b.verified.lock().unwrap_or_else(|e| e.into_inner());
            let ready = current.is_some() && current.as_ref() == verified.as_ref();
            Status {
                state: if ready { "verified" } else { "unverified" }.into(),
                message: if ready {
                    "NAS 映射已核验；能力检测仅代表检测时的文件系统"
                } else {
                    "已配置 NAS SSH，等待核验当前挂载"
                }
                .into(),
                local_root: Some(b.cfg.local_root.clone()),
                remote_root: Some(b.cfg.remote_root.clone()),
            }
        }
    }
}
/// Explicit capability probe. Its only writes are in a newly owned temporary
/// directory under the configured mount; it never uses a media path.
pub fn diagnose(cancel: &dyn Fn() -> bool) -> io::Result<Status> {
    ensure_writes_allowed()?;
    let configured = { configuration().backend.clone() };
    let Some(b) = configured else {
        return Ok(status());
    };
    crate::append::check_stopped(cancel)?;
    let guard = crate::pipeline::RootGuard::capture(&b.cfg.local_root, false)?;
    let temp = tempfile::Builder::new()
        .prefix(".kiroku-nas-check-")
        .tempdir_in(&b.cfg.local_root)?;
    let probe = temp.path().join("probe");
    let token = format!("neutral:{}:{:?}", std::process::id(), Instant::now());
    fs::write(&probe, token.as_bytes())?;
    let result = b.call(serde_json::json!({"op":"capabilities","root":b.cfg.remote_root,"from":b.mapped(&probe)?,"token":token}), cancel);
    // Keep the root identity bound across remote execution and cleanup.
    guard.verify()?;
    let cleanup = temp.close();
    let value = result?;
    if value["ok"] != true {
        return Err(io::Error::other("NAS 路径映射或文件操作能力核验失败"));
    }
    cleanup.map_err(|_| io::Error::other("NAS 核验完成，但临时探针未能清理，请检查挂载"))?;
    *b.verified.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(same_file::Handle::from_path(&b.cfg.local_root)?);
    Ok(status())
}
fn backend(path: &Path) -> Option<Arc<Backend>> {
    configuration()
        .backend
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
                95 => io::ErrorKind::Unsupported,
                _ => io::ErrorKind::Other,
            };
            let stage = match value.get("stage").and_then(|v| v.as_str()) {
                Some("mapping") => "路径映射",
                Some("link") => "恢复硬链接",
                Some("rename") => "不覆盖移动",
                Some("digest") => "摘要",
                Some("sync") => "目录同步",
                _ => "文件操作",
            };
            return Err(io::Error::new(
                kind,
                format!("NAS {stage}核验失败（服务器错误 {n}）"),
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
 stage='mapping'
 src=safe(q['from'])
 if q['op']=='capabilities':
  with open(src,'rb') as f:
   if f.read(1024)!=q['token'].encode():raise OSError(13,'mapping mismatch')
  parent=os.path.dirname(src);anchor=os.path.join(parent,'anchor');target=os.path.join(parent,'target');occupied=os.path.join(parent,'occupied')
  owned={}
  def own(p):
   st=os.lstat(p);owned[p]=(st.st_dev,st.st_ino)
  stage='rename'
  libc=ctypes.CDLL(None,use_errno=True)
  if not hasattr(libc,'renameat2'):raise OSError(95,'exclusive rename unsupported')
  rename=libc.renameat2
  rename.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint];rename.restype=ctypes.c_int
  def move(a,b):
   if rename(-100,os.fsencode(a),-100,os.fsencode(b),1)!=0:raise OSError(ctypes.get_errno(),'exclusive rename failed')
  try:
   stage='link'
   os.link(src,anchor,follow_symlinks=False);own(anchor)
   with open(occupied,'xb') as f:f.write(b'neutral occupied');f.flush();os.fsync(f.fileno())
   own(occupied)
   stage='rename'
   try:move(src,occupied)
   except OSError as e:
    if e.errno!=17:raise
   else:raise OSError(5,'collision was overwritten')
   move(src,target);own(target)
   if not os.path.samefile(target,anchor):raise OSError(5,'anchor mismatch')
   stage='digest'
   with open(target,'rb') as f:
    if hashlib.sha256(f.read()).hexdigest()!=hashlib.sha256(q['token'].encode()).hexdigest():raise OSError(5,'digest mismatch')
   move(target,src)
   stage='sync'
   fd=os.open(parent,os.O_RDONLY)
   try:os.fsync(fd)
   finally:os.close(fd)
   result={'ok':True}
  finally:
   for p,expected in owned.items():
    try:
     st=os.lstat(p)
     if (st.st_dev,st.st_ino)==expected:os.unlink(p)
    except FileNotFoundError:pass
 elif q['op']=='probe':
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
except OSError as e:print(json.dumps({'errno':e.errno or 5,'stage':stage}))
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
