//! Offline package round trips between two unrelated library roots.
use kiroku_core::{
    actor_hosting,
    library::{CsvColumn, CsvOptions, Library, Query},
    metadata::{Actor, Gender},
    package_restore::{self, Package, Rule, Rules},
    portability::{self, FileKind, MANIFEST_NAME},
    revision,
    settings::Settings,
    Metadata,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

const CODE: &str = "ABC-123";
const NAME: &str = "演员甲";
const ORIGIN: &str = "https://example.org/person.png";

fn png(color: u8) -> Vec<u8> {
    let mut buf = Cursor::new(vec![]);
    image::RgbImage::from_pixel(2, 2, image::Rgb([color, 20, 30]))
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

struct Fixture {
    _temp: tempfile::TempDir,
    package: PathBuf,
    root: PathBuf,
    dest: PathBuf,
    settings: Settings,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let area = temp.path().canonicalize().unwrap();
        let source = area.join("source");
        let src = source.join("first-location");
        let root = area.join("destination");
        let dest = root.join("different-location");
        let exports = area.join("external");
        let hosted = area.join("local-avatar-cache");
        for dir in [&src, &dest, &exports, &hosted] {
            fs::create_dir_all(dir).unwrap();
        }
        let mut src_meta = Metadata {
            code: CODE.into(),
            asset_stem: Some("original-stem".into()),
            favorite: true,
            ..Default::default()
        };
        src_meta.fields.title = Some("资料包标题".into());
        src_meta.text_versions.original.title = Some("原始语言片名".into());
        src_meta.fields.tags = vec!["资料标签".into()];
        src_meta.fields.actors = vec![Actor::new(NAME, Gender::Unknown).with_thumb(ORIGIN)];
        src_meta.locked_fields.insert("tags".into());
        fs::write(
            src.join("kiroku.json"),
            serde_json::to_vec(&src_meta).unwrap(),
        )
        .unwrap();
        fs::write(
            src.join("original-stem.nfo"),
            kiroku_core::nfo::to_nfo(&src_meta)
                .replace("</movie>", "<studio>NFO 补充片商</studio></movie>"),
        )
        .unwrap();
        fs::write(src.join("original-stem.mp4"), b"source-video").unwrap();
        fs::write(src.join("original-stem-poster.png"), png(80)).unwrap();
        fs::create_dir(src.join(".actors")).unwrap();
        fs::write(src.join(".actors").join(format!("{NAME}.png")), png(65)).unwrap();
        let lib = Library::open(&source).unwrap();
        lib.reindex_dir(&src).unwrap();
        let cancel = CancellationToken::new();
        let plan = portability::preview(
            &lib.reader_handle().open().unwrap(),
            &[CODE.into()],
            &[
                FileKind::Nfo,
                FileKind::Sidecar,
                FileKind::Poster,
                FileKind::Avatar,
                FileKind::Video,
            ],
            &exports,
            "kiroku-test",
            &Default::default(),
            &cancel,
        )
        .unwrap();
        let package = portability::export(plan, &cancel, |_, _| {})
            .unwrap()
            .destination;
        let mut local = Metadata {
            code: CODE.into(),
            asset_stem: Some("local-stem".into()),
            ..Default::default()
        };
        local.fields.title = Some("本地标题".into());
        local.locked_fields.insert("title".into());
        let mut json = serde_json::to_value(&local).unwrap();
        json["user_data"] = serde_json::json!({"retain": 123});
        fs::write(dest.join("kiroku.json"), serde_json::to_vec(&json).unwrap()).unwrap();
        fs::write(
            dest.join("local-stem.nfo"),
            "<movie><num>ABC-123</num><title>本地标题</title><custom>保留扩展</custom></movie>",
        )
        .unwrap();
        fs::write(dest.join("local-stem.mp4"), b"original-destination-video").unwrap();
        fs::write(dest.join("local-stem-poster.png"), png(20)).unwrap();
        let settings = Settings {
            actor_hosting: actor_hosting::Config {
                enabled: true,
                directory: hosted.to_str().unwrap().into(),
                base_url: "https://example.org/new-host/".into(),
            },
            ..Default::default()
        };
        Self {
            _temp: temp,
            package,
            root,
            dest,
            settings,
        }
    }

    fn prepare(&self, rules: &Rules) -> std::io::Result<package_restore::Candidate> {
        let package = Package::open(&self.package)?;
        package_restore::prepare(
            &package,
            &package.manifest.movies[0],
            &self.root,
            &self.dest,
            rules,
            &self.settings,
            &CancellationToken::new(),
        )
    }

    fn read(&self, name: &str) -> Vec<u8> {
        fs::read(self.dest.join(name)).unwrap()
    }
}

#[test]
fn roundtrip_rebinds_paths_preserves_locked_fields_and_can_be_undone() {
    let f = Fixture::new();
    let before_side = f.read("kiroku.json");
    let before_nfo = f.read("local-stem.nfo");
    let candidate = f.prepare(&Rules::default()).unwrap();
    let signature = candidate.signature().unwrap();
    assert_eq!(
        f.prepare(&Rules::default()).unwrap().signature().unwrap(),
        signature
    );
    let preview = candidate.preview.unwrap();
    assert_eq!(preview.after.fields.title.as_deref(), Some("本地标题"));
    assert!(preview.after.locked_fields.contains("tags") && preview.after.favorite);
    assert_eq!(preview.after.fields.studio.as_deref(), Some("NFO 补充片商"));
    // The sidecar keeps the original-language title separately. Its own NFO
    // projection must not appear as an additional external field on restore.
    assert!(preview.after.fields.original_title.is_none());
    assert_eq!(preview.after.text_versions.original.title.as_deref(), Some("原始语言片名"));
    assert_eq!(preview.after.asset_stem.as_deref(), Some("local-stem"));
    assert!(preview.after.hosted_actor_images[ORIGIN]
        .url
        .starts_with("https://example.org/new-host/"));
    assert!(candidate
        .files
        .iter()
        .any(|p| p.ends_with(".actors/演员甲.png")));
    assert!(!candidate.files.iter().any(|p| p.ends_with("-poster.png")));
    let mut result = revision::apply(&preview).unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(String::from_utf8(f.read("local-stem.nfo"))
        .unwrap()
        .contains("<custom>保留扩展</custom>"));
    assert!(String::from_utf8(f.read("local-stem.nfo"))
        .unwrap()
        .contains("资料标签"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&f.read("kiroku.json")).unwrap()["user_data"]
            ["retain"],
        123
    );
    assert!(f.prepare(&Rules::default()).unwrap().preview.is_none());
    assert_eq!(f.read(".actors/演员甲.png"), png(65));
    assert_eq!(f.read("local-stem-poster.png"), png(20));
    assert!(result.journal.rollback().is_empty());
    assert_eq!(before_side, f.read("kiroku.json"));
    assert_eq!(before_nfo, f.read("local-stem.nfo"));
    assert!(!f.dest.join(".actors/演员甲.png").exists());
    assert_eq!(f.read("local-stem.mp4"), b"original-destination-video");
}

#[test]
fn raw_original_title_change_stays_visible_when_both_nfo_projections_match() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let mut manifest = Package::open(&f.package).unwrap().manifest;
    let side = manifest.movies[0].files.iter_mut()
        .find(|file| file.kind == FileKind::Sidecar).unwrap();
    let side_path = f.package.join(&side.path);
    let mut source: Metadata = serde_json::from_slice(&fs::read(&side_path).unwrap()).unwrap();
    source.fields.original_title = source.text_versions.original.title.clone();
    let bytes = serde_json::to_vec(&source).unwrap();
    side.sha256 = format!("{:x}", Sha256::digest(&bytes));
    side.bytes = bytes.len() as u64;
    fs::write(side_path, bytes).unwrap();
    fs::write(f.package.join(MANIFEST_NAME), serde_json::to_vec(&manifest).unwrap()).unwrap();

    let mut local: Metadata = serde_json::from_slice(&f.read("kiroku.json")).unwrap();
    local.text_versions.original.title = source.fields.original_title;
    fs::write(f.dest.join("kiroku.json"), serde_json::to_vec(&local).unwrap()).unwrap();
    let preview = f.prepare(&Rules::default()).unwrap().preview.unwrap();
    let changes: Vec<_> = preview.changes.iter().filter(|c| c.field == "original_title").collect();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].before.is_null());
    assert_eq!(changes[0].after.as_str(), Some("原始语言片名"));
}

