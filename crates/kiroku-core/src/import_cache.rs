//! Disposable, crash-safe import cache; never an authority for media file operations.
use std::{fs,io::{self,Read,Write},path::{Path,PathBuf}};
use sha2::{Digest,Sha256};
use crate::{code::Code,metadata::SourceSnapshot,settings::Settings};
const MAGIC:&[u8]=b"KIROKU-CACHE-1\n";
#[derive(Clone)]
pub struct ImportCache {root:PathBuf}
pub fn cache_path(settings:&Path)->PathBuf {settings.parent().unwrap_or(Path::new(".")).join("cache/import-v1")}
pub fn image_scope(settings:&Settings)->String {
    hash(&serde_json::to_vec(&(&settings.sources,&settings.proxy)).expect("settings serialize"))
}
fn hash(bytes:&[u8])->String{format!("{:x}",Sha256::digest(bytes))}
impl ImportCache {
 pub fn new(root:PathBuf)->Self{Self{root}}
 fn path(&self,kind:&str,key:&str)->PathBuf{self.root.join(kind).join(hash(key.as_bytes()))}
 pub(crate) fn read(&self,kind:&str,key:&str,limit:usize)->Option<Vec<u8>>{
  let mut file=fs::File::open(self.path(kind,key)).ok()?;
  if file.metadata().ok()?.len()> (limit+MAGIC.len()+64) as u64{return None}
  let mut bytes=Vec::new();Read::by_ref(&mut file).take((limit+MAGIC.len()+65) as u64).read_to_end(&mut bytes).ok()?;
  let body=bytes.get(MAGIC.len()+64..)?;
  if !bytes.starts_with(MAGIC)||body.len()>limit||bytes.get(MAGIC.len()..MAGIC.len()+64)?!=hash(body).as_bytes(){return None}
  Some(body.to_vec())
 }
 pub(crate) fn write(&self,kind:&str,key:&str,bytes:&[u8])->io::Result<()> {
  let path=self.path(kind,key);let parent=path.parent().unwrap();fs::create_dir_all(parent)?;
  let mut file=tempfile::NamedTempFile::new_in(parent)?;
  file.write_all(MAGIC)?;file.write_all(hash(bytes).as_bytes())?;file.write_all(bytes)?;file.as_file().sync_all()?;
  file.persist(&path).map_err(|e|e.error)?;
  #[cfg(unix)] crate::durable_fs::sync_directory(parent)?;
  Ok(())
 }
 fn source_key(code:&Code,site:&str)->String{serde_json::to_string(&(&code.id,code.content_type,site)).unwrap()}
 pub(crate) fn source(&self,code:&Code,site:&str)->Option<SourceSnapshot>{serde_json::from_slice(&self.read("sources",&Self::source_key(code,site),4*1024*1024)?).ok()}
 pub(crate) fn save_source(&self,code:&Code,site:&str,snapshot:&SourceSnapshot)->io::Result<()> {
  let bytes=serde_json::to_vec(snapshot)?;
  if bytes.len()>4*1024*1024{return Err(io::Error::other("来源缓存超过大小限制"))}
  self.write("sources",&Self::source_key(code,site),&bytes)
 }
 pub fn clear(&self)->io::Result<()> {
  match fs::remove_dir_all(&self.root){Ok(())=>Ok(()),Err(e) if e.kind()==io::ErrorKind::NotFound=>Ok(()),Err(e)=>Err(e)}
 }
}
