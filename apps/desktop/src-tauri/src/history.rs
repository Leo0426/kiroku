//! Read-only history requests: separate from the single mutation job slot.
use crate::state::AppState;
use kiroku_core::journal::{HistoryCache,HistoryPage,HistoryFilter,HistoryDetail};
use std::{collections::{HashMap,VecDeque},sync::{Arc,Mutex,atomic::{AtomicU64,Ordering}}};
use tauri::{AppHandle,Manager,State};
use tokio_util::sync::CancellationToken;
#[derive(Default)]
pub struct Requests {active:HashMap<String,CancellationToken>,stopped:VecDeque<String>}
impl Requests {
    fn begin(&mut self,id:String)->Result<CancellationToken,String>{
        if id.is_empty()||id.len()>100{return Err("无效的读取请求".into())}
        if self.active.len()>=16 || self.active.contains_key(&id){return Err("历史读取繁忙，请稍后重试".into())}
        let token=CancellationToken::new();if self.stopped.iter().any(|s|s==&id){token.cancel();}self.stopped.retain(|s|s!=&id);
        self.active.insert(id,token.clone());Ok(token)
    }
    fn cancel(&mut self,id:&str)->bool{
        if id.is_empty()||id.len()>100{return false}
        if let Some(token)=self.active.get(id){token.cancel();return true}
        if !self.stopped.iter().any(|s|s==id){self.stopped.push_back(id.to_owned());if self.stopped.len()>128{self.stopped.pop_front();}}
        false
    }
    pub fn cancel_all(&self){for token in self.active.values(){token.cancel();}}
}
pub struct HistoryState {pub cache:Mutex<HistoryCache>,requests:Mutex<Requests>,queue:Arc<tokio::sync::Semaphore>}
impl Default for HistoryState {fn default()->Self{Self{cache:Mutex::new(HistoryCache::default()),requests:Mutex::new(Requests::default()),queue:Arc::new(tokio::sync::Semaphore::new(1))}}}
impl HistoryState {pub fn cancel_all(&self){
    self.requests.lock().unwrap_or_else(|e|e.into_inner()).cancel_all();
    // Never wait for disk I/O during a settings commit. An active read clears
    // itself on cancellation; an idle cache can release its old root now.
    if let Ok(mut cache)=self.cache.try_lock(){cache.clear();}
}}
struct Lease {app:AppHandle,id:String}
impl Drop for Lease {fn drop(&mut self){self.app.state::<AppState>().history.requests.lock().unwrap_or_else(|e|e.into_inner()).active.remove(&self.id);}}
#[tauri::command]
/// 一页历史摘要：`before` 为上一页返回的游标，`limit` 默认 50、最多 1000。
pub async fn history_list(app:AppHandle,state:State<'_,AppState>,request_id:Option<String>,before:Option<String>,limit:Option<usize>,filter:Option<HistoryFilter>)->Result<HistoryPage,String>{
    let limit=limit.unwrap_or(50).clamp(1,1000);
    static NEXT:AtomicU64=AtomicU64::new(0);
    let id=request_id.unwrap_or_else(||format!("history-{}",NEXT.fetch_add(1,Ordering::Relaxed)));
    let snapshot=state.history_snapshot()?;
    let cancel=state.history.requests.lock().unwrap_or_else(|e|e.into_inner()).begin(id.clone())?;
    let lease=Lease{app:app.clone(),id};let queue=state.history.queue.clone();
    let permit=tokio::select!{_ = cancel.cancelled()=>return Err("历史读取已取消".into()),permit=queue.acquire_owned()=>permit.map_err(|e|e.to_string())?};
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;let _lease=lease;let state=app.state::<AppState>();
        state.validate_history(&snapshot)?;
        let result=state.history.cache.lock().unwrap_or_else(|e|e.into_inner()).read_filtered_page(&snapshot.root,before.as_deref(),limit,&filter.unwrap_or_default(),&cancel).map_err(|e|e.to_string());
        if let Err(error)=state.validate_history(&snapshot){state.history.cache.lock().unwrap_or_else(|e|e.into_inner()).clear();return Err(error)}
        let result=result?;if cancel.is_cancelled(){return Err("历史读取已取消".into())}Ok(result)
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub async fn history_detail(app:AppHandle,state:State<'_,AppState>,request_id:String,journal_id:String,offset:usize)->Result<HistoryDetail,String>{
    let snapshot=state.history_snapshot()?;
    let cancel=state.history.requests.lock().unwrap_or_else(|e|e.into_inner()).begin(request_id.clone())?;
    let lease=Lease{app:app.clone(),id:request_id};let queue=state.history.queue.clone();
    let permit=tokio::select!{_ = cancel.cancelled()=>return Err("历史读取已取消".into()),permit=queue.acquire_owned()=>permit.map_err(|e|e.to_string())?};
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;let _lease=lease;let state=app.state::<AppState>();
        state.validate_history(&snapshot)?;
        let result=HistoryDetail::read(&snapshot.root,&journal_id,offset,&cancel).map_err(|e|e.to_string())?;
        state.validate_history(&snapshot)?;if cancel.is_cancelled(){return Err("历史读取已取消".into())}Ok(result)
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub fn history_cancel(state:State<'_,AppState>,request_id:String)->bool{state.history.requests.lock().unwrap_or_else(|e|e.into_inner()).cancel(&request_id)}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn read_cancellation_is_scoped_and_early_intent_survives(){let mut r=Requests::default();r.cancel("early");assert!(r.begin("early".into()).unwrap().is_cancelled());let token=r.begin("current".into()).unwrap();r.cancel("other");assert!(!token.is_cancelled());assert!(r.cancel("current"));assert!(token.is_cancelled());}
    #[test] fn cancelling_waiting_read_does_not_need_queue_owner_to_finish(){let runtime=tokio::runtime::Runtime::new().unwrap();runtime.block_on(async{let queue=Arc::new(tokio::sync::Semaphore::new(1));let _held=queue.clone().acquire_owned().await.unwrap();let cancel=CancellationToken::new();cancel.cancel();let result=tokio::select!{_ = cancel.cancelled()=>true,_ = queue.acquire_owned()=>false};assert!(result);});}
}
