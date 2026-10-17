//! Incremental durable checkpoints. A single journal file contains a framed
//! snapshot followed by checked deltas; public save compacts it to legacy JSON.
use super::{Journal, JournalItem, JournalStatus, Pending, Proof, Replacement};
use crate::organize::Op;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read, Seek, Write}, path::{Path, PathBuf}, sync::{Arc,Mutex,Weak,OnceLock}, hash::{Hash,Hasher}};

const MAGIC: &[u8] = b"KIROKU-JOURNAL\x01\n";
const BUFFER: usize = 64*1024;

#[derive(Debug, Default)]
pub(super) struct Storage(Mutex<Option<Cursor>>);
impl Clone for Storage {fn clone(&self)->Self {Self(Mutex::new(self.0.lock().unwrap_or_else(|e|e.into_inner()).clone()))}}
// Storage cursors are not domain state or serialized data.
impl PartialEq for Storage {fn eq(&self,_:&Self)->bool {true}}
#[derive(Debug,Clone)]
struct Cursor {root:Arc<same_file::Handle>,file:Binding,bytes:u64,physical:u64,seq:u64,counts:Counts,last_proof:Option<usize>,framed:bool}
#[derive(Debug,Clone)]
enum Binding {Active(Arc<same_file::Handle>),Cold(Token,[u8;32])}
#[derive(Debug,Clone,PartialEq)]
pub(super) struct Token {identity:[u8;32],modified:std::time::SystemTime,created:Option<std::time::SystemTime>,#[cfg(unix)]ctime:(i64,i64)}
struct IdentityHash(Sha256);
impl Hasher for IdentityHash {fn finish(&self)->u64{unreachable!("identity bytes are consumed as SHA") }fn write(&mut self,b:&[u8]){self.0.update(b)}}
impl Token {pub(super) fn of(h:&same_file::Handle)->io::Result<Self>{
    let mut hash=IdentityHash(Sha256::new());h.hash(&mut hash);let m=h.as_file().metadata()?;
    Ok(Self{identity:hash.0.finalize().into(),modified:m.modified()?,created:m.created().ok(),
        #[cfg(unix)]ctime:{use std::os::unix::fs::MetadataExt;(m.ctime(),m.ctime_nsec())}})
}}
// Historical models retain one shared root anchor, not an open file per log.
fn shared_root(h:same_file::Handle)->Arc<same_file::Handle>{
    static ROOTS:OnceLock<Mutex<Vec<Weak<same_file::Handle>>>>=OnceLock::new();
    let mut roots=ROOTS.get_or_init(||Mutex::new(vec![])).lock().unwrap_or_else(|e|e.into_inner());
    roots.retain(|r|r.strong_count()>0);
    if let Some(root)=roots.iter().filter_map(Weak::upgrade).find(|r|**r==h){return root}
    if roots.len()>=16{roots.remove(0);}
    let root=Arc::new(h);roots.push(Arc::downgrade(&root));root
}
#[derive(Debug,Clone,Copy,PartialEq,Serialize,Deserialize)]
struct Counts {ops:usize,proofs:usize,items:usize,trashed:usize,rolled:usize,replacements:usize}
impl Counts {fn of(j:&Journal)->Self{Self{ops:j.ops.len(),proofs:j.proofs.len(),items:j.items.len(),trashed:j.trashed.len(),rolled:j.rolled_back_ops.len(),replacements:j.replacements.len()}}}
#[derive(Serialize)]
struct Snapshot<'a> {kind:&'static str,data:&'a Journal}
#[derive(Serialize)]
struct DeltaRecord<'a>{kind:&'static str,data:DeltaRef<'a>}
#[derive(Serialize)]
struct DeltaRef<'a>{seq:u64,before:Counts,version:u32,status:JournalStatus,pending:&'a Option<Pending>,ops:&'a [Op],proofs:Vec<(usize,&'a Proof)>,items:&'a [JournalItem],trashed:&'a [PathBuf],rolled:&'a [usize],replacements:&'a [Replacement],undone:Option<usize>}
#[derive(Deserialize)]
#[serde(tag="kind",content="data")]
enum Record {Snapshot(Box<Journal>),Delta(Delta)}
#[derive(Deserialize)]
struct Delta{seq:u64,before:Counts,version:u32,status:JournalStatus,pending:Option<Pending>,ops:Vec<Op>,proofs:Vec<(usize,Proof)>,items:Vec<JournalItem>,trashed:Vec<PathBuf>,rolled:Vec<usize>,replacements:Vec<Replacement>,undone:Option<usize>}
fn invalid(message:&str)->io::Error {io::Error::new(io::ErrorKind::InvalidData,message)}
fn changed()->io::Error {io::Error::other("日志或媒体库身份/长度已变化，请重新读取历史")}

struct HashWriter {hash:Sha256,bytes:u64}
impl Write for HashWriter {fn write(&mut self,b:&[u8])->io::Result<usize>{self.bytes=self.bytes.checked_add(b.len() as u64).ok_or_else(||invalid("日志过大"))?;self.hash.update(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
// serde and io::copy retry Interrupted. Abort inside the reader with Other,
// then translate to Interrupted at the public cancellation boundary.
struct CheckedReader<'a,R>{inner:R,cancel:&'a dyn Fn()->bool}
impl<R:Read> Read for CheckedReader<'_,R>{fn read(&mut self,b:&mut[u8])->io::Result<usize>{if (self.cancel)(){return Err(io::Error::other("已取消日志读取"))}let n=self.inner.read(b)?;#[cfg(test)]metrics::record_read(n as u64);Ok(n)}}
impl<R:Seek> Seek for CheckedReader<'_,R>{fn seek(&mut self,to:io::SeekFrom)->io::Result<u64>{crate::append::check_stopped(self.cancel)?;self.inner.seek(to)}}
struct HashReader<R>{inner:R,hash:Sha256}
impl<R:Read+Seek> Seek for HashReader<R>{fn seek(&mut self,to:io::SeekFrom)->io::Result<u64>{
    let pos=self.inner.seek(to)?;self.hash=Sha256::new();
    if pos==MAGIC.len() as u64{self.hash.update(MAGIC);}else if pos!=0{return Err(invalid("日志读取位置无效"))}Ok(pos)
}}
impl<R:Read> Read for HashReader<R>{fn read(&mut self,b:&mut[u8])->io::Result<usize>{let n=self.inner.read(b)?;self.hash.update(&b[..n]);Ok(n)}}
fn write_frame(writer:&mut impl Write,value:&impl Serialize)->io::Result<u64>{
    let mut counter=HashWriter{hash:Sha256::new(),bytes:0};
    serde_json::to_writer(&mut counter,value).map_err(io::Error::other)?;
    let size=counter.bytes.to_le_bytes();let digest=counter.hash.finalize();
    let mut header=Sha256::new();header.update(size);header.update(digest);
    writer.write_all(&size)?;writer.write_all(&digest)?;writer.write_all(&header.finalize())?;
    serde_json::to_writer(writer,value).map_err(io::Error::other)?;
    let bytes=72+counter.bytes;
    #[cfg(test)] metrics::record(bytes);
    Ok(bytes)
}
fn sync_parent(p:&Path)->io::Result<()> {#[cfg(unix)] fs::File::open(p.parent().ok_or_else(changed)?)?.sync_all()?;Ok(())}
fn handle(path:&Path)->io::Result<same_file::Handle>{crate::append::regular(path)?;same_file::Handle::from_path(path)}
/// Open without truncation first: stale ordinary atomic-write files are reusable,
/// while a link, substituted path, or shared inode must never clobber old history.
pub(super) fn open_temporary(j:&Journal,root:&same_file::Handle)->io::Result<(fs::File,same_file::Handle)> {
    let tmp=j.path().with_extension("json.tmp");
    let file=match fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
        Ok(file)=>file,
        Err(error) if error.kind()==io::ErrorKind::AlreadyExists=>fs::OpenOptions::new().write(true).open(&tmp)?,
        Err(error)=>return Err(error),
    };
    let owner=same_file::Handle::from_file(file.try_clone()?)?;
    if handle(&tmp)?!=owner || same_file::Handle::from_path(&j.root)?!=*root {return Err(changed())}
    if handle(&j.path()).is_ok_and(|current|current==owner){return Err(changed())}
    #[cfg(unix)] {use std::os::unix::fs::MetadataExt;if file.metadata()?.nlink()!=1{return Err(changed())}}
    file.set_len(0)?;Ok((file,owner))
}
pub(super) fn verify_temporary(j:&Journal,root:&same_file::Handle,owner:&same_file::Handle)->io::Result<()> {
    if same_file::Handle::from_path(&j.root)?!=*root || handle(&j.path().with_extension("json.tmp"))?!=*owner{return Err(changed())}Ok(())
}
fn create_directory(root:&Path)->io::Result<()> {
    let parent=super::dir(root);
    if root.is_dir(){match fs::create_dir(&parent){Ok(())=>{},Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e)}}
    else{fs::create_dir_all(&parent)?;}
    crate::append::confined_path(root,&parent)?;Ok(())
}
impl Cursor {
    fn capture(j:&Journal,bytes:u64,seq:u64)->io::Result<Self>{Ok(Self{root:shared_root(same_file::Handle::from_path(&j.root)?),file:Binding::Active(Arc::new(handle(&j.path())?)),bytes,physical:bytes,seq,counts:Counts::of(j),last_proof:j.proofs.last_key_value().map(|(i,_)|*i),framed:true})}
    fn verify(&self,j:&Journal)->io::Result<()> {
        if same_file::Handle::from_path(&j.root)?!=*self.root {return Err(changed())}
        let current=handle(&j.path())?;
        match &self.file {
            Binding::Active(file) if current!=**file=>return Err(changed()),
            Binding::Cold(token,_) if Token::of(&current)?!=*token=>return Err(changed()),
            _=>{},
        }
        crate::append::confined_path(&j.root,&j.path())?;Ok(())
    }
    fn activate(&mut self,j:&Journal)->io::Result<()> {self.activate_checked(j,&||false)}
    fn activate_checked(&mut self,j:&Journal,cancel:&dyn Fn()->bool)->io::Result<()> {
        crate::append::check_stopped(cancel)?;self.verify(j)?;
        if let Binding::Cold(token,digest)=&self.file {
            let file=fs::File::open(j.path())?;let h=same_file::Handle::from_file(file.try_clone()?)?;
            if file.metadata()?.len()!=self.physical || Token::of(&h)?!=*token{return Err(changed())}
            let mut input=HashReader{inner:CheckedReader{inner:file,cancel},hash:Sha256::new()};io::copy(&mut input,&mut io::sink()).map_err(|e|if cancel(){io::Error::new(io::ErrorKind::Interrupted,"已取消日志读取")}else{e})?;
            if <[u8;32]>::from(input.hash.finalize())!=*digest || Token::of(&h)?!=*token{return Err(changed())}
            self.verify(j)?;self.file=Binding::Active(Arc::new(h));
        }
        Ok(())
    }
    fn active_file(&self)->&same_file::Handle{match &self.file{Binding::Active(file)=>file,Binding::Cold(..)=>unreachable!("activated writer")}}

}

impl Journal {
    /// Only internal monotonic changes use this path; arbitrary caller edits use save().
    pub(super) fn persist_progress(&self)->io::Result<PathBuf>{self.persist_progress_undo(None)}
    pub(super) fn persist_progress_undo(&self,undone:Option<usize>)->io::Result<PathBuf>{
        #[cfg(test)] metrics::record_persist();
        #[cfg(test)] if metrics::use_snapshots(){return self.save()}
        let mut state=self.storage.0.lock().unwrap_or_else(|e|e.into_inner());
        let path=self.path();
        if state.as_ref().is_none_or(|c|!c.framed) {
            if state.is_none() && crate::organize::occupied(&path){return Err(io::Error::new(io::ErrorKind::AlreadyExists,"日志编号已被占用"))}
            if let Some(cursor)=state.as_mut(){cursor.activate(self)?;if fs::metadata(&path)?.len()!=cursor.physical{return Err(changed())}}
            // Initial snapshot includes the current change; no media operation happens
            // until the rename and directory sync below have completed.
            if state.is_none(){create_directory(&self.root)?;}
            crate::append::confined_path(&self.root,&path)?;
            let root=same_file::Handle::from_path(&self.root)?;
            let tmp=path.with_extension("json.tmp");
            let (file,owner)=open_temporary(self,&root)?;
            let result=(||{
                let mut writer=io::BufWriter::with_capacity(BUFFER,file);
                writer.write_all(MAGIC)?;
                #[cfg(test)] metrics::record(MAGIC.len() as u64);
                let bytes=MAGIC.len() as u64+write_frame(&mut writer,&Snapshot{kind:"Snapshot",data:self})?;
                writer.flush()?;writer.get_ref().sync_all()?;
                verify_temporary(self,&root,&owner)?;
                if let Some(cursor)=state.as_ref(){cursor.verify(self)?;if fs::metadata(&path)?.len()!=cursor.physical{return Err(changed())}}
                else if crate::organize::occupied(&path){return Err(changed())}
                fs::rename(&tmp,&path)?;sync_parent(&path)?;
                *state=Some(Cursor::capture(self,bytes,0)?);Ok(path.clone())
            })();
            if result.is_err() && handle(&tmp).is_ok_and(|h|h==owner){let _=fs::remove_file(&tmp);}
            return result;
        }
        let cursor=state.as_mut().unwrap();cursor.activate(self)?;
        let counts=Counts::of(self);let before=cursor.counts;
        if counts.ops<before.ops || counts.proofs<before.proofs || counts.items<before.items || counts.trashed<before.trashed || counts.rolled<before.rolled || counts.replacements<before.replacements {return Err(invalid("增量日志不能缩短既有历史"))}
        let proofs=self.proofs.range((cursor.last_proof.map_or(std::ops::Bound::Unbounded,std::ops::Bound::Excluded),std::ops::Bound::Unbounded)).map(|(i,p)|(*i,p)).collect::<Vec<_>>();
        if proofs.len()!=counts.proofs-before.proofs{return Err(invalid("增量日志不能修改既有证明"))}
        let delta=DeltaRecord{kind:"Delta",data:DeltaRef{seq:cursor.seq+1,before,version:self.version,status:self.status,pending:&self.pending,ops:&self.ops[before.ops..],proofs,items:&self.items[before.items..],trashed:&self.trashed[before.trashed..],rolled:&self.rolled_back_ops[before.rolled..],replacements:&self.replacements[before.replacements..],undone}};
        let file=fs::OpenOptions::new().read(true).write(true).open(&path)?;
        if same_file::Handle::from_file(file.try_clone()?)?!=*cursor.active_file() || file.metadata()?.len()!=cursor.physical {return Err(changed())}
        let mut writer=io::BufWriter::with_capacity(BUFFER,file);if cursor.physical!=cursor.bytes {writer.get_ref().set_len(cursor.bytes)?;}
        writer.seek(io::SeekFrom::Start(cursor.bytes))?;
        let added=write_frame(&mut writer,&delta)?;
        writer.flush()?;writer.get_ref().sync_all()?;cursor.verify(self)?;
        cursor.bytes+=added;cursor.physical=cursor.bytes;cursor.seq+=1;cursor.counts=counts;cursor.last_proof=self.proofs.last_key_value().map(|(i,_)|*i);
        Ok(path)
    }