#[test]
fn replace_honors_destination_extensions_and_external_changes() {
    let f = Fixture::new();
    let rules = Rules {
        title: Rule::Replace,
        poster: Rule::Replace,
        ..Rules::default()
    };
    let candidate = f.prepare(&rules).unwrap();
    let first = candidate.signature();
    assert_eq!(
        candidate
            .preview
            .as_ref()
            .unwrap()
            .after
            .fields
            .title
            .as_deref(),
        Some("资料包标题")
    );
    fs::write(f.dest.join("local-stem-poster.png"), png(4)).unwrap();
    assert!(revision::apply(&candidate.preview.unwrap()).is_err());
    let candidate = f.prepare(&rules).unwrap();
    assert_ne!(candidate.signature(), first);
    let mut result = revision::apply(&candidate.preview.unwrap()).unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(f.read("local-stem-poster.png"), png(80));
    assert!(!f.dest.join("local-stem-poster.jpg").exists());
    assert!(result.journal.rollback().is_empty());
    assert_eq!(f.read("local-stem-poster.png"), png(4));
}

#[test]
fn corrupted_and_outside_package_inputs_cannot_be_applied() {
    let f = Fixture::new();
    let package = Package::open(&f.package).unwrap();
    let side = package.manifest.movies[0]
        .files
        .iter()
        .find(|f| f.kind == FileKind::Sidecar)
        .unwrap();
    let path = f.package.join(&side.path);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] = b'[';
    fs::write(&path, bytes).unwrap();
    assert!(f.prepare(&Rules::default()).is_err());
    let mut manifest = package.manifest;
    manifest.movies[0].files[0].path = "../elsewhere".into();
    fs::write(
        f.package.join(MANIFEST_NAME),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(Package::open(&f.package).is_err());
    assert_eq!(f.read("local-stem.mp4"), b"original-destination-video");
}

