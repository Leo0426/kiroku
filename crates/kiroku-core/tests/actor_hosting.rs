use kiroku_core::{actor_hosting::{self, Config}, metadata::{Actor, Gender}, nfo, revision, settings::Settings, Metadata};
use std::{fs, io::{Cursor, Read, Write}};
use tokio_util::sync::CancellationToken;

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Metadata, Settings) {
    let root = tempfile::tempdir().unwrap(); let cache = tempfile::tempdir().unwrap();
    let dir = root.path().join("SMPL-001"); fs::create_dir(&dir).unwrap();
    let mut meta = Metadata { code: "SMPL-001".into(), ..Default::default() };
    meta.fields.actors = vec![Actor::new("Performer A", Gender::Female).with_thumb("http://127.0.0.1:1/image.jpg")];
    let settings = Settings { actor_hosting: Config { enabled:true, directory:cache.path().to_str().unwrap().into(), base_url:"http://nas.home.arpa:8088/actors".into() }, ..Default::default() };
    (root, cache, meta, settings)
}
fn write_meta(root: &std::path::Path, meta:&Metadata) {
    let dir=root.join(&meta.code);
    fs::write(dir.join("kiroku.json"),serde_json::to_vec(meta).unwrap()).unwrap();
    fs::write(dir.join("movie.nfo"),nfo::to_nfo(meta).replace("</movie>","<custom>untouched</custom></movie>")).unwrap();
}
fn server() -> (String, std::thread::JoinHandle<()>) {
    let l=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let addr=l.local_addr().unwrap();
    let thread=std::thread::spawn(move||{
        let(mut s,_)=l.accept().unwrap();s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut req=[0;4096];let n=s.read(&mut req).unwrap();assert!(String::from_utf8_lossy(&req[..n]).to_lowercase().contains("referer: http://127.0.0.1/"));
        let mut png=Cursor::new(Vec::new());image::DynamicImage::new_rgb8(3,3).write_to(&mut png,image::ImageFormat::Png).unwrap();let bytes=png.into_inner();
        write!(s,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();s.write_all(&bytes).unwrap();
    });(format!("http://{addr}/image.jpg"),thread)
}

#[tokio::test]
async fn preview_publish_rehost_and_rollback_preserve_sources_and_shared_cache() {
    let(root,cache,mut meta,mut settings)=fixture();let(url,thread)=server();meta.fields.actors[0].thumb=Some(url.clone());
    write_meta(root.path(),&meta);let dir=root.path().join(&meta.code);let old_nfo=fs::read(dir.join("movie.nfo")).unwrap();let old_side=fs::read(dir.join("kiroku.json")).unwrap();
    let capture=revision::capture(root.path(),&dir,&meta.code).unwrap();
    let(after,plan,warnings)=actor_hosting::prepare(&meta,&settings,&CancellationToken::new()).await.unwrap();thread.join().unwrap();
    assert_eq!(fs::read_dir(cache.path()).unwrap().count(),0,"preview must not write");
    assert_eq!(after.fields.actors,meta.fields.actors,"source URL remains intact");
    let hosted=&after.hosted_actor_images[&url];assert!(hosted.url.starts_with("http://nas.home.arpa:8088/actors/"));
    let preview=capture.prepare_after(after.clone()).unwrap().with_actor_cache(plan,warnings).unwrap();
    let mut out=revision::apply(&preview).unwrap();assert!(out.error.is_none());
    assert!(cache.path().join(&hosted.file).exists());let xml=fs::read_to_string(dir.join("movie.nfo")).unwrap();assert!(xml.contains(&hosted.url));assert!(xml.contains("<custom>untouched</custom>"));
    // Origin server is gone: changing the hostname must reuse the verified local cache.
    settings.actor_hosting.base_url="http://new-nas.home.arpa/images/".into();
    let(rehosted,_,_)=actor_hosting::prepare(&after,&settings,&CancellationToken::new()).await.unwrap();
    assert!(rehosted.hosted_actor_images[&url].url.starts_with("http://new-nas.home.arpa/images/"));
    assert!(out.journal.rollback().is_empty());assert_eq!(fs::read(dir.join("movie.nfo")).unwrap(),old_nfo);assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(),old_side);
    assert!(cache.path().join(&hosted.file).exists(),"rollback must retain shared cache");
}

#[tokio::test]
async fn stale_metadata_missing_mount_and_corrupt_cache_never_overwrite() {
    let(root,cache,mut meta,settings)=fixture();let(url,thread)=server();meta.fields.actors[0].thumb=Some(url);write_meta(root.path(),&meta);
    let dir=root.path().join(&meta.code);let captured=revision::capture(root.path(),&dir,&meta.code).unwrap();
    let(after,plan,warnings)=actor_hosting::prepare(&meta,&settings,&CancellationToken::new()).await.unwrap();thread.join().unwrap();
    let preview=captured.prepare_after(after.clone()).unwrap().with_actor_cache(plan.clone(),warnings).unwrap();
    fs::write(dir.join("movie.nfo"),b"external edit").unwrap();assert!(revision::apply(&preview).is_err());assert_eq!(fs::read_dir(cache.path()).unwrap().count(),0);
    let file=&after.hosted_actor_images.values().next().unwrap().file;fs::write(cache.path().join(file),b"corrupt").unwrap();assert!(plan.publish().is_err());assert_eq!(fs::read(cache.path().join(file)).unwrap(),b"corrupt");
    fs::remove_file(cache.path().join(file)).unwrap();fs::remove_dir(cache.path()).unwrap();assert!(plan.publish().is_err());assert!(!cache.path().exists());
    assert!(actor_hosting::prepare(&meta,&settings,&CancellationToken::new()).await.is_err());assert!(!cache.path().exists());
}

#[test]
fn config_rejects_credentials_queries_and_non_http_urls_and_old_settings_load() {
    let(_,_,_,mut settings)=fixture();
    for url in ["file:///tmp/images", "http://user:secret@nas/images", "http://nas/images?token=x", "http://nas/#x"] {settings.actor_hosting.base_url=url.into();assert!(settings.validate().is_err());}
    let mut json=serde_json::to_value(Settings::default()).unwrap();json.as_object_mut().unwrap().remove("actor_hosting");let old:Settings=serde_json::from_value(json).unwrap();assert!(!old.actor_hosting.enabled);
}

#[tokio::test]
async fn cancelled_preview_does_not_publish() {
    let(root,cache,meta,settings)=fixture();write_meta(root.path(),&meta);
    let cancel=CancellationToken::new();cancel.cancel();
    assert!(actor_hosting::prepare(&meta,&settings,&cancel).await.unwrap_err().to_string().contains("已取消"));
    assert_eq!(fs::read_dir(cache.path()).unwrap().count(),0);
}
