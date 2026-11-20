use kiroku_core::{
    artwork, badge::Phase, library::Library, nfo, revision, settings::Settings, Metadata,
};
use std::{fs, path::Path};

const CODE: &str = "TEST-105";
const MOVIE: &str = "Example/TEST-105";
const VIDEO: &str = "TEST-105-C.mp4";
const POSTER: &str = "TEST-105-C-poster.jpg";

pub fn seed(temporary: &Path, lib: &Library, settings: &Settings) {
    let root = lib.root();
    let dir = root.join(MOVIE);
    fs::create_dir_all(&dir).unwrap();
    let phase = Phase {
        subtitle: true,
        ..Phase::default()
    };
    let mut meta = Metadata {
        code: CODE.into(),
        asset_stem: Some("TEST-105-C".into()),
        artwork_phase: Some(phase.clone()),
        ..Metadata::default()
    };
    meta.fields.title = Some("Fixture".into());
    fs::write(dir.join(VIDEO), b"synthetic video").unwrap();
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    fs::write(dir.join("TEST-105-C.nfo"), nfo::to_nfo(&meta)).unwrap();
    let capture = revision::capture(root, &dir, CODE).unwrap();
    let original = artwork::Original::from_bytes(
        capture,
        include_bytes!("../../icons/128x128.png").to_vec(),
        None,
        phase.clone(),
    )
    .unwrap();
    fs::write(dir.join(&original.artwork.original), &original.bytes).unwrap();
    let stamp = settings.badges.stamping(&phase, None).unwrap();
    let initial = artwork::crop(
        &original.bytes,
        artwork::CropBox {
            x: 0,
            y: 0,
            width: 90,
            height: 128,
        },
        Some(&stamp),
    )
    .unwrap();
    meta.artwork = Some(original.artwork);
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    fs::write(dir.join("TEST-105-C.nfo"), nfo::to_nfo(&meta)).unwrap();
    fs::write(dir.join(POSTER), &initial).unwrap();
    fs::write(dir.join("poster.jpg"), &initial).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let version = lib.get(CODE).unwrap().unwrap().card.poster_version.unwrap();
    let cache = Settings::default_path()
        .parent()
        .unwrap()
        .join("cache/posters-v1");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join(format!("{version}.jpg")), &initial).unwrap();
    fs::write(temporary.join("initial-poster"), &initial).unwrap();
    let source = Path::new(&settings.source_dirs[0]).join(MOVIE);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::rename(dir, source).unwrap();
}

// Test-only filesystem observations keep trusted paths outside the JavaScript bridge.
#[tauri::command]
pub fn release_linked_files() -> Result<serde_json::Value, String> {
    if std::env::args().nth(1).as_deref() != Some("--linked-crop") {
        return Err("linked crop fixture is not active".into());
    }
    let temporary = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
    let movie = temporary.join("source").join(MOVIE);
    let read = |p| fs::read(p).map_err(|err| err.to_string());
    let named = read(movie.join(POSTER))?;
    let alias = read(movie.join("poster.jpg"))?;
    let meta: Metadata =
        serde_json::from_slice(&read(movie.join("kiroku.json"))?).map_err(|err| err.to_string())?;
    let artwork = meta.artwork.ok_or("missing original reference")?;
    Ok(serde_json::json!({
        "initial": named == read(temporary.join("initial-poster"))?,
        "same": named == alias,
        "dimensions": artwork::dimensions(&named).map_err(|err| err.to_string())?,
        "crop": artwork.crop,
        "videoOk": read(movie.join(VIDEO))? == b"synthetic video",
        "originalOk": read(movie.join(artwork.original))? ==
            artwork::normalize(include_bytes!("../../icons/128x128.png")).map_err(|err| err.to_string())?,
        "oldDirectoryAbsent": !temporary.join("library").join(MOVIE).exists(),
    }))
}

#[derive(Default)]
pub struct ScanFixture(std::sync::Mutex<Option<crate::state::JobGuard>>);

// Hold the real job slot until the test has observed the editor's waiting state.
#[tauri::command]
pub fn release_scan_fixture(app: tauri::AppHandle, hold: bool) -> Result<(), String> {
    use tauri::Manager;
    if std::env::args().nth(1).as_deref() != Some("--linked-crop") {
        return Err("scan fixture is not active".into());
    }
    let fixture = app.state::<ScanFixture>();
    let mut guard = fixture.0.lock().map_err(|e| e.to_string())?;
    if hold {
        let (id, _) = app
            .state::<crate::state::AppState>()
            .begin_job("pending-scan")?;
        *guard = Some(crate::state::JobGuard {
            app: app.clone(),
            id,
        });
    } else {
        guard.take().ok_or("scan fixture did not claim the slot")?;
    }
    Ok(())
}
