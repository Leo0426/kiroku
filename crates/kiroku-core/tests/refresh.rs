use kiroku_core::{
    code::parse_code,
    metadata::SourceSnapshot,
    refresh::{merge_selected, select_source, set_lock},
    Metadata,
};
use std::collections::BTreeMap;
#[test]
fn locked_empty_and_nonselected_values_survive() {
    let mut old = Metadata::default();
    old.code = "ABC-001".into();
    old.fields.plot = Some("old".into());
    old.fields.studio = Some("keep".into());
    old.locked_fields.insert("title".into());
    let mut candidate = old.clone();
    candidate.fields.title = Some("new".into());
    candidate.fields.plot = Some("new plot".into());
    candidate.fields.studio = Some("discard".into());
    let next = merge_selected(&old, &candidate, &["title".into(), "plot".into()], true).unwrap();
    assert_eq!(next.fields.title, None);
    assert_eq!(next.fields.plot.as_deref(), Some("new plot"));
    assert_eq!(next.fields.studio.as_deref(), Some("keep"));
    let filled = merge_selected(&old, &candidate, &["plot".into()], false).unwrap();
    assert_eq!(filled.fields.plot, old.fields.plot);
    candidate.fields.plot = None;
    assert_eq!(
        merge_selected(&old, &candidate, &["plot".into()], true)
            .unwrap()
            .fields
            .plot,
        old.fields.plot
    );
}
#[test]
fn source_selection_locks_honest_value_and_rejects_empty() {
    let mut old = Metadata::default();
    old.code = "ABC-001".into();
    let mut raw = kiroku_core::metadata::PartialMetadata::default();
    raw.title = Some("raw title".into());
    let code = parse_code(&old.code).unwrap();
    old.raw_sources = BTreeMap::from([(
        "javdb".into(),
        SourceSnapshot::new(&code, "https://javdb.com", &raw),
    )]);
    let next = select_source(&old, "title", "javdb").unwrap();
    assert_eq!(next.fields.title, raw.title);
    assert_eq!(next.provenance["title"], "javdb");
    assert!(next.locked_fields.contains("title"));
    assert!(select_source(&old, "plot", "javdb").is_err());
    assert!(select_source(&old, "title", "missing").is_err());
    assert!(set_lock(&old, "poster_urls", true)
        .unwrap()
        .locked_fields
        .contains("poster_urls"));
    assert!(set_lock(&old, "invalid", true).is_err());
}
use async_trait::async_trait;
use kiroku_core::{
    code::Code,
    net::Http,
    pipeline::Scraper,
    settings::Settings,
    source::{Source, SourceError, SourceId},
};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
struct Fake {
    id: SourceId,
    calls: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl Source for Fake {
    fn id(&self) -> SourceId {
        self.id
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(
        &self,
        _: &Code,
        _: &Http,
    ) -> Result<kiroku_core::metadata::PartialMetadata, SourceError> {
        self.calls.lock().unwrap().push(self.id.into());
        Ok(kiroku_core::metadata::PartialMetadata {
            title: Some(self.id.into()),
            ..Default::default()
        })
    }
}
#[tokio::test]
async fn matching_cache_fetches_only_missing_changed_or_forced_sources() {
    let calls = Arc::new(Mutex::new(vec![]));
    let settings = Settings::default();
    let code = parse_code("ABC-001").unwrap();
    let sc = Scraper::with_sources(
        settings.clone(),
        settings
            .sources
            .iter()
            .map(|s| {
                Box::new(Fake {
                    id: match s.id.as_str() {
                        "javdb" => "javdb",
                        "javbus" => "javbus",
                        _ => "jav321",
                    },
                    calls: calls.clone(),
                }) as Box<dyn Source>
            })
            .collect(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let first = sc
        .scrape_cached(&code, &BTreeMap::new(), false, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(calls.lock().unwrap().len(), 3);
    calls.lock().unwrap().clear();
    sc.scrape_cached(&code, &first.raw_sources, false, &cancel, &|_| {})
        .await
        .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    let mut missing = first.raw_sources.clone();
    missing.remove("javbus");
    sc.scrape_cached(&code, &missing, false, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["javbus"]);
    calls.lock().unwrap().clear();
    let mut changed = first.raw_sources.clone();
    changed.get_mut("javdb").unwrap().base = "https://old.example".into();
    sc.scrape_cached(&code, &changed, false, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["javdb"]);
    calls.lock().unwrap().clear();
    sc.scrape_cached(&code, &first.raw_sources, true, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(calls.lock().unwrap().len(), 3);
    cancel.cancel();
    assert!(sc
        .scrape_cached(&code, &first.raw_sources, false, &cancel, &|_| {})
        .await
        .is_err());
}
#[test]
fn disk_guard_catches_external_change_before_candidate_preview_and_undo_restores_raw() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join("ABC-001");
    std::fs::create_dir(&dir).unwrap();
    let mut before = Metadata::default();
    before.code = "ABC-001".into();
    before.fields.title = Some("old".into());
    let side = dir.join("kiroku.json");
    let nfo = dir.join("movie.nfo");
    let video = dir.join("ABC-001.mp4");
    std::fs::write(&side, serde_json::to_vec(&before).unwrap()).unwrap();
    std::fs::write(&nfo, kiroku_core::nfo::to_nfo(&before)).unwrap();
    std::fs::write(&video, b"video").unwrap();
    let original_side = std::fs::read(&side).unwrap();
    let original_nfo = std::fs::read(&nfo).unwrap();
    let guard = kiroku_core::revision::prepare(root, &dir, &before.code, before.clone()).unwrap();
    let mut after = set_lock(&before, "title", true).unwrap();
    after.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::new(
            &parse_code(&before.code).unwrap(),
            "https://javdb.com",
            &before.fields,
        ),
    );
    std::fs::write(&side, b"changed externally").unwrap();
    assert!(guard.prepare_after(after.clone()).is_err());
    std::fs::write(&side, &original_side).unwrap();
    let preview = guard.prepare_after(after).unwrap();
    let mut outcome = kiroku_core::revision::apply(&preview).unwrap();
    assert!(outcome.error.is_none());
    assert!(std::fs::read_to_string(&side)
        .unwrap()
        .contains("raw_sources"));
    assert!(outcome.journal.rollback().is_empty());
    assert_eq!(std::fs::read(&side).unwrap(), original_side);
    assert_eq!(std::fs::read(&nfo).unwrap(), original_nfo);
    assert_eq!(std::fs::read(&video).unwrap(), b"video");
}
#[test]
fn snapshot_never_serializes_url_credentials() {
    let code = parse_code("ABC-001").unwrap();
    let raw = kiroku_core::metadata::PartialMetadata {
        source_url: Some("https://user:secret@example.com/movie".into()),
        poster_urls: vec!["https://cdn.example/a?signature=public".into()],
        title: Some("x".into()),
        ..Default::default()
    };
    let snap = SourceSnapshot::new(&code, "https://user:secret@example.com", &raw);
    let text = serde_json::to_string(&snap).unwrap();
    assert!(!text.contains("secret"));
    assert!(!text.contains("user:"));
    assert!(text.contains("signature=public"));
    assert!(snap.matches(&code, "https://user:secret@example.com"));
    assert!(!snap.matches(&code, "https://user:changed@example.com"));
}
struct Failing {
    calls: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl Source for Failing {
    fn id(&self) -> SourceId {
        "javdb"
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(
        &self,
        _: &Code,
        _: &Http,
    ) -> Result<kiroku_core::metadata::PartialMetadata, SourceError> {
        self.calls.lock().unwrap().push("javdb".into());
        Err(SourceError::Parse("bad response".into()))
    }
}
#[tokio::test]
async fn failed_refresh_keeps_prior_snapshot_and_does_not_claim_old_fetch_success() {
    let code = parse_code("ABC-001").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let settings = Settings::default();
    let mut cached = BTreeMap::new();
    for cfg in &settings.sources {
        cached.insert(
            cfg.id.clone(),
            SourceSnapshot::new(
                &code,
                &cfg.base,
                &kiroku_core::metadata::PartialMetadata {
                    title: Some("old".into()),
                    ..Default::default()
                },
            ),
        );
    }
    let sc = Scraper::with_sources(
        settings.clone(),
        vec![
            Box::new(Failing {
                calls: calls.clone(),
            }),
            Box::new(Fake {
                id: "javbus",
                calls: calls.clone(),
            }),
        ],
    )
    .unwrap();
    let result = sc
        .scrape_cached(&code, &cached, true, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(result.raw_sources["javdb"], cached["javdb"]);
    assert_eq!(result.fields.title.as_deref(), Some("javbus"));
    let all_fail = Scraper::with_sources(settings, vec![Box::new(Failing { calls })]).unwrap();
    assert!(all_fail
        .scrape_cached(&code, &cached, true, &CancellationToken::new(), &|_| {})
        .await
        .is_err());
}
#[test]
fn snapshots_and_empty_locks_survive_restart_reindex_and_old_sidecar_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ABC-001");
    std::fs::create_dir(&dir).unwrap();
    let mut meta = Metadata {
        code: "ABC-001".into(),
        ..Default::default()
    };
    meta.locked_fields.insert("title".into());
    meta.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::new(
            &parse_code(&meta.code).unwrap(),
            "https://javdb.com",
            &kiroku_core::metadata::PartialMetadata {
                title: Some("raw".into()),
                ..Default::default()
            },
        ),
    );
    std::fs::write(dir.join("movie.nfo"), kiroku_core::nfo::to_nfo(&meta)).unwrap();
    std::fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    std::fs::write(dir.join("ABC-001.mp4"), b"video").unwrap();
    {
        let lib = kiroku_core::library::Library::open(tmp.path()).unwrap();
        lib.reindex_dir(&dir).unwrap();
    }
    let lib = kiroku_core::library::Library::open(tmp.path()).unwrap();
    assert_eq!(lib.get(&meta.code).unwrap().unwrap().meta, meta);
    let mut old = serde_json::to_value(&meta).unwrap();
    old.as_object_mut().unwrap().remove("raw_sources");
    assert!(serde_json::from_value::<Metadata>(old)
        .unwrap()
        .raw_sources
        .is_empty());
}
#[test]
fn malformed_cache_entry_does_not_hide_other_metadata_or_valid_snapshots() {
    let mut meta = Metadata {
        code: "ABC-001".into(),
        ..Default::default()
    };
    meta.fields.title = Some("keep".into());
    meta.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::new(
            &parse_code(&meta.code).unwrap(),
            "https://javdb.com",
            &meta.fields,
        ),
    );
    let mut value = serde_json::to_value(&meta).unwrap();
    value["raw_sources"]["broken"] = serde_json::json!({"fetched_at":"bad"});
    let decoded: Metadata = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.fields.title, meta.fields.title);
    assert_eq!(decoded.raw_sources, meta.raw_sources);
}
struct Japanese;
#[async_trait]
impl Source for Japanese {
    fn id(&self) -> SourceId {
        "javdb"
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(
        &self,
        _: &Code,
        _: &Http,
    ) -> Result<kiroku_core::metadata::PartialMetadata, SourceError> {
        Ok(kiroku_core::metadata::PartialMetadata {
            title: Some("タイトル".into()),
            plot: Some("あらすじ".into()),
            ..Default::default()
        })
    }
}
fn ai_server(
    stop: &'static str,
) -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = vec![];
        let mut buf = [0; 4096];
        loop {
            let n = socket.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let len = header
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + len {
                    break;
                }
            }
        }
        tx.send(String::from_utf8(bytes).unwrap()).unwrap();
        let body=serde_json::json!({"stop_reason":stop,"content":[{"type":"text","text":"{\"title\":\"unexpected\",\"plot\":\"中文简介\"}"}]}).to_string();
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    (address, rx, handle)
}
#[tokio::test]
async fn only_selected_unlocked_plot_is_translated_and_raw_cache_stays_pre_ai() {
    let (address, request, handle) = ai_server("end_turn");
    let mut settings = Settings::default();
    settings.ai.enabled = true;
    settings.ai.providers[0].api_key = "fake".into();
    settings.ai.providers[0].base_url = address;
    let sc = Scraper::with_sources(settings, vec![Box::new(Japanese)]).unwrap();
    let code = parse_code("ABC-001").unwrap();
    let mut before = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    before.locked_fields.insert("title".into());
    let after = sc
        .refresh_item(
            &code,
            &before,
            &["title".into(), "plot".into()],
            true,
            true,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(after.fields.title, None);
    assert_eq!(after.fields.plot.as_deref(), Some("中文简介"));
    assert_eq!(after.provenance["plot"], "ai");
    assert_eq!(
        after.raw_sources["javdb"].fields.plot.as_deref(),
        Some("あらすじ")
    );
    let body = request.recv().unwrap();
    assert!(!body.contains("タイトル"));
    assert!(body.contains("あらすじ"));
    handle.join().unwrap();
}
#[tokio::test]
async fn translation_failure_yields_no_candidate_and_preserves_old_metadata() {
    let (address, request, handle) = ai_server("max_tokens");
    let mut settings = Settings::default();
    settings.ai.enabled = true;
    settings.ai.providers[0].api_key = "fake".into();
    settings.ai.providers[0].base_url = address;
    let sc = Scraper::with_sources(settings, vec![Box::new(Japanese)]).unwrap();
    let code = parse_code("ABC-001").unwrap();
    let before = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let old = before.clone();
    assert!(sc
        .refresh_item(
            &code,
            &before,
            &["plot".into()],
            true,
            true,
            &CancellationToken::new()
        )
        .await
        .unwrap_err()
        .contains("AI"));
    assert_eq!(before, old);
    request.recv().unwrap();
    handle.join().unwrap();
}
#[tokio::test]
async fn credential_identity_reuses_same_cookie_and_invalidates_only_changed_source() {
    let mut settings = Settings::default();
    settings.sources[0].cookie = "private-cookie-A".into();
    settings.proxy = Some("http://proxyuser:private-proxy-pass@127.0.0.1:9".into());
    let calls = Arc::new(Mutex::new(vec![]));
    let code = parse_code("ABC-001").unwrap();
    // Use no real network: fake sources receive an Http configured with this fictional proxy.
    let build = |settings: Settings| {
        Scraper::with_sources(
            settings,
            vec![
                Box::new(Fake {
                    id: "javdb",
                    calls: calls.clone(),
                }),
                Box::new(Fake {
                    id: "javbus",
                    calls: calls.clone(),
                }),
            ],
        )
        .unwrap()
    };
    let first = build(settings.clone())
        .scrape_cached(
            &code,
            &BTreeMap::new(),
            false,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
    calls.lock().unwrap().clear();
    build(settings.clone())
        .scrape_cached(
            &code,
            &first.raw_sources,
            false,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    settings.sources[0].cookie = "private-cookie-B".into();
    build(settings.clone())
        .scrape_cached(
            &code,
            &first.raw_sources,
            false,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["javdb"]);
    let text = serde_json::to_string(&first).unwrap();
    assert!(!text.contains("private-cookie"));
    assert!(!text.contains("private-proxy-pass"));
    assert!(!text.contains("proxyuser"));
    calls.lock().unwrap().clear();
    settings.proxy = None;
    build(settings)
        .scrape_cached(
            &code,
            &first.raw_sources,
            false,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
    assert_eq!(calls.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn regular_import_plan_persists_source_snapshots_without_translation() {
    let settings = Settings::default();
    let sc = Scraper::with_sources(settings, vec![Box::new(Japanese)]).unwrap();
    let code = parse_code("ABC-001").unwrap();
    let meta = sc
        .scrape_item(&code, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    let plan = kiroku_core::organize::plan(
        std::path::Path::new("/fake/ABC-001.mp4"),
        &code,
        &meta,
        std::path::Path::new("/fake-library"),
        "{code}",
    );
    let content = plan
        .ops
        .iter()
        .find_map(|op| match op {
            kiroku_core::organize::Op::WriteFile { path, content }
                if path.ends_with("kiroku.json") =>
            {
                Some(content)
            }
            _ => None,
        })
        .unwrap();
    let decoded: Metadata = serde_json::from_str(content).unwrap();
    assert_eq!(
        decoded.raw_sources["javdb"].fields.title.as_deref(),
        Some("タイトル")
    );
}
#[tokio::test]
async fn original_identity_type_and_current_route_control_cache_eligibility() {
    let code = parse_code("ABC-001").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut settings = Settings::default();
    let mut cached = BTreeMap::new();
    for cfg in &settings.sources {
        cached.insert(
            cfg.id.clone(),
            SourceSnapshot::new(
                &code,
                &cfg.base,
                &kiroku_core::metadata::PartialMetadata {
                    title: Some(cfg.id.clone()),
                    ..Default::default()
                },
            ),
        );
    }
    settings
        .routes
        .insert(code.content_type, vec!["javdb".into()]);
    settings
        .field_priority
        .insert("title".into(), vec!["javbus".into()]);
    let sc = Scraper::with_sources(
        settings.clone(),
        vec![
            Box::new(Fake {
                id: "javdb",
                calls: calls.clone(),
            }),
            Box::new(Fake {
                id: "javbus",
                calls: calls.clone(),
            }),
        ],
    )
    .unwrap();
    let matched = sc
        .scrape_cached(&code, &cached, false, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(matched.fields.title.as_deref(), Some("javdb"));
    assert!(calls.lock().unwrap().is_empty());
    cached.get_mut("javdb").unwrap().code = "ABC-1".into();
    sc.scrape_cached(&code, &cached, false, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["javdb"]);
    calls.lock().unwrap().clear();
    cached.get_mut("javdb").unwrap().code = code.id.clone();
    cached.get_mut("javdb").unwrap().content_type = kiroku_core::code::ContentType::Fc2;
    sc.scrape_cached(&code, &cached, false, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["javdb"]);
}
#[test]
fn adopted_refresh_source_links_follow_new_domain_only_for_adopted_sources() {
    let before = Metadata {
        code: "ABC-001".into(),
        source_urls: BTreeMap::from([
            ("javdb".into(), "https://old.example/movie".into()),
            ("javbus".into(), "https://keep.example/movie".into()),
        ]),
        ..Default::default()
    };
    let mut candidate = before.clone();
    candidate.fields.title = Some("new".into());
    candidate.provenance.insert("title".into(), "javdb".into());
    candidate.source_urls.insert(
        "javdb".into(),
        "https://user:private@new.example/movie".into(),
    );
    candidate
        .source_urls
        .insert("javbus".into(), "https://unused.example".into());
    let after = merge_selected(&before, &candidate, &["title".into()], true).unwrap();
    assert_eq!(after.source_urls["javdb"], "https://new.example/movie");
    assert_eq!(after.source_urls["javbus"], before.source_urls["javbus"]);
    let locked = set_lock(&before, "title", true).unwrap();
    assert_eq!(
        merge_selected(&locked, &candidate, &["title".into()], true)
            .unwrap()
            .source_urls,
        locked.source_urls
    );
}
#[test]
fn source_candidates_exclude_empty_disabled_unrouted_and_stale_snapshots() {
    let code = parse_code("ABC-001").unwrap();
    let mut settings = Settings::default();
    let mut meta = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    for cfg in &settings.sources {
        meta.raw_sources.insert(
            cfg.id.clone(),
            SourceSnapshot::configured(
                &code,
                cfg,
                None,
                &kiroku_core::metadata::PartialMetadata {
                    title: Some(cfg.id.clone()),
                    ..Default::default()
                },
            ),
        );
    }
    let candidates = kiroku_core::refresh::source_candidates(&meta, &settings);
    assert_eq!(candidates["title"].len(), 3);
    assert!(candidates["plot"].is_empty());
    settings.sources[0].cookie = "changed".into();
    settings.sources[1].enabled = false;
    settings
        .routes
        .insert(code.content_type, vec!["javdb".into(), "javbus".into()]);
    assert!(kiroku_core::refresh::source_candidates(&meta, &settings)["title"].is_empty());
}

#[test]
fn adopting_source_without_current_link_drops_its_stale_link() {
    let before = Metadata {
        code: "ABC-001".into(),
        source_urls: BTreeMap::from([("javdb".into(), "https://old.example/movie".into())]),
        ..Default::default()
    };
    let mut candidate = Metadata {
        code: before.code.clone(),
        ..Default::default()
    };
    candidate.fields.title = Some("new".into());
    candidate.provenance.insert("title".into(), "javdb".into());
    assert!(
        !merge_selected(&before, &candidate, &["title".into()], true)
            .unwrap()
            .source_urls
            .contains_key("javdb")
    );
}