#[test]
fn csv_selection_uses_requested_order_tags_and_current_paths() {
    let f = Fixture::new();
    let p = f.prepare(&Rules::default()).unwrap().preview.unwrap();
    assert!(revision::apply(&p).unwrap().error.is_none());
    let lib = Library::open(&f.root).unwrap();
    lib.reindex_dir(&f.dest).unwrap();
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(
        other.join("movie.nfo"),
        "<movie><num>DEF-456</num><title>另一部</title></movie>",
    )
    .unwrap();
    fs::write(other.join("DEF-456.mkv"), b"video").unwrap();
    lib.reindex_dir(&other).unwrap();
    let options = CsvOptions {
        columns: vec![CsvColumn::Paths, CsvColumn::Tags, CsvColumn::Code],
        codes: Some(vec![CODE.into()]),
    };
    let mut bytes = Vec::new();
    let count = lib
        .export_csv_with(
            &Query::default(),
            &options,
            &mut bytes,
            &CancellationToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    let csv = String::from_utf8(bytes).unwrap();
    assert_eq!(count, 1);
    assert!(csv.starts_with("\u{feff}文件路径,标签,编号\r\n"));
    assert!(csv.contains(Path::new(&f.dest).join("local-stem.mp4").to_str().unwrap()));
    assert!(csv.contains("\"资料标签\",\"ABC-123\""));
    assert!(!csv.contains("DEF-456"));
}
