use kiroku_core::{
    pipeline::{self, Scraper},
    settings::Settings,
};
use std::{collections::HashMap, fs, io::Cursor};
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
#[test]
fn large_companion_batch_can_plan_under_a_small_descriptor_limit() {
    use std::{os::unix::process::CommandExt, process::Command};

    const CHILD: &str = "KIROKU_TEST_LOW_FD_IMPORT";
    if std::env::var_os(CHILD).is_none() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "large_companion_batch_can_plan_under_a_small_descriptor_limit",
                "--nocapture",
            ])
            .env(CHILD, "1");
        // Only lower the test child's soft limit before exec; parallel tests are unaffected.
        unsafe {
            command.pre_exec(|| {
                let mut limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                limit.rlim_cur = limit.rlim_cur.min(256);
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        assert!(command.status().unwrap().success());
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("incoming");
    let library = temp.path().join("library");
    fs::create_dir(&library).unwrap();
    const FILMS: usize = 320;
    for index in 1..=FILMS {
        let code = format!("TEST-{index:03}");
        let dir = source.join(&code);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{code}.mp4")), b"synthetic video").unwrap();
        fs::write(
            dir.join(format!("{code}.nfo")),
            "<movie><title>Local title</title></movie>",
        )
        .unwrap();
    }
    let mut settings = Settings::default();
    settings.scan.min_video_mb = 0;
    let cancel = CancellationToken::new();
    let scan = pipeline::scan(
        &[source],
        Some(&library),
        &settings.video_filter(),
        &cancel,
        &mut |_| {},
    );
    assert_eq!(scan.items.len(), FILMS);
    let (_, ready) = pipeline::inspect_import(&scan.items, &cancel);
    let items = scan
        .items
        .iter()
        .map(|item| (item, item.code.as_ref().unwrap()))
        .collect::<Vec<_>>();
    let plans = pipeline::build_plans(&items, &ready, &library, "{code}", settings.plan_opts());
    let first_conflict = plans.iter().find(|plan| !plan.selected);
    assert!(
        first_conflict.is_none(),
        "{:?}",
        first_conflict.map(|plan| &plan.notices)
    );
    assert_eq!(
        plans.len(),
        FILMS,
        "every companion must be available for planning"
    );
}

#[tokio::test]
async fn import_uses_companion_nfo_and_artwork_without_a_source_route() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("incoming");
    let library = temp.path().join("library");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&library).unwrap();
    fs::write(source.join("TEST-001.mp4"), b"synthetic video").unwrap();
    let nfo = "<movie><title>Local title</title><plot>Local synopsis</plot><custom>keep me</custom></movie>";
    fs::write(source.join("TEST-001.nfo"), nfo).unwrap();
    let mut poster = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(20, 30)
        .write_to(&mut poster, image::ImageFormat::Png)
        .unwrap();
    fs::write(source.join("poster.png"), poster.into_inner()).unwrap();

    let mut settings = Settings::default();
    settings.scan.min_video_mb = 0;
    let scraper = Scraper::with_sources(settings.clone(), vec![]).unwrap();
    let cancel = CancellationToken::new();
    let items = pipeline::scan(
        &[source.clone()],
        Some(&library),
        &settings.video_filter(),
        &cancel,
        &mut |_| {},
    )
    .items;
    let (inspection, ready) = pipeline::inspect_import(&items, &cancel);
    assert_eq!(inspection.len(), 1);
    assert!(inspection[0].reusable);
    assert!(inspection[0].preview.local_cover);
    let metadata = ready.get("TEST-001").expect("local metadata is enough");
    assert_eq!(metadata.fields.title.as_deref(), Some("Local title"));
    let pairs = items
        .iter()
        .map(|item| (item, item.code.as_ref().unwrap()))
        .collect::<Vec<_>>();
    let plans = pipeline::build_plans(&pairs, &ready, &library, "{code}", settings.plan_opts());
    let outcome = pipeline::apply(plans, &scraper.http, &library, None, &cancel, &|_| {}).await;
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert_eq!(outcome.applied.len(), 1);
    let dest = &outcome.applied[0].to;
    let written = fs::read_to_string(dest.with_extension("nfo")).unwrap();
    assert!(written.contains("<custom>keep me</custom>"));
    let artwork = dest.with_file_name(format!(
        "{}-poster.jpg",
        dest.file_stem().unwrap().to_string_lossy()
    ));
    assert_eq!(
        image::load_from_memory(&fs::read(artwork).unwrap())
            .unwrap()
            .height(),
        30
    );
    assert_eq!(
        fs::read_to_string(source.join("TEST-001.nfo")).unwrap(),
        nfo
    );
}

struct TestSite(Option<String>);

