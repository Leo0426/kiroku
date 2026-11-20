use async_trait::async_trait;
use kiroku_core::{code::{Code,parse_code},metadata::PartialMetadata,net::Http,pipeline::Scraper,settings::Settings,source::{Source,SourceError,SourceId}};
use std::{sync::{Arc,Mutex},path::Path,time::Duration};
use tokio_util::sync::CancellationToken;
struct Fake {id:SourceId,calls:Arc<Mutex<Vec<String>>>,bad:bool,hang:bool}
#[async_trait]
impl Source for Fake {
 fn id(&self)->SourceId{self.id}
 fn supports(&self,_:&Code)->bool{true}
 async fn fetch(&self,_:&Code,_:&Http)->Result<PartialMetadata,SourceError>{
  self.calls.lock().unwrap().push(self.id.into());
  if self.hang {std::future::pending::<()>().await;}
  if self.bad{return Err(SourceError::NotFound)}
  Ok(PartialMetadata{title:Some("Neutral title".into()),..Default::default()})
 }
}
fn scraper(path:&Path,settings:Settings,calls:Arc<Mutex<Vec<String>>>,bad:bool,hang:bool)->Scraper{
 let sources=["javdb","javbus","jav321"].into_iter().map(|id|Box::new(Fake{id,calls:calls.clone(),bad:bad&&id=="javbus",hang:hang&&id!="javdb"}) as Box<dyn Source>).collect();
 Scraper::with_sources(settings,sources).unwrap().with_import_cache(path.into())
}
#[tokio::test]
async fn restart_reuses_successes_retries_failures_and_honors_configuration_and_force(){
 let temp=tempfile::tempdir().unwrap();let calls=Arc::new(Mutex::new(vec![]));let settings=Settings::default();let code=parse_code("ABC-001").unwrap();let cancel=CancellationToken::new();
 scraper(temp.path(),settings.clone(),calls.clone(),true,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();
 assert_eq!(calls.lock().unwrap().len(),3);calls.lock().unwrap().clear();
 scraper(temp.path(),settings.clone(),calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();
 assert_eq!(*calls.lock().unwrap(),vec!["javbus"]);calls.lock().unwrap().clear();
 scraper(temp.path(),settings.clone(),calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert!(calls.lock().unwrap().is_empty());
 let mut changed=settings.clone();changed.sources.iter_mut().find(|s|s.id=="javdb").unwrap().cookie="private-test-cookie".into();
 scraper(temp.path(),changed,calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(*calls.lock().unwrap(),vec!["javdb"]);
 for entry in std::fs::read_dir(temp.path().join("sources")).unwrap(){assert!(!String::from_utf8_lossy(&std::fs::read(entry.unwrap().path()).unwrap()).contains("private-test-cookie"));}
 calls.lock().unwrap().clear();
 scraper(temp.path(),settings,calls.clone(),false,false).scrape_cached(&code,&Default::default(),true,&cancel,&|_|{}).await.unwrap();assert_eq!(calls.lock().unwrap().len(),3);
}
#[tokio::test]
async fn cancellation_while_other_sources_wait_keeps_completed_source(){
 let temp=tempfile::tempdir().unwrap();let calls=Arc::new(Mutex::new(vec![]));let settings=Settings::default();let code=parse_code("ABC-001").unwrap();let cancel=CancellationToken::new();
 let sc=scraper(temp.path(),settings.clone(),calls.clone(),false,true);
 let work=sc.scrape_item(&code,&cancel,&|_|{});
 let stop=async{tokio::time::timeout(Duration::from_secs(5),async{loop{if std::fs::read_dir(temp.path().join("sources")).is_ok_and(|d|d.filter_map(Result::ok).any(|e|e.file_name().to_string_lossy().len()==64)){break}tokio::time::sleep(Duration::from_millis(10)).await}}).await.unwrap();cancel.cancel();};
 let (result,_)=tokio::join!(work,stop);assert!(result.is_err());calls.lock().unwrap().clear();
 scraper(temp.path(),settings,calls.clone(),false,false).scrape_item(&code,&CancellationToken::new(),&|_|{}).await.unwrap();
 let called=calls.lock().unwrap();assert_eq!(called.len(),2);assert!(!called.iter().any(|s|s=="javdb"));
}
#[tokio::test]
async fn corrupt_cache_and_cleared_cache_are_downloaded_again(){
 let temp=tempfile::tempdir().unwrap();let calls=Arc::new(Mutex::new(vec![]));let settings=Settings::default();let code=parse_code("ABC-001").unwrap();let cancel=CancellationToken::new();
 scraper(temp.path(),settings.clone(),calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();calls.lock().unwrap().clear();
 let file=std::fs::read_dir(temp.path().join("sources")).unwrap().next().unwrap().unwrap().path();std::fs::write(file,b"torn cache").unwrap();
 scraper(temp.path(),settings.clone(),calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(calls.lock().unwrap().len(),1);
 kiroku_core::import_cache::ImportCache::new(temp.path().into()).clear().unwrap();calls.lock().unwrap().clear();
 scraper(temp.path(),settings,calls.clone(),false,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(calls.lock().unwrap().len(),3);
}
#[tokio::test]
async fn images_survive_new_http_client_and_corrupt_or_invalid_images_are_retried(){
 use std::io::{Read,Write};
 let temp=tempfile::tempdir().unwrap();let mut png=std::io::Cursor::new(Vec::new());image::DynamicImage::new_rgb8(3,3).write_to(&mut png,image::ImageFormat::Png).unwrap();let image=png.into_inner();
 let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/image",listener.local_addr().unwrap());
 let bytes=image.clone();let server=std::thread::spawn(move||{for body in [b"<html>blocked</html>".to_vec(),bytes.clone(),bytes]{let(mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let mut req=[0;4096];stream.read(&mut req).unwrap();write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();stream.write_all(&body).unwrap();}});
 let http=||Http::new(Duration::ZERO,None).unwrap().with_retry(kiroku_core::net::Retry{attempts:1,unit:Duration::ZERO}).with_image_cache(temp.path().into(),"test".into());
 assert!(http().get_image(&url).await.unwrap().1.starts_with(b"<html>"));
 assert_eq!(http().get_image(&url).await.unwrap().1,image);
 assert_eq!(http().for_images().get_image(&url).await.unwrap().1,image);
 let file=std::fs::read_dir(temp.path().join("images")).unwrap().next().unwrap().unwrap().path();std::fs::write(file,b"truncated").unwrap();
 assert_eq!(http().get_image(&url).await.unwrap().1,image);server.join().unwrap();
 assert_eq!(http().get_image(&url).await.unwrap().1,image); // Server is gone; only persistent cache can succeed.
 assert!(Http::new(Duration::ZERO,None).unwrap().with_retry(kiroku_core::net::Retry{attempts:1,unit:Duration::ZERO}).with_image_cache(temp.path().into(),"different-account".into()).get_image(&url).await.is_err());
}

#[tokio::test]
async fn cached_only_resume_never_requests_missing_sites(){
 let temp=tempfile::tempdir().unwrap();let calls=Arc::new(Mutex::new(vec![]));let settings=Settings::default();let code=parse_code("ABC-001").unwrap();let cancel=CancellationToken::new();
 scraper(temp.path(),settings.clone(),calls.clone(),true,false).scrape_item(&code,&cancel,&|_|{}).await.unwrap();calls.lock().unwrap().clear();
 let sc=scraper(temp.path(),settings,calls.clone(),false,false).with_cached_sources_only();
 sc.scrape_item(&code,&cancel,&|_|panic!("cached resume must not request sites")).await.unwrap();
 assert!(sc.scrape_item(&parse_code("ABC-999").unwrap(),&cancel,&|_|panic!("cache miss must not request sites")).await.is_err());
 assert!(calls.lock().unwrap().is_empty());
}


#[tokio::test]
async fn cache_summary_distinguishes_reuse_invalidation_damage_and_cache_only_skip(){
 let temp=tempfile::tempdir().unwrap();let calls=Arc::new(Mutex::new(vec![]));let settings=Settings::default();let code=parse_code("ABC-001").unwrap();let cancel=CancellationToken::new();
 let sc=scraper(temp.path(),settings.clone(),calls.clone(),false,false);
 sc.scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(sc.cache_summary().missing,3);
 let sc=scraper(temp.path(),settings.clone(),calls.clone(),false,false);
 sc.scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(sc.cache_summary().reused,3);
 let mut changed=settings.clone();changed.sources.iter_mut().find(|s|s.id=="javdb").unwrap().cookie="neutral-secret".into();
 let sc=scraper(temp.path(),changed,calls.clone(),false,false);
 sc.scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(sc.cache_summary().invalidated,1);assert_eq!(sc.cache_summary().reused,2);
 let file=std::fs::read_dir(temp.path().join("sources")).unwrap().next().unwrap().unwrap().path();std::fs::write(file,b"broken").unwrap();
 let sc=scraper(temp.path(),settings.clone(),calls.clone(),false,false);
 sc.scrape_item(&code,&cancel,&|_|{}).await.unwrap();assert_eq!(sc.cache_summary().corrupt,1);
 let sc=scraper(temp.path(),settings,calls.clone(),false,false).with_cached_sources_only();
 assert!(matches!(sc.scrape_item(&parse_code("ABC-999").unwrap(),&cancel,&|_|panic!("no network")).await,Err(kiroku_core::pipeline::ScrapeError::CacheMiss)));
 assert_eq!(sc.cache_summary().skipped_movies,1);
 let cache=kiroku_core::import_cache::ImportCache::new(temp.path().into());
 let usage=cache.usage(&cancel).unwrap();assert_eq!(usage.sources.files,3);assert!(usage.sources.bytes>0);assert_eq!(usage.images.files,0);
 cancel.cancel();assert_eq!(cache.usage(&cancel).unwrap_err().kind(),std::io::ErrorKind::Interrupted);
}
