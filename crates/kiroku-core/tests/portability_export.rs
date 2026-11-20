use kiroku_core::{
    actor_hosting,
    library::Library,
    portability::{self, FileKind, Manifest, MANIFEST_NAME},
    Metadata,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _tmp: tempfile::TempDir,
    library: Library,
    movie: PathBuf,
    exports: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("library");
        let movie = root.join("ABC-123");
        let exports = tmp.path().join("copies");
        fs::create_dir_all(&movie).unwrap();
        fs::create_dir(&exports).unwrap();
        let mut meta = Metadata {
            code: "ABC-123".to_string(),
            asset_stem: Some("ABC-123".into()),
            favorite: true,
            ..Default::default()
        };
        meta.fields.title = Some("测试影片".into());
        meta.fields.tags = vec!["测试标签".into()];
        meta.fields.actors = vec![kiroku_core::metadata::Actor::new(
            "测试演员",
            kiroku_core::metadata::Gender::Unknown,
        )];
        fs::create_dir(movie.join(".actors")).unwrap();
        fs::write(movie.join(".actors/测试演员.png"), b"avatar").unwrap();
        fs::write(
            movie.join("kiroku.json"),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        fs::write(
            movie.join("ABC-123.nfo"),
            b"<movie><num>ABC-123</num><custom>preserved</custom></movie>",
        )
        .unwrap();
        fs::write(movie.join("ABC-123-poster.jpg"), b"poster").unwrap();
        fs::write(movie.join("ABC-123-fanart.png"), b"fanart").unwrap();
        fs::write(movie.join("ABC-123.mp4"), vec![7; 200_000]).unwrap();
        fs::write(movie.join("ABC-123.zh.srt"), b"subtitle").unwrap();
        let library = Library::open(&root).unwrap();
        library.reindex_dir(&movie).unwrap();
        Self {
            _tmp: tmp,
            library,
            movie,
            exports,
        }
    }
    fn preview(
        &self,
        kinds: &[FileKind],
        directory: &Path,
        name: &str,
    ) -> std::io::Result<portability::ExportPlan> {
        portability::preview(
            &self.library.reader_handle().open().unwrap(),
            &["ABC-123".into()],
            kinds,
            directory,
            name,
            &actor_hosting::Config::default(),
            &CancellationToken::new(),
        )
    }
}

#[test]
fn copies_selected_assets_without_renaming_originals_and_manifest_authenticates_every_file() {
    let f = Fixture::new();
    let kinds = [
        FileKind::Nfo,
        FileKind::Sidecar,
        FileKind::Poster,
        FileKind::Fanart,
        FileKind::Video,
        FileKind::Subtitle,
        FileKind::Avatar,
    ];
    let plan = f.preview(&kinds, &f.exports, "kiroku-test").unwrap();
    assert_eq!(plan.preview.files, 7);
    assert!(!plan.preview.destination.exists());
    let result = portability::export(plan, &CancellationToken::new(), |_, _| {}).unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(result.destination.join(MANIFEST_NAME)).unwrap()).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.movies[0].code, "ABC-123");
    let mut count = 0;
    for file in &manifest.movies[0].files {
        assert!(!Path::new(&file.path).is_absolute());
        let data = fs::read(result.destination.join(&file.path)).unwrap();
        assert_eq!(file.sha256, format!("{:x}", Sha256::digest(&data)));
        assert_eq!(file.bytes, data.len() as u64);
        if file.kind == FileKind::Avatar {
            assert_eq!(file.actor.as_deref(), Some("测试演员"));
            assert!(file.path.ends_with("/.actors/测试演员.png"));
            let movie = result.destination.join("movies/00001-ABC-123");
            assert_eq!(
                kiroku_core::supplement::local_avatar_path(&movie, "测试演员").unwrap(),
                Some(movie.join(".actors/测试演员.png"))
            );
        }
        if file.kind == FileKind::Sidecar {
            assert_eq!(data, fs::read(f.movie.join("kiroku.json")).unwrap());
        }
        count += file.bytes;
    }
    assert_eq!(count, result.bytes);
    assert!(f.movie.join("ABC-123.mp4").is_file());
}

#[test]
fn rejects_destination_collisions_changed_sources_and_cleans_a_cancelled_partial_copy() {
    let f = Fixture::new();
    let kinds = [FileKind::Nfo, FileKind::Video];
    assert!(f
        .preview(&kinds, &f.movie, "kiroku-inside-library")
        .is_err());
    let plan = f.preview(&kinds, &f.exports, "kiroku-race").unwrap();
    let folder = plan.preview.destination.clone();
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("owned-by-user"), b"x").unwrap();
    assert!(portability::export(plan, &CancellationToken::new(), |_, _| {}).is_err());
    assert_eq!(fs::read(folder.join("owned-by-user")).unwrap(), b"x");

    let plan = f.preview(&kinds, &f.exports, "kiroku-changed").unwrap();
    fs::write(f.movie.join("ABC-123.mp4"), b"replacement").unwrap();
    assert!(portability::export(plan, &CancellationToken::new(), |_, _| {}).is_err());
    assert!(!f.exports.join("kiroku-changed").exists());
    let token = CancellationToken::new();
    let plan = f.preview(&kinds, &f.exports, "kiroku-cancelled").unwrap();
    let result = portability::export(plan, &token, |_, _| token.cancel());
    assert!(result.is_err());
    assert!(!f.exports.join("kiroku-cancelled").exists());
}

#[cfg(unix)]
#[test]
fn external_image_symlink_is_never_exported() {
    let f = Fixture::new();
    let image = f.movie.join("ABC-123-poster.jpg");
    fs::remove_file(&image).unwrap();
    let foreign = f.exports.join("private");
    fs::write(&foreign, b"secret").unwrap();
    std::os::unix::fs::symlink(foreign, &image).unwrap();
    assert!(f
        .preview(&[FileKind::Poster], &f.exports, "kiroku-links")
        .is_err());
}

#[test]
fn normalized_avatar_collisions_keep_distinct_manifest_links() {
    use kiroku_core::metadata::{Actor, Gender};
    let f = Fixture::new();
    let path = f.movie.join("kiroku.json");
    let mut meta: Metadata = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta.fields.actors = vec![
        Actor::new("Test/One", Gender::Unknown),
        Actor::new("Test:One", Gender::Unknown),
    ];
    fs::write(f.movie.join(".actors/Test One.webp"), b"synthetic avatar").unwrap();
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    let plan = f
        .preview(&[FileKind::Avatar], &f.exports, "kiroku-names")
        .unwrap();
    let exported = portability::export(plan, &CancellationToken::new(), |_, _| {}).unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(exported.destination.join(MANIFEST_NAME)).unwrap())
            .unwrap();
    let avatars = &manifest.movies[0].files;
    assert_eq!(avatars.len(), 2);
    assert_eq!(avatars[0].actor.as_deref(), Some("Test/One"));
    assert_eq!(avatars[1].actor.as_deref(), Some("Test:One"));
    assert!(avatars[0].path.ends_with("/.actors/Test One.webp"));
    assert!(avatars[1]
        .path
        .ends_with("/.actors/kiroku-actor-00002.webp"));
    assert_eq!(avatars[0].sha256, avatars[1].sha256);
}