#[async_trait::async_trait]
impl kiroku_core::source::Source for TestSite {
    fn id(&self) -> &'static str {
        "javdb"
    }
    fn supports(&self, _: &kiroku_core::code::Code) -> bool {
        true
    }
    async fn fetch(
        &self,
        _: &kiroku_core::code::Code,
        _: &kiroku_core::net::Http,
    ) -> Result<kiroku_core::metadata::PartialMetadata, kiroku_core::source::SourceError> {
        let Some(url) = &self.0 else {
            return Err(kiroku_core::source::SourceError::NotFound);
        };
        Ok(kiroku_core::metadata::PartialMetadata {
            title: Some("Fresh title".into()),
            poster_urls: vec![url.clone()],
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn refresh_adopts_available_values_and_preserves_local_fields_and_artwork_on_misses() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("incoming");
    let library = temp.path().join("library");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&library).unwrap();
    fs::write(source.join("TEST-002.mp4"), b"synthetic video").unwrap();
    fs::write(source.join("movie.NFO"), "<movie><title>Local title</title><plot>Local synopsis</plot><custom>keep me</custom></movie>").unwrap();
    let mut poster = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(22, 33)
        .write_to(&mut poster, image::ImageFormat::Png)
        .unwrap();
    fs::write(source.join("TEST-002-poster.png"), poster.into_inner()).unwrap();

    let mut settings = Settings::default();
    settings.scan.min_video_mb = 0;
    let cancel = CancellationToken::new();
    let items = pipeline::scan(
        &[source.clone()],
        Some(&library),
        &settings.video_filter(),
        &cancel,
        &mut |_| {},
    )
    .items;
    let offline = Scraper::with_sources(settings.clone(), vec![Box::new(TestSite(None))]).unwrap();
    let default =
        pipeline::scrape_import(&offline, &items, &HashMap::new(), false, &cancel, &|_| {}).await;
    let previous = HashMap::from([(
        "TEST-002".to_owned(),
        default["TEST-002"].as_ref().unwrap().clone(),
    )]);
    let missed = pipeline::scrape_import(&offline, &items, &previous, true, &cancel, &|_| {}).await;
    assert_eq!(
        missed["TEST-002"].as_ref().unwrap().fields.title.as_deref(),
        Some("Local title")
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let missing_image = format!("http://{}/image.png", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut cached = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(19, 29)
            .write_to(&mut cached, image::ImageFormat::Png)
            .unwrap();
        let cached = cached.into_inner();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            stream.read(&mut [0; 4096]).unwrap();
            if index == 0 {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    cached.len()
                )
                .unwrap();
                stream.write_all(&cached).unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
    });
    let online = Scraper::with_sources(
        settings.clone(),
        vec![Box::new(TestSite(Some(missing_image.clone())))],
    )
    .unwrap()
    .with_import_cache(temp.path().join("cache"));
    assert_eq!(online.http.get_image(&missing_image).await.unwrap().0, 200);
    let results = pipeline::scrape_import(&online, &items, &previous, true, &cancel, &|_| {}).await;
    let metadata = results["TEST-002"].as_ref().unwrap();
    assert_eq!(metadata.fields.title.as_deref(), Some("Fresh title"));
    assert_eq!(metadata.fields.plot.as_deref(), Some("Local synopsis"));
    let pairs = items
        .iter()
        .map(|item| (item, item.code.as_ref().unwrap()))
        .collect::<Vec<_>>();
    let plans = pipeline::build_plans(
        &pairs,
        &HashMap::from([(metadata.code.clone(), metadata.clone())]),
        &library,
        "{code}",
        settings.plan_opts(),
    );
    let outcome = pipeline::apply(plans, &online.http, &library, None, &cancel, &|_| {}).await;
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    let dest = &outcome.applied[0].to;
    let nfo = fs::read_to_string(dest.with_extension("nfo")).unwrap();
    assert!(
        nfo.contains("Fresh title")
            && nfo.contains("Local synopsis")
            && nfo.contains("<custom>keep me</custom>")
    );
    let artwork = dest.with_file_name(format!(
        "{}-poster.jpg",
        dest.file_stem().unwrap().to_string_lossy()
    ));
    assert_eq!(
        image::load_from_memory(&fs::read(artwork).unwrap())
            .unwrap()
            .height(),
        33
    );
    server.join().unwrap();
}

#[tokio::test]
async fn generic_companions_are_not_borrowed_by_unrelated_videos_in_one_folder() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path();
    for name in ["TEST-008.mp4", "TEST-009.mp4"] {
        fs::write(source.join(name), b"synthetic video").unwrap();
    }
    fs::write(
        source.join("movie.nfo"),
        "<movie><title>Generic record</title></movie>",
    )
    .unwrap();
    fs::write(
        source.join("TEST-008.nfo"),
        "<movie><title>Named companion</title></movie>",
    )
    .unwrap();
    fs::write(
        source.join("TEST-009.nfo"),
        "<movie><num>TEST-008</num><title>Wrong number</title></movie>",
    )
    .unwrap();
    let mut settings = Settings::default();
    settings.scan.min_video_mb = 0;
    let scraper = Scraper::with_sources(settings.clone(), vec![]).unwrap();
    let cancel = CancellationToken::new();
    let items = pipeline::scan(
        &[source.to_path_buf()],
        None,
        &settings.video_filter(),
        &cancel,
        &mut |_| {},
    )
    .items;
    let results =
        pipeline::scrape_import(&scraper, &items, &HashMap::new(), false, &cancel, &|_| {}).await;
    assert_eq!(
        results["TEST-008"]
            .as_ref()
            .unwrap()
            .fields
            .title
            .as_deref(),
        Some("Named companion")
    );
    assert!(results["TEST-009"].is_err());
}
