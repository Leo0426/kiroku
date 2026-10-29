//! Display-only bounded summaries. Never used by recovery or ownership discovery.
use super::{Journal,JournalStatus,storage};
use crate::pipeline::RootGuard;
use serde::{Serialize,Deserialize};
use std::{collections::BTreeMap,fs,io,path::{Path,PathBuf}};
use tokio_util::sync::CancellationToken;
const MAX_ENTRIES:usize=4096;
const MAX_BYTES:usize=4*1024*1024;
#[derive(Debug,Clone,PartialEq,Serialize)]
#[serde(rename_all="camelCase")]
pub struct HistorySummary {pub id:String,pub ts:u64,pub status:JournalStatus,pub item_count:usize,pub op_count:usize,pub codes:Vec<String>,pub trashed:usize,pub pending_operations:usize,pub error:Option<String>}
impl HistorySummary {
    fn of(j:Journal)->Self {Self{id:j.id,ts:j.ts,status:j.status,item_count:j.items.len(),op_count:j.ops.len()+j.replacements.len(),codes:j.items.into_iter().take(6).map(|i|i.code).collect(),trashed:j.trashed.len(),pending_operations:usize::from(j.pending.is_some())+if j.status==JournalStatus::Partial{j.replacements.iter().filter(|r|!r.undone).count()}else{0},error:None}}
    fn failed(id:String,error:io::Error)->Self{Self{ts:id.parse().unwrap_or(0),id,status:JournalStatus::Partial,item_count:0,op_count:0,codes:vec![],trashed:0,pending_operations:0,error:Some(error.to_string())}}
    fn bytes(&self)->usize{std::mem::size_of::<Self>()+self.id.capacity()+self.codes.capacity()*std::mem::size_of::<String>()+self.codes.iter().map(String::capacity).sum::<usize>()+self.error.as_ref().map_or(0,String::capacity)}
}
/// 一页历史摘要。`next` 是下一页的游标（本页最后一条的 id），没有更多时为 None。
#[derive(Debug,Clone,PartialEq,Serialize)]
#[serde(rename_all="camelCase")]
pub struct HistoryPage {pub items:Vec<HistorySummary>,pub total:usize,pub next:Option<String>}
#[derive(Debug,Clone,Default,Deserialize)]
#[serde(rename_all="camelCase", default)]
pub struct HistoryFilter { pub text:String, pub status:Option<JournalStatus>, pub from:Option<u64>, pub until:Option<u64> }
impl HistoryFilter {
    fn active(&self)->bool { !self.text.trim().is_empty() || self.status.is_some() || self.from.is_some() || self.until.is_some() }
    fn matches(&self, summary:&HistorySummary, codes:&str)->bool {
        self.status.is_none_or(|v|v==summary.status) && self.from.is_none_or(|v|summary.ts>=v) && self.until.is_none_or(|v|summary.ts<v)
          && self.text.split_whitespace().all(|word|codes.contains(&word.to_lowercase()) || summary.id.to_lowercase().contains(&word.to_lowercase()))
    }
}
#[derive(Clone,PartialEq)]
pub(super) struct Stamp {token:storage::Token,len:u64}
pub(super) fn file_stamp(path:&Path)->io::Result<Stamp>{crate::append::regular(path)?;let h=same_file::Handle::from_path(path)?;Ok(Stamp{token:storage::Token::of(&h)?,len:h.as_file().metadata()?.len()})}
struct Cached {stamp:Stamp,summary:HistorySummary,codes:String,bytes:usize}
#[derive(Default)]
pub struct HistoryCache {root:Option<(PathBuf,RootGuard)>,entries:BTreeMap<PathBuf,Cached>,bytes:usize}
impl HistoryCache {
    pub fn retained(&self)->(usize,usize){(self.entries.len(),self.bytes)}
    pub fn clear(&mut self){self.entries.clear();self.bytes=0;self.root=None;}
    fn insert(&mut self,path:PathBuf,stamp:Stamp,summary:HistorySummary,codes:String){
        if let Some(old)=self.entries.remove(&path){self.bytes-=old.bytes;}
        let bytes=summary.bytes()+codes.capacity()+path.capacity()+std::mem::size_of::<Cached>()+128;
        if bytes>MAX_BYTES{return}
        self.bytes+=bytes;self.entries.insert(path,Cached{stamp,summary,codes,bytes});
        // Keep newest filenames, avoiding a full-list LRU scan evicting all hot entries.
        while self.entries.len()>MAX_ENTRIES || self.bytes>MAX_BYTES {if let Some((_,old))=self.entries.pop_first(){self.bytes-=old.bytes;}}
    }
    pub fn read(&mut self,root:&Path,cancel:&CancellationToken)->io::Result<Vec<HistorySummary>>{
        self.read_page(root,None,usize::MAX,cancel).map(|page|page.items)
    }
    /// 按 id 从新到旧取 `before` 之后的最多 `limit` 条；只解码本页的日志。
    /// 以 id 作游标，翻页期间新增的日志不会让后续页重复或漏项。
    pub fn read_page(&mut self,root:&Path,before:Option<&str>,limit:usize,cancel:&CancellationToken)->io::Result<HistoryPage>{
        self.read_filtered_page(root,before,limit,&HistoryFilter::default(),cancel)
    }
    pub fn read_filtered_page(&mut self,root:&Path,before:Option<&str>,limit:usize,filter:&HistoryFilter,cancel:&CancellationToken)->io::Result<HistoryPage>{
        if filter.text.len()>512 || filter.from.zip(filter.until).is_some_and(|(a,b)|a>=b){return Err(io::Error::other("历史筛选范围无效"))}
        let result=self.read_inner(root,before,limit,filter,cancel);if result.is_err(){self.clear();}result
    }
    fn read_inner(&mut self,root:&Path,before:Option<&str>,limit:usize,filter:&HistoryFilter,cancel:&CancellationToken)->io::Result<HistoryPage>{
        crate::append::check_cancel(cancel)?;
        let alias=std::path::absolute(root)?;
        if !self.root.as_ref().is_some_and(|(p,guard)|*p==alias && guard.verify().is_ok()){self.clear();self.root=Some((alias,RootGuard::capture(root,false)?));}
        let guard=self.root.as_ref().unwrap().1.clone();guard.verify()?;
        let directory=super::dir(root);crate::append::confined_path(root,&directory)?;
        let entries=match fs::read_dir(directory){Ok(entries)=>Some(entries),Err(e) if e.kind()==io::ErrorKind::NotFound=>None,Err(e)=>return Err(e)};
        let mut logs=vec![];
        for entry in entries.into_iter().flatten(){crate::append::check_cancel(cancel)?;let entry=entry?;let name=entry.file_name().to_string_lossy().into_owned();if let Some(id)=name.strip_prefix("journal-").and_then(|n|n.strip_suffix(".json")){logs.push((id.to_owned(),entry.path()));}}
        logs.sort_by(|a,b|b.0.cmp(&a.0));
        let total=logs.len();
        let present:std::collections::HashSet<&Path>=logs.iter().map(|(_,p)|p.as_path()).collect();
        self.entries.retain(|path,cached|{let keep=present.contains(path.as_path());if !keep{self.bytes-=cached.bytes;}keep});
        drop(present);
        let filtered=filter.active();
        let start=if filtered {0} else {before.map_or(0,|before|logs.partition_point(|(id,_)|id.as_str()>=before))};
        let end=if filtered {total} else {start.saturating_add(limit).min(total)};
        let next=(end<total && end>start).then(||logs[end-1].0.clone());
        let mut out=Vec::with_capacity(limit.min(end-start));
        let mut matched=0;let mut more=false;
        for (id,path) in logs.drain(start..end) {
            crate::append::check_cancel(cancel)?;guard.verify()?;
            let result=(|| {
                crate::append::confined_path(root,&path)?;let before=file_stamp(&path)?;
                if let Some(cached)=self.entries.get(&path).filter(|c|c.stamp==before){return Ok((cached.summary.clone(),cached.codes.clone()))}
                let journal=Journal::load_cancellable(&path,cancel)?;
                let codes=journal.items.iter().map(|item|item.code.to_lowercase()).collect::<Vec<_>>().join("\n");
                let summary=HistorySummary::of(journal);
                let after=file_stamp(&path)?;if before!=after{return Err(io::Error::other("读取期间日志已变化，请重试"))}
                guard.verify()?;crate::append::check_cancel(cancel)?;
                self.insert(path.clone(),after,summary.clone(),codes.clone());Ok((summary,codes))
            })();
            let (summary,codes)=match result {Ok(value)=>value,Err(e) if e.kind()==io::ErrorKind::Interrupted=>return Err(e),Err(e)=>{if let Some(old)=self.entries.remove(&path){self.bytes-=old.bytes;}(HistorySummary::failed(id.clone(),e),String::new())}};
            if !filtered {out.push(summary)} else if filter.matches(&summary,&codes) {
                matched+=1;
                if before.is_none_or(|cursor|id.as_str()<cursor) {if out.len()<limit {out.push(summary)} else {more=true}}
            }
        }
        let (total,next)=if filtered {(matched,if more {out.last().map(|s|s.id.clone())}else{None})}else{(total,next)};
        guard.verify()?;crate::append::check_cancel(cancel)?;Ok(HistoryPage{items:out,total,next})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_budget_drops_older_summaries_without_lru_scan_thrashing(){
        let root=tempfile::tempdir().unwrap();let mut cache=HistoryCache::default();
        let j=Journal::new(root.path());j.save().unwrap();let stamp=file_stamp(&j.path()).unwrap();
        for i in 0..5000 {let path=root.path().join(format!("journal-{i:08}.json"));let mut row=HistorySummary::of(Journal::new(root.path()));row.id=format!("{i:08}");cache.insert(path,stamp.clone(),row,String::new());}
        assert_eq!(cache.retained().0,4096);assert!(cache.retained().1<=MAX_BYTES);assert!(cache.entries.contains_key(&root.path().join("journal-00004999.json")));assert!(!cache.entries.contains_key(&root.path().join("journal-00000000.json")));
    }
    #[test]
    fn journal_read_cancels_inside_a_large_file_and_returns_no_model(){
        let root=tempfile::tempdir().unwrap();let mut j=Journal::new(root.path());j.push_ops((0..5000).map(|i|crate::organize::Op::WriteFile{path:root.path().join(format!("{i}.nfo")),content:String::new()}));j.save().unwrap();
        let polls=std::cell::Cell::new(0);storage::metrics::take_reads();
        let result=Journal::load_checked(&j.path(),&||{let n=polls.get();polls.set(n+1);n>3});
        assert_eq!(result.unwrap_err().kind(),io::ErrorKind::Interrupted);assert!(storage::metrics::take_reads()<fs::metadata(j.path()).unwrap().len());
    }
    #[test]
    fn unchanged_history_reuses_only_display_summaries_and_changed_logs_are_reread(){
        let root=tempfile::tempdir().unwrap();let mut j=Journal::new(root.path());j.save().unwrap();
        let mut cache=HistoryCache::default();let cancel=CancellationToken::new();
        storage::metrics::take_reads();let first=cache.read(root.path(),&cancel).unwrap();let cold=storage::metrics::take_reads();
        assert!(cold>0);assert_eq!(cache.read(root.path(),&cancel).unwrap(),first);assert_eq!(storage::metrics::take_reads(),0);
        j.status=JournalStatus::Partial;j.save().unwrap();assert_eq!(cache.read(root.path(),&cancel).unwrap()[0].status,JournalStatus::Partial);assert!(storage::metrics::take_reads()>0);
        fs::write(j.path(),b"broken").unwrap();assert!(cache.read(root.path(),&cancel).unwrap()[0].error.is_some());assert_eq!(cache.retained().0,0);
        let cancelled=CancellationToken::new();cancelled.cancel();assert_eq!(cache.read(root.path(),&cancelled).unwrap_err().kind(),io::ErrorKind::Interrupted);
    }
    #[test]
    fn filtered_history_searches_every_code_counts_matches_and_keeps_cursor() {
        let root=tempfile::tempdir().unwrap();
        for index in 0..3 {let mut j=Journal::new(root.path());j.ts=1000+index;j.status=if index==0{JournalStatus::RolledBack}else{JournalStatus::Applied};
          j.items=(0..9).map(|i|super::super::JournalItem{code:format!("SMPL-{i:03}"),dir:root.path().into()}).collect();j.save().unwrap();}
        let cancel=CancellationToken::new();let mut cache=HistoryCache::default();
        let filter=HistoryFilter{text:"smpl-008".into(),status:Some(JournalStatus::Applied),from:Some(1001),until:Some(1003)};
        let first=cache.read_filtered_page(root.path(),None,1,&filter,&cancel).unwrap();assert_eq!(first.total,2);assert_eq!(first.items.len(),1);assert_eq!(first.items[0].codes.len(),6);
        let second=cache.read_filtered_page(root.path(),first.next.as_deref(),1,&filter,&cancel).unwrap();assert_eq!(second.total,2);assert_eq!(second.items.len(),1);assert_eq!(second.next,None);assert_ne!(first.items[0].id,second.items[0].id);
        storage::metrics::take_reads();cache.read_filtered_page(root.path(),None,1,&filter,&cancel).unwrap();assert_eq!(storage::metrics::take_reads(),0);
        assert_eq!(cache.read_page(root.path(),None,1,&cancel).unwrap().total,3);
    }

    #[test]
    fn pages_follow_a_stable_cursor_and_cold_pages_decode_only_their_logs(){
        let root=tempfile::tempdir().unwrap();
        for _ in 0..5 {let mut j=Journal::new(root.path());j.push_ops((0..200).map(|i|crate::organize::Op::WriteFile{path:root.path().join(format!("{i}.nfo")),content:String::new()}));j.save().unwrap();}
        let cancel=CancellationToken::new();
        let all=HistoryCache::default().read(root.path(),&cancel).unwrap();let ids=all.iter().map(|s|s.id.clone()).collect::<Vec<_>>();
        storage::metrics::take_reads();
        let mut cold=HistoryCache::default();let first=cold.read_page(root.path(),None,2,&cancel).unwrap();let page_bytes=storage::metrics::take_reads();
        HistoryCache::default().read(root.path(),&cancel).unwrap();let full_bytes=storage::metrics::take_reads();
        assert!(page_bytes*2<full_bytes,"page {page_bytes} full {full_bytes}");
        assert_eq!(first.total,5);assert_eq!(first.items.iter().map(|s|&s.id).collect::<Vec<_>>(),ids[..2].iter().collect::<Vec<_>>());assert_eq!(first.next.as_deref(),Some(ids[1].as_str()));
        // A log created after the first page must not shift the next page.
        let newer=Journal::new(root.path());newer.save().unwrap();
        let second=cold.read_page(root.path(),first.next.as_deref(),2,&cancel).unwrap();
        assert_eq!(second.total,6);assert_eq!(second.items.iter().map(|s|&s.id).collect::<Vec<_>>(),ids[2..4].iter().collect::<Vec<_>>());
        let last=cold.read_page(root.path(),second.next.as_deref(),2,&cancel).unwrap();
        assert_eq!(last.items.iter().map(|s|&s.id).collect::<Vec<_>>(),vec![&ids[4]]);assert_eq!(last.next,None);
    }
}

#[cfg(test)]
mod growth_tests {
    use super::*;
    #[test]
    #[ignore="isolated summary elapsed and log read bytes"]
    fn history_summary_growth(){
        for (histories,ops) in [(10,100),(100,1000),(500,1000)] {
            let t=tempfile::tempdir().unwrap();for h in 0..histories {let mut j=Journal::new(t.path());j.push_ops((0..ops).map(|i|crate::organize::Op::WriteFile{path:t.path().join(format!("{h}-{i}.nfo")),content:String::new()}));j.save().unwrap();}
            storage::metrics::take_reads();let start=std::time::Instant::now();let full=Journal::inspect(t.path()).unwrap();let old_ms=start.elapsed().as_millis();let old_bytes=storage::metrics::take_reads();let ids=full.iter().map(|e|e.id.clone()).collect::<Vec<_>>();drop(full);
            let mut cache=HistoryCache::default();let cancel=CancellationToken::new();let start=std::time::Instant::now();let cold=cache.read(t.path(),&cancel).unwrap();let cold_ms=start.elapsed().as_millis();let cold_bytes=storage::metrics::take_reads();let start=std::time::Instant::now();let hot=cache.read(t.path(),&cancel).unwrap();let hot_ms=start.elapsed().as_millis();let hot_bytes=storage::metrics::take_reads();
            assert_eq!(cold,hot);assert_eq!(cold.iter().map(|s|s.id.clone()).collect::<Vec<_>>(),ids);assert_eq!(hot_bytes,0);assert!(cache.retained().1<=MAX_BYTES);
            let mut paged=HistoryCache::default();let start=std::time::Instant::now();let page=paged.read_page(t.path(),None,50,&cancel).unwrap();let page_ms=start.elapsed().as_millis();let page_bytes=storage::metrics::take_reads();
            assert_eq!(page.items,cold[..cold.len().min(50)]);assert_eq!(page.total,histories);
            println!("history n={histories} ops={ops} full={old_ms}ms/{old_bytes}B cold={cold_ms}ms/{cold_bytes}B hot={hot_ms}ms/{hot_bytes}B page50-cold={page_ms}ms/{page_bytes}B cached={:?}",cache.retained());
        }
    }
}