    /// 本日志是否已经写过(之后的写入都核对库与文件身份,不会再建目录)。
    pub(super) fn storage_initialized(&self)->bool{self.storage.0.lock().unwrap_or_else(|e|e.into_inner()).is_some()}

    pub(super) fn prepare_storage_directory(&self)->io::Result<()> {
        let initialized=self.storage.0.lock().unwrap_or_else(|e|e.into_inner()).is_some();
        if !initialized{create_directory(&self.root)?;}Ok(())
    }
    pub(super) fn check_storage(&self)->io::Result<()> {
        let mut state=self.storage.0.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(cursor)=state.as_mut(){cursor.activate(self)?;if fs::metadata(self.path())?.len()!=cursor.physical{return Err(changed())}}
        Ok(())
    }
    pub(super) fn verify_read_identity(&self)->io::Result<()> {
        let state=self.storage.0.lock().unwrap_or_else(|e|e.into_inner());
        let cursor=state.as_ref().ok_or_else(changed)?;cursor.verify(self)?;
        if fs::metadata(self.path())?.len()!=cursor.physical{return Err(changed())}Ok(())
    }
    pub(super) fn verify_read_checked(&self,cancel:&dyn Fn()->bool)->io::Result<()> {
        let mut state=self.storage.0.lock().unwrap_or_else(|e|e.into_inner());
        let cursor=state.as_mut().ok_or_else(changed)?;cursor.activate_checked(self,cancel)?;
        if fs::metadata(self.path())?.len()!=cursor.physical{return Err(changed())}Ok(())
    }
    pub(super) fn mark_snapshot(&self)->io::Result<()> {
        let mut cursor=Cursor::capture(self,fs::metadata(self.path())?.len(),0)?;cursor.framed=false;
        *self.storage.0.lock().unwrap_or_else(|e|e.into_inner())=Some(cursor);Ok(())
    }
}

impl Delta {
    fn apply(self,j:&mut Journal,seq:u64)->io::Result<()> {
        if self.seq!=seq+1 || self.before!=Counts::of(j){return Err(invalid("日志增量序号或历史长度不匹配"))}
        for (index,_) in &self.proofs {if *index<j.ops.len() || *index>=j.ops.len()+self.ops.len() || j.proofs.contains_key(index){return Err(invalid("日志证明索引无效"))}}
        for index in &self.rolled {if *index>=j.ops.len()+self.ops.len() || j.rolled_back_ops.contains(index){return Err(invalid("日志撤销索引无效"))}}
        if let Some(index)=self.undone {let r=j.replacements.get_mut(index).ok_or_else(||invalid("日志修订索引无效"))?;r.undone=true;}
        j.version=self.version;j.status=self.status;j.pending=self.pending;
        j.ops.extend(self.ops);j.proofs.extend(self.proofs);j.items.extend(self.items);j.trashed.extend(self.trashed);j.rolled_back_ops.extend(self.rolled);j.replacements.extend(self.replacements);
        Ok(())
    }
}

/// Replay a valid prefix without collecting file bytes or all records. A torn tail
/// is never applied; a complete but corrupt frame fails closed.
pub(super) fn read_checked(path:&Path,cancel:&dyn Fn()->bool)->io::Result<Journal>{
    read_inner(path,cancel).map_err(|e|if cancel(){io::Error::new(io::ErrorKind::Interrupted,"已取消日志读取")}else{e})
}
fn read_inner(path:&Path,cancel:&dyn Fn()->bool)->io::Result<Journal>{
    crate::append::check_stopped(cancel)?;
    let owner=path.parent().and_then(Path::parent).ok_or_else(changed)?;
    let guard=crate::pipeline::RootGuard::capture(owner,false)?;
    crate::append::confined_path(owner,path)?;crate::append::regular(path)?;
    let file=fs::File::open(path)?;let identity=Arc::new(same_file::Handle::from_file(file.try_clone()?)?);
    let total=file.metadata()?.len();let mut reader=io::BufReader::with_capacity(BUFFER,HashReader{inner:CheckedReader{inner:file,cancel},hash:Sha256::new()});
    let mut magic=vec![0;MAGIC.len()];let n=reader.read(&mut magic)?;
    let journal=if n==MAGIC.len() && magic==MAGIC {

        let mut pos=MAGIC.len() as u64;let mut seq=0;let mut j=None;let mut torn=false;
        while pos<total {
            if total-pos<72 {torn=true;break}
            let mut len=[0;8];let mut digest=[0;32];let mut checksum=[0;32];reader.read_exact(&mut len)?;reader.read_exact(&mut digest)?;reader.read_exact(&mut checksum)?;
            let mut header=Sha256::new();header.update(len);header.update(digest);
            if header.finalize().as_slice()!=checksum{return Err(invalid("日志记录头校验失败"))}
            let size=u64::from_le_bytes(len);
            if size>total-pos-72 {torn=true;break}
            let mut input=HashReader{inner:reader.by_ref().take(size),hash:Sha256::new()};
            let record:Record=serde_json::from_reader(&mut input).map_err(|e|invalid(&format!("日志记录无法读取: {e}")))?;
            if input.inner.limit()!=0 || input.hash.finalize().as_slice()!=digest{return Err(invalid("日志记录内容校验失败"))}
            match record {
                Record::Snapshot(value) if j.is_none()=>{j=Some(*value);},
                Record::Delta(delta)=>{let value=j.as_mut().ok_or_else(||invalid("日志缺少初始快照"))?;delta.apply(value,seq)?;seq+=1;},
                _=>return Err(invalid("日志快照重复")),
            }
            pos+=72+size;
        }
        let mut j=j.ok_or_else(||invalid("日志初始快照不完整"))?;
        if torn {j.status=JournalStatus::Partial;}
        let mut cursor=Cursor::capture(&j,pos,seq)?;
        // Keep the original file identity even if a replacement appears during replay.
        cursor.file=Binding::Active(identity.clone());cursor.physical=total;cursor.root=shared_root(same_file::Handle::from_path(owner)?);
        *j.storage.0.lock().unwrap_or_else(|e|e.into_inner())=Some(cursor);
        j
    }else {
        reader.seek_relative(-(n as i64))?;
        match serde_json::from_reader::<_,Journal>(&mut reader) {
            Ok(j)=>j,
            Err(_)=> {reader.seek(io::SeekFrom::Start(0))?;let ops:Vec<Op>=serde_json::from_reader(&mut reader).map_err(|e|invalid(&e.to_string()))?;
                let stem=path.file_stem().and_then(|s|s.to_str()).unwrap_or("legacy");let id=stem.strip_prefix("journal-").unwrap_or(stem).to_owned();
                let mut j=Journal::new(owner);j.ts=id.parse().unwrap_or(0);j.id=id;j.root=owner.to_path_buf();j.version=1;j.ops=ops;j
            }
        }
    };
    guard.verify()?;
    let needs_cursor=journal.storage.0.lock().unwrap_or_else(|e|e.into_inner()).is_none();
    if needs_cursor{journal.mark_snapshot()?;}
    if handle(path)?!=*identity {return Err(changed())}
    // Drain a torn tail too: the activation checksum binds the complete physical
    // file observed here, without another retained byte buffer or descriptor.
    io::copy(&mut reader,&mut io::sink())?;
    let digest:[u8;32]=reader.get_ref().hash.clone().finalize().into();
    let token=Token::of(&identity)?;
    guard.verify()?;if handle(path)?!=*identity{return Err(changed())}
    crate::append::check_stopped(cancel)?;
    let mut state=journal.storage.0.lock().unwrap_or_else(|e|e.into_inner());
    state.as_mut().unwrap().file=Binding::Cold(token,digest);drop(state);
    Ok(journal)
}

#[cfg(test)]
pub(super) mod metrics {
    thread_local!{static WRITTEN:std::cell::Cell<u64>=const{std::cell::Cell::new(0)};}
    thread_local!{static SNAPSHOTS:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    thread_local!{static READ:std::cell::Cell<u64>=const{std::cell::Cell::new(0)};}
    thread_local!{static PERSISTS:std::cell::Cell<u64>=const{std::cell::Cell::new(0)};}
    pub fn record_persist(){PERSISTS.set(PERSISTS.get()+1)}
    pub fn take_persists()->u64{PERSISTS.replace(0)}
    pub fn record_read(n:u64){READ.set(READ.get()+n)}
    pub fn take_reads()->u64{READ.replace(0)}
    pub fn use_snapshots()->bool{SNAPSHOTS.get()}
    pub fn snapshots(value:bool){SNAPSHOTS.set(value)}
    pub fn record(n:u64){WRITTEN.set(WRITTEN.get()+n)}
    pub fn take()->u64{WRITTEN.replace(0)}
}

#[cfg(test)]
mod growth_tests {
    use super::*;
    #[test]
    #[ignore="journal-only checkpoint timing and actual serialized write volume"]
    fn checkpoint_write_growth(){
        for legacy in [true,false] {
            let sizes=std::env::var("KIROKU_JOURNAL_SCALE").ok().map(|s|vec![s.parse::<usize>().expect("positive checkpoint count")])
                .unwrap_or_else(||if legacy{vec![100,1000]}else{vec![100,1000,5000]});
            for n in sizes {
                let root=tempfile::tempdir().unwrap();let mut j=Journal::new(root.path());
                metrics::snapshots(legacy);metrics::take();let start=std::time::Instant::now();
                j.status=JournalStatus::Partial;j.persist_progress().unwrap();
                for i in 0..n {
                    let op=Op::WriteFile{path:root.path().join(format!("{i}.nfo")),content:String::new()};
                    let proof=Proof{stage:root.path().join(format!(".kiroku-{}-{i}.tmp",j.id)),digest:"00".repeat(32),link:None,source_proof:None};
                    j.items.push(JournalItem{code:format!("ABC-{i:05}"),dir:root.path().to_path_buf()});
                    j.pending=Some(Pending{op:op.clone(),proof:proof.clone()});j.persist_progress().unwrap();
                    j.push_ops([op]);j.proofs.insert(i,proof);j.pending=None;j.persist_progress().unwrap();
                }
                j.status=JournalStatus::Applied;j.save().unwrap();let elapsed=start.elapsed().as_millis();let bytes=metrics::take();
                assert_eq!(Journal::load(&j.path()).unwrap(),j);
                println!("checkpoints legacy={legacy} n={n} ms={elapsed} written={bytes}B final={}B",fs::metadata(j.path()).unwrap().len());
                if !legacy {assert!(bytes<(n as u64)*2500+65536,"incremental writes must grow linearly");}
                metrics::snapshots(false);
            }
        }
    }
}
