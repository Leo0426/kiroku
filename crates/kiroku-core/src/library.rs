//! 媒体库索引:SQLite,位于 `<库根>/.kiroku/library.db`。
//!
//! 磁盘是唯一真相(`kiroku.json` / 视频同名 NFO(兼容 movie.nfo) + 视频文件),索引只服务于浏览和搜索,
//! 随时可以删掉重建。所以写入路径只有一条:`reindex_dir` 读目录 → 更新索引;整理、回滚、重建都走它。

use crate::{
    code::{parse_file_info, parse_text},
    metadata::{Metadata, SourcedScore},
    nfo_read,
    organize::SIDECAR,
    pipeline::VIDEO_EXT,
};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("数据库: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("文件系统: {0}")]
    Io(#[from] io::Error),
}
pub type Result<T> = std::result::Result<T, LibraryError>;

const SCHEMA: &str = "
CREATE TABLE movie(
  code TEXT PRIMARY KEY,
  title TEXT, original_title TEXT, year INTEGER, release_date TEXT,
  studio TEXT, series TEXT,
  actors TEXT NOT NULL DEFAULT '[]',
  search TEXT NOT NULL DEFAULT '',
  runtime_min INTEGER, score REAL, content_type TEXT,
  dir TEXT NOT NULL,       -- 相对库根,正斜杠
  poster TEXT,             -- 相对库根;无海报为 NULL
  poster_version TEXT, fanart_version TEXT,
  added_at INTEGER NOT NULL,
  meta_json TEXT NOT NULL
);
CREATE TABLE file(
  id INTEGER PRIMARY KEY,
  code TEXT NOT NULL REFERENCES movie(code) ON DELETE CASCADE,
  rel_path TEXT NOT NULL, part INTEGER, size INTEGER NOT NULL DEFAULT 0,
  UNIQUE(code, rel_path)
);
CREATE INDEX movie_added ON movie(added_at DESC);
CREATE INDEX movie_year ON movie(year);
CREATE INDEX movie_type ON movie(content_type);
CREATE INDEX movie_dir ON movie(dir);
";

/// v4:筛选维度子表,随影片行删除。
const FACET_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS movie_facet(
  code TEXT NOT NULL REFERENCES movie(code) ON DELETE CASCADE,
  kind TEXT NOT NULL, value TEXT NOT NULL,
  PRIMARY KEY(code, kind, value)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS movie_facet_value ON movie_facet(kind, value, code);
";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    /// 最近入库在前
    #[default]
    Added,
    Title,
    Year,
    Score,
}

/// 海报墙可筛选的维度;值来自 sidecar(角标来自整理时的相位与视频文件名)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FacetKind { Tag, Actor, Studio, Series, Badge, Favorite }

impl FacetKind {
    fn as_str(self) -> &'static str {
        match self { Self::Tag => "tag", Self::Actor => "actor", Self::Studio => "studio", Self::Series => "series", Self::Badge => "badge", Self::Favorite => "favorite" }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetFilter { pub kind: FacetKind, pub value: String }

/// 每个维度在 `facets()` 里最多返回的取值数(按影片数降序)。
const FACET_VALUES: usize = 300;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Query {
    /// 空格分隔的多个词,全部命中(番号/标题/演员/片商/系列)
    pub text: Option<String>,
    pub content_type: Option<String>,
    pub year: Option<i32>,
    pub sort: Sort,
    pub offset: usize,
    pub limit: usize,
    /// 维度筛选,全部满足(AND)
    pub filters: Vec<FacetFilter>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieCard {
    pub code: String,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub actors: Vec<String>,
    /// 海报绝对路径
    pub poster: Option<String>,
    pub poster_version: Option<String>,
    pub content_type: Option<String>,
    pub score: Option<f32>,
    pub added_at: i64,
    pub parts: usize,
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieFile {
    pub path: String,
    pub part: Option<u8>,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieDetail {
    #[serde(flatten)]
    pub card: MovieCard,
    pub meta: Metadata,
    pub dir: String,
    pub files: Vec<MovieFile>,
    /// 横版封面(fanart.jpg)绝对路径,存在时作详情页背景
    pub fanart: Option<String>,
    pub fanart_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub total: usize,
    pub items: Vec<MovieCard>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub total: usize,
    pub years: Vec<(i32, usize)>,
    pub content_types: Vec<(String, usize)>,
    pub tags: Vec<(String, usize)>,
    pub actors: Vec<(String, usize)>,
    pub studios: Vec<(String, usize)>,
    pub series: Vec<(String, usize)>,
    pub badges: Vec<(String, usize)>,
    pub favorites: usize,
}

/// 由元数据和视频文件名派生的筛选维度。只在写索引时调用,是可重建的缓存。
fn facets_of<'a>(m: &Metadata, files: impl IntoIterator<Item = &'a str>) -> std::collections::BTreeSet<(FacetKind, String)> {
    let f = &m.fields;
    let clean = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    let mut out = std::collections::BTreeSet::new();
    out.extend(f.actors.iter().filter_map(|a| clean(&a.name)).map(|v| (FacetKind::Actor, v)));
    out.extend(f.tags.iter().filter_map(|t| clean(t)).map(|v| (FacetKind::Tag, v)));
    out.extend(f.studio.as_deref().and_then(clean).map(|v| (FacetKind::Studio, v)));
    out.extend(f.series.as_deref().and_then(clean).map(|v| (FacetKind::Series, v)));
    let mut phase = m.artwork_phase.clone().unwrap_or_default();
    for name in files {
        let name = name.rsplit('/').next().unwrap_or(name);
        if let Some(code) = crate::code::parse_code(name) { phase.merge(&crate::badge::Phase::of(&code)) }
    }
    out.extend(phase.stems().into_iter().map(|(_, stem)| (FacetKind::Badge, stem)));
    if m.favorite { out.insert((FacetKind::Favorite, "yes".into())); }
    out
}

pub struct Library {
    root_identity: Arc<same_file::Handle>,
    db_identity: Arc<same_file::Handle>,
    root: PathBuf,
    conn: Connection,
    read_cache: Arc<query::ReadCache>,
}

mod reader;
mod query;
mod rebuild;
pub use reader::{LibraryReadHandle, LibraryReader};

fn rel_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn like_pattern(word: &str) -> String {
    let esc = word.to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{esc}%")
}

fn optional_metadata_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io::Error::new(e.kind(), format!("{}: {e}",path.display()))),
    }
}

impl Library {
    pub fn db_path(root: &Path) -> PathBuf {
        crate::journal::dir(root).join("library.db")
    }

    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(crate::journal::dir(root))?;
        let root_identity = Arc::new(same_file::Handle::from_path(root)?);
        let path = Self::db_path(root);
        let file = match fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&path)?.is_file() {
                    return Err(io::Error::other("索引文件不是普通文件，请重新打开媒体库").into());
                }
                fs::File::open(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        let binding = LibraryReadHandle { root: root.to_path_buf(), root_identity, read_cache: Arc::new(query::ReadCache::default()),
            db_identity: Arc::new(same_file::Handle::from_file(file)?) };
        Self::open_bound(binding)
    }

    fn open_bound(binding: LibraryReadHandle) -> Result<Self> {
        binding.verify()?;
        let conn = Connection::open_with_flags(Self::db_path(&binding.root),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        binding.verify()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            conn.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version=2; COMMIT;"))?;
        } else if v == 1 {
            conn.execute_batch("BEGIN; ALTER TABLE movie ADD COLUMN poster_version TEXT; ALTER TABLE movie ADD COLUMN fanart_version TEXT; PRAGMA user_version=2; COMMIT;")?;
        }
        if v < 3 {
            conn.execute_batch(&format!("BEGIN; {} PRAGMA user_version=3; COMMIT;", query::MIGRATION))?;
        }
        if v < 4 {
            // 从已缓存的行回填维度,不扫描磁盘。
            let transaction = conn.unchecked_transaction()?;
            transaction.execute_batch(FACET_SCHEMA)?;
            let rows = transaction.prepare("SELECT code, meta_json FROM movie")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (code, meta_json) in rows {
                let meta: Metadata = serde_json::from_str(&meta_json).unwrap_or_default();
                let files = transaction.prepare("SELECT rel_path FROM file WHERE code = ?1")?
                    .query_map(params![code], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for (kind, value) in facets_of(&meta, files.iter().map(String::as_str)) {
                    transaction.execute("INSERT OR IGNORE INTO movie_facet(code,kind,value) VALUES(?1,?2,?3)", params![code, kind.as_str(), value])?;
                }
            }
            transaction.execute_batch("PRAGMA user_version=4;")?;
            transaction.commit()?;
        }
        let library = Self { root_identity: binding.root_identity, db_identity: binding.db_identity, root: binding.root, conn, read_cache: binding.read_cache };
        library.verify_binding()?;
        Ok(library)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Cheap clone: opening a query connection happens later, off the UI thread.
    pub fn reader_handle(&self) -> LibraryReadHandle {
        LibraryReadHandle { root: self.root.clone(), root_identity: self.root_identity.clone(), db_identity: self.db_identity.clone(), read_cache: self.read_cache.clone() }
    }
    fn verify_binding(&self) -> io::Result<()> { self.reader_handle().verify() }

    fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    // ---- 写入:唯一入口是 reindex_dir ------------------------------------------------

    /// 读取一个影片目录并更新索引;目录不存在、没有元数据或没有视频文件时移除该目录的索引。
    /// 返回入库的番号。
    pub fn reindex_dir(&self,dir:&Path)->Result<Option<String>>{self.reindex_dir_cancellable(dir,&CancellationToken::new())}
    pub fn reindex_dir_cancellable(&self,dir:&Path,cancel:&CancellationToken)->Result<Option<String>> {
        crate::append::check_cancel(cancel)?;
        self.verify_binding()?;
        // Publish metadata and the file set atomically for independent readers.
        // Rebuild already owns an outer transaction and must not nest BEGIN.
        let transaction = if self.conn.is_autocommit() { Some(self.conn.unchecked_transaction()?) } else { None };
        let Ok(rel) = dir.strip_prefix(&self.root) else { return Ok(None) };
        let rel = rel_str(rel);
        crate::append::confined_path(&self.root,dir)?;
        let mut links=vec![];
        let (mut videos, mut sidecar, mut nfo) = (vec![], None::<String>, None::<String>);
        let mut nfo_path = None;
        let rd = match fs::read_dir(dir) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(rd) = rd {
            for entry in rd {
                let e = entry?;
                let name = e.file_name().to_string_lossy().into_owned();
                let ext = Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                if name == SIDECAR {
                    sidecar = optional_metadata_text(&e.path())?;
                } else if ext == "nfo" && (nfo.is_none() || name == "movie.nfo") {
                    nfo = optional_metadata_text(&e.path())?;
                    nfo_path = Some(e.path());
                } else if VIDEO_EXT.contains(&ext.as_str()) && !name.starts_with("._") {
                    if e.file_type()?.is_symlink(){links.push(e.path());continue;}
                    match e.metadata() {
                        Ok(metadata) => videos.push((name, metadata.len())),
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {},
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = sidecar
            .and_then(|s| serde_json::from_str::<Metadata>(&s).ok())
            .or_else(|| nfo.and_then(|x| nfo_read::parse_nfo(&x, &dir_name)).map(|mut meta| {
                if let Some(path) = &nfo_path { crate::assets::infer_from_nfo(&mut meta, path); }
                meta
            }));
        if let Some(meta)=&meta {for link in &links {let resolved=crate::placement::resolve_media_cancellable(&self.root,dir,link,meta,cancel)?;videos.push((link.file_name().unwrap().to_string_lossy().into_owned(),fs::metadata(resolved)?.len()));}}
        else if !links.is_empty(){return Err(io::Error::other("未登记媒体链接").into())}
        let Some(meta) = meta.filter(|_| !videos.is_empty()) else {
            self.conn.execute("DELETE FROM movie WHERE dir = ?1", params![rel])?;
            self.verify_binding()?;
            crate::append::check_cancel(cancel)?;
            if let Some(transaction) = transaction { transaction.commit()?; }
            return Ok(None);
        };
        self.upsert(&meta, &rel, dir, &videos, cancel)?;
        self.verify_binding()?;
        crate::append::check_cancel(cancel)?;
        if let Some(transaction) = transaction { transaction.commit()?; }
        Ok(Some(meta.code))
    }

    fn upsert(&self, m: &Metadata, rel_dir: &str, dir: &Path, videos: &[(String, u64)], cancel: &CancellationToken) -> Result<()> {
        let f = &m.fields;
        let actors: Vec<&str> = f.actors.iter().map(|a| a.name.as_str()).collect();
        let search = [
            Some(m.code.as_str()), f.title.as_deref(), f.original_title.as_deref(), f.studio.as_deref(), f.series.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(actors.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
        let year: Option<i32> = m.year().and_then(|y| y.parse().ok());
        let ct = serde_json::to_value(parse_text(&m.code).content_type).ok().and_then(|v| v.as_str().map(String::from));
        let assets = crate::assets::Paths::for_movie(dir, m)?;
        let poster = assets.poster.exists().then(|| rel_str(assets.poster.strip_prefix(&self.root).expect("movie within library")));
        let poster_version = self.image_version(&assets.poster, cancel)?;
        let fanart_version = self.image_version(&assets.fanart, cancel)?;
        crate::append::check_cancel(cancel)?;
        self.verify_binding()?;
        let added = fs::metadata(dir).and_then(|md| md.modified()).ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        let score = m.mean_score();
        self.conn.execute(
            "INSERT INTO movie(code,title,original_title,year,release_date,studio,series,actors,search,runtime_min,score,content_type,dir,poster,added_at,meta_json,poster_version,fanart_version)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(code) DO UPDATE SET title=?2,original_title=?3,year=?4,release_date=?5,studio=?6,series=?7,actors=?8,search=?9,
               runtime_min=?10,score=?11,content_type=?12,dir=?13,poster=?14,meta_json=?16,poster_version=?17,fanart_version=?18",
            params![
                m.code, f.title, f.original_title, year, f.release_date, f.studio, f.series,
                serde_json::to_string(&actors).unwrap_or_else(|_| "[]".into()), search, f.runtime_min, score, ct,
                rel_dir, poster, added, serde_json::to_string(m).unwrap_or_default(), poster_version, fanart_version
            ],
        )?;
        self.conn.execute("DELETE FROM file WHERE code = ?1", params![m.code])?;
        for (name, size) in videos {
            let part = parse_file_info(name).cd;
            self.conn.execute(
                "INSERT OR REPLACE INTO file(code,rel_path,part,size) VALUES(?1,?2,?3,?4)",
                params![m.code, format!("{rel_dir}/{name}"), part, *size as i64],
            )?;
        }
        self.conn.execute("DELETE FROM movie_facet WHERE code = ?1", params![m.code])?;
        for (kind, value) in facets_of(m, videos.iter().map(|(name, _)| name.as_str())) {
            self.conn.execute("INSERT INTO movie_facet(code,kind,value) VALUES(?1,?2,?3)", params![m.code, kind.as_str(), value])?;
        }
        Ok(())
    }

    fn image_version(&self, path: &Path, cancel: &CancellationToken) -> Result<Option<String>> {
        use std::io::Read;
        use sha2::{Digest, Sha256};
        let result = (|| -> io::Result<Option<String>> {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            if !metadata.is_file() { return Ok(None); }
            crate::append::confined_path(&self.root, path)?;
            let mut file = fs::File::open(path)?;
            let mut digest = Sha256::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                crate::append::check_cancel(cancel)?;
                let n = file.read(&mut buffer)?;
                if n == 0 { break; }
                digest.update(&buffer[..n]);
            }
            Ok(Some(format!("{:x}", digest.finalize())))
        })();
        match result {
            Ok(version) => Ok(version),
            Err(error) if cancel.is_cancelled() => Err(error.into()),
            Err(error) => { log::warn!(target: "kiroku::library", "图片版本不可用 {}: {error}", path.display()); Ok(None) }
        }
    }

    /// 清空并从磁盘重建。`on_progress(done, total)`;取消或失败保留原索引。
    pub fn rebuild(&self, cancel: &CancellationToken, on_progress: &mut dyn FnMut(usize, usize)) -> Result<usize> {
        self.verify_binding()?;
        if cancel.is_cancelled() { return Ok(0); }
        let directories = rebuild::Directories::capture(&self.root,cancel)?;
        if cancel.is_cancelled() { return Ok(0); }
        let total = directories.len()?;
        self.verify_binding()?;
        let transaction = self.conn.unchecked_transaction()?;
        self.conn.execute_batch("DELETE FROM file; DELETE FROM movie;")?;
        let mut run = || -> Result<usize> {
            let mut n = 0;
            directories.visit(cancel,&mut |done,directory| {
                match self.reindex_dir_cancellable(directory,cancel) {
                    Err(_) if cancel.is_cancelled()=>return Ok(false),
                    Err(error)=>return Err(error),
                    Ok(Some(_))=>n+=1,
                    Ok(None)=>{},
                }
                on_progress(done,total);
                Ok(true)
            })?;
            Ok(n)
        };
        match run() {
            Ok(n) => {
                if cancel.is_cancelled() { transaction.rollback()?; } else { self.verify_binding()?; transaction.commit()?; }
                Ok(n)
            }
            Err(e) => {
                drop(transaction);
                Err(e)
            }
        }
    }

    // ---- 读取 ----------------------------------------------------------------------

    pub fn list(&self, q: &Query) -> Result<Page> {
        // A writer callback inside an uncommitted rebuild must never publish
        // counts/anchors under a revision that could later roll back.
        let shared_cache = self.conn.is_readonly("main")? || self.conn.is_autocommit();
        let transaction = if self.conn.is_autocommit() { Some(self.conn.unchecked_transaction()?) } else { None };
        let revision: i64 = self.conn.query_row("SELECT revision FROM library_state WHERE id=1", [], |row|row.get(0))?;
        let count_key = serde_json::to_string(&(&q.text, &q.content_type, q.year, &q.filters)).map_err(io::Error::other)?;
        let page_key = format!("{:?}:{count_key}",q.sort);
        let (mut wh, mut args): (Vec<String>, Vec<Value>) = (vec![], vec![]);
        for w in q.text.as_deref().unwrap_or("").split_whitespace() {
            // FTS narrows long literal substrings; LIKE remains the final authority
            // for wildcard escaping, Unicode and multiple-word semantics.
            if w.to_lowercase().chars().count() >= 3 && !w.contains('\0') {
                wh.push("rowid IN (SELECT rowid FROM movie_search WHERE movie_search MATCH ?)".into());
                args.push(Value::Text(format!("\"{}\"", w.to_lowercase().replace('"', "\"\""))));
            }
            wh.push("search LIKE ? ESCAPE '\\'".into());
            args.push(Value::Text(like_pattern(w)));
        }
        if let Some(ct) = q.content_type.as_deref().filter(|c| !c.is_empty()) {
            wh.push("content_type = ?".into());
            args.push(Value::Text(ct.into()));
        }
        if let Some(y) = q.year {
            wh.push("year = ?".into());
            args.push(Value::Integer(y as i64));
        }
        for filter in &q.filters {
            wh.push("code IN (SELECT code FROM movie_facet WHERE kind = ? AND value = ?)".into());
            args.push(Value::Text(filter.kind.as_str().into()));
            args.push(Value::Text(filter.value.clone()));
        }
        let count_clause = if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) };
        let total = match shared_cache.then(||self.read_cache.count(revision,&count_key)).flatten() {
            Some(count)=>count,
            None=>{
                let count=self.conn.query_row(&format!("SELECT COUNT(*) FROM movie {count_clause}"), params_from_iter(args.iter()), |r|r.get::<_,i64>(0))? as usize;
                if shared_cache {self.read_cache.remember_count(revision,count_key,count);}count
            }
        };
        let anchor=shared_cache.then(||self.read_cache.anchor(revision,&page_key,q.offset)).flatten();
        let limit=if q.limit==0{60}else{q.limit.clamp(1,500)};
        let (sql,args)=query::page_sql(&wh,args,q.sort,q.offset,limit,anchor);
        let mut statement=self.conn.prepare(&sql)?;
        let rows=statement.query_map(params_from_iter(args.iter()),|row|Ok((self.card(row),row.get::<_,Value>(10)?)))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        if let Some((card,value))=rows.last().filter(|_|shared_cache) {
            self.read_cache.remember_anchor(revision,page_key,query::Anchor{offset:q.offset.saturating_add(rows.len()),value:value.clone(),code:card.code.clone()});
        }
        let mut items:Vec<MovieCard>=rows.into_iter().map(|(card,_)|card).collect();
        drop(statement);
        self.mark_favorites(&mut items)?;
        if let Some(transaction)=transaction {transaction.commit()?;}
        Ok(Page{total,items})
    }

    /// 卡片的收藏标记来自维度表(与 sidecar 同步写入)。
    fn mark_favorites(&self, cards: &mut [MovieCard]) -> Result<()> {
        let mut statement = self.conn.prepare_cached("SELECT 1 FROM movie_facet WHERE code = ?1 AND kind = 'favorite'")?;
        for card in cards { card.favorite = statement.exists(params![card.code])?; }
        Ok(())
    }

    fn card(&self, r: &rusqlite::Row) -> MovieCard {
        let actors: String = r.get(3).unwrap_or_default();
        MovieCard {
            code: r.get(0).unwrap_or_default(),
            title: r.get(1).ok().flatten(),
            year: r.get(2).ok().flatten(),
            actors: serde_json::from_str(&actors).unwrap_or_default(),
            poster: r.get::<_, Option<String>>(4).ok().flatten().map(|p| self.abs(&p).to_string_lossy().into_owned()),
            poster_version: r.get(9).ok().flatten(),
            content_type: r.get(5).ok().flatten(),
            score: r.get::<_, Option<f64>>(6).ok().flatten().map(|s| s as f32),
            added_at: r.get(7).unwrap_or(0),
            parts: r.get::<_, i64>(8).unwrap_or(0) as usize,
            favorite: false,
        }
    }

    pub fn get(&self, code: &str) -> Result<Option<MovieDetail>> {
        let row = self
            .conn
            .query_row(
                "SELECT code,title,year,actors,poster,content_type,score,added_at,(SELECT COUNT(*) FROM file WHERE file.code=movie.code),poster_version,meta_json,dir,fanart_version
                 FROM movie WHERE code = ?1",
                params![code],
                |r| Ok((self.card(r), r.get::<_, String>(10)?, r.get::<_, String>(11)?, r.get::<_, Option<String>>(12)?)),
            )
            .optional()?;
        let Some((mut card, meta_json, rel_dir, fanart_version)) = row else { return Ok(None) };
        self.mark_favorites(std::slice::from_mut(&mut card))?;
        let mut st = self.conn.prepare("SELECT rel_path,part,size FROM file WHERE code = ?1 ORDER BY COALESCE(part,0), rel_path")?;
        let files = st
            .query_map(params![code], |r| {
                Ok(MovieFile { path: self.abs(&r.get::<_, String>(0)?).to_string_lossy().into_owned(), part: r.get::<_, Option<u8>>(1)?, size: r.get::<_, i64>(2)? as u64 })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let dir = self.abs(&rel_dir);
        let meta: Metadata = serde_json::from_str(&meta_json).unwrap_or_default();
        let assets = crate::assets::Paths::for_movie(&dir, &meta)?;
        let fanart = assets.fanart.exists().then(|| assets.fanart.to_string_lossy().into_owned());
        Ok(Some(MovieDetail { card, meta, dir: dir.to_string_lossy().into_owned(), files, fanart, fanart_version }))
    }

    pub fn facets(&self) -> Result<Facets> {
        let total = self.conn.query_row("SELECT COUNT(*) FROM movie", [], |r| r.get::<_, i64>(0))? as usize;
        let years = self
            .conn
            .prepare("SELECT year,COUNT(*) FROM movie WHERE year IS NOT NULL GROUP BY year ORDER BY year DESC")?
            .query_map([], |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i64>(1)? as usize)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let content_types = self
            .conn
            .prepare("SELECT content_type,COUNT(*) FROM movie WHERE content_type IS NOT NULL GROUP BY content_type ORDER BY COUNT(*) DESC")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let values = |kind: FacetKind| -> Result<Vec<(String, usize)>> {
            Ok(self.conn
                .prepare_cached("SELECT value, COUNT(*) FROM movie_facet WHERE kind = ?1 GROUP BY value ORDER BY COUNT(*) DESC, value LIMIT ?2")?
                .query_map(params![kind.as_str(), FACET_VALUES as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)))?
                .collect::<std::result::Result<Vec<_>, _>>()?)
        };
        let favorites = self.conn.query_row("SELECT COUNT(*) FROM movie_facet WHERE kind = 'favorite'", [], |r| r.get::<_, i64>(0))? as usize;
        Ok(Facets { total, years, content_types, tags: values(FacetKind::Tag)?, actors: values(FacetKind::Actor)?, studios: values(FacetKind::Studio)?, series: values(FacetKind::Series)?, badges: values(FacetKind::Badge)?, favorites })
    }

    /// 某番号的评分来源(详情页展示用)。
    pub fn scores(&self, code: &str) -> Result<Vec<SourcedScore>> {
        Ok(self.get(code)?.map(|d| d.meta.scores).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        metadata::{Actor, Gender, PartialMetadata},
        organize::{execute, plan},
        code::parse_code,
    };

    fn meta(code: &str, title: &str, actor: &str, date: &str) -> Metadata {
        Metadata {
            code: code.into(),
            fields: PartialMetadata {
                title: Some(title.into()),
                actors: vec![Actor::new(actor, Gender::Female)],
                release_date: Some(date.into()),
                studio: Some("Studio".into()),
                ..Default::default()
            },
            scores: vec![SourcedScore { site: "a".into(), score: 80.0 }],
            ..Default::default()
        }
    }

    /// 用真实的 plan/execute 在临时库里造一部片(无网络:没有封面 URL,海报由测试自己放)。
    fn place(root: &Path, m: &Metadata, file: &str) -> PathBuf {
        let src = root.parent().unwrap().join("in").join(file);
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"video").unwrap();
        let code = parse_code(file).unwrap();
        let p = plan(&src, &code, m, root, "{actor}/{code}");
        let ex = execute(&p, &mut |_| unreachable!(), &|| false);
        assert!(ex.error.is_none());
        ex.done.iter().find_map(|o| if let crate::organize::Op::MoveFile { to, .. } = o { to.parent().map(Path::to_path_buf) } else { None }).unwrap()
    }

    fn lib() -> (tempfile::TempDir, PathBuf, Library) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("lib");
        fs::create_dir_all(&root).unwrap();
        let l = Library::open(&root).unwrap();
        (d, root, l)
    }

    #[test]
    fn artwork_versions_follow_content_across_reindex_edit_and_restore() {
        let (_d, root, library) = lib();
        let dir = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-03-04"), "AAA-111.mp4");
        let poster = dir.join("AAA-111-poster.jpg");
        let fanart = dir.join("AAA-111-fanart.jpg");
        fs::write(&poster, b"old").unwrap(); fs::write(&fanart, b"background").unwrap();
        library.reindex_dir(&dir).unwrap();
        let version = |library: &Library| serde_json::to_value(library.list(&Query::default()).unwrap().items[0].clone()).unwrap()["posterVersion"].clone();
        let original = version(&library);
        assert_eq!(original, crate::journal::replacement::digest(b"old"));
        assert_eq!(version(&library), original, "queries must not create new versions");
        // A metadata edit/reindex keeps the image revision stable.
        library.reindex_dir(&dir).unwrap(); assert_eq!(version(&library), original);
        fs::write(&poster, b"new").unwrap(); library.reindex_dir(&dir).unwrap();
        assert_ne!(version(&library), original, "same-size replacements must invalidate the image");
        fs::write(&poster, b"old").unwrap(); library.reindex_dir(&dir).unwrap();
        assert_eq!(version(&library), original, "restored bytes recover the original version");
        let detail = serde_json::to_value(library.get("AAA-111").unwrap().unwrap()).unwrap();
        assert_eq!(detail["fanartVersion"], crate::journal::replacement::digest(b"background"));
    }

    fn faceted(code: &str, actor: &str, tags: &[&str], studio: &str, series: Option<&str>) -> Metadata {
        let mut m = meta(code, code, actor, "2021-01-01");
        m.fields.tags = tags.iter().map(|t| t.to_string()).collect();
        m.fields.studio = Some(studio.into());
        m.fields.series = series.map(String::from);
        m
    }
    fn codes(library: &Library, filters: Vec<FacetFilter>) -> Vec<String> {
        let mut out: Vec<_> = library.list(&Query { filters, ..Default::default() }).unwrap().items.into_iter().map(|c| c.code).collect();
        out.sort();
        out
    }
    fn f(kind: FacetKind, value: &str) -> FacetFilter { FacetFilter { kind, value: value.into() } }

    #[test]
    fn facet_filters_combine_and_survive_a_full_rebuild() {
        let (_d, root, library) = lib();
        for (m, file) in [
            (faceted("AAA-111", "Foo", &["Drama"], "S1", None), "AAA-111-C.mp4"),
            (faceted("BBB-222", "Bar", &["Drama", "Comedy"], "S2", None), "BBB-222.mp4"),
            (faceted("CCC-333", "Foo", &[], "S1", Some("X")), "CCC-333-4K.mp4"),
        ] { let dir = place(&root, &m, file); library.reindex_dir(&dir).unwrap(); }
        let check = |library: &Library| {
            assert_eq!(codes(library, vec![f(FacetKind::Actor, "Foo")]), ["AAA-111", "CCC-333"]);
            assert_eq!(codes(library, vec![f(FacetKind::Actor, "Foo"), f(FacetKind::Badge, "subtitle")]), ["AAA-111"]);
            assert_eq!(codes(library, vec![f(FacetKind::Tag, "Drama")]), ["AAA-111", "BBB-222"]);
            assert_eq!(codes(library, vec![f(FacetKind::Studio, "S1"), f(FacetKind::Series, "X")]), ["CCC-333"]);
            assert_eq!(codes(library, vec![f(FacetKind::Badge, "4k")]), ["CCC-333"]);
            assert!(codes(library, vec![f(FacetKind::Tag, "Drama"), f(FacetKind::Tag, "Missing")]).is_empty());
            assert_eq!(library.list(&Query { filters: vec![f(FacetKind::Actor, "Foo")], ..Default::default() }).unwrap().total, 2);
            let facets = library.facets().unwrap();
            assert_eq!(facets.actors, vec![("Foo".to_string(), 2), ("Bar".to_string(), 1)]);
            assert_eq!(facets.tags, vec![("Drama".to_string(), 2), ("Comedy".to_string(), 1)]);
            assert_eq!(facets.studios, vec![("S1".to_string(), 2), ("S2".to_string(), 1)]);
            assert_eq!(facets.series, vec![("X".to_string(), 1)]);
            assert!(facets.badges.contains(&("subtitle".to_string(), 1)) && facets.badges.contains(&("4k".to_string(), 1)));
        };
        check(&library);
        library.rebuild(&CancellationToken::new(), &mut |_, _| {}).unwrap();
        check(&library);
        // Removing a movie removes its facets.
        let gone = root.join("Bar/BBB-222");
        fs::remove_dir_all(&gone).unwrap();
        library.reindex_dir(&gone).unwrap();
        assert_eq!(library.facets().unwrap().tags, vec![("Drama".to_string(), 1)]);
    }

    #[test]
    #[ignore = "5000-movie facet query timing; run alone"]
    fn facet_query_timing_on_a_large_library() {
        let (_d, root, library) = lib();
        let start = std::time::Instant::now();
        let transaction = library.conn.unchecked_transaction().unwrap();
        for i in 0..5000 {
            let code = format!("ABC-{i:05}");
            let m = faceted(&code, &format!("Actor{}", i % 400), &[&format!("Tag{}", i % 60), &format!("Tag{}", i % 7)], &format!("S{}", i % 30), (i % 3 == 0).then_some("Series"));
            let file = if i % 4 == 0 { format!("{code}-C.mp4") } else { format!("{code}.mp4") };
            let dir = place(&root, &m, &file);
            library.reindex_dir(&dir).unwrap();
        }
        transaction.commit().unwrap();
        println!("index 5000: {} ms", start.elapsed().as_millis());
        let time = |label: &str, q: Query| { let t = std::time::Instant::now(); let page = library.list(&q).unwrap(); println!("{label}: total={} {} µs", page.total, t.elapsed().as_micros()); };
        time("no filter", Query::default());
        time("actor", Query { filters: vec![f(FacetKind::Actor, "Actor7")], ..Default::default() });
        time("tag+badge", Query { filters: vec![f(FacetKind::Tag, "Tag3"), f(FacetKind::Badge, "subtitle")], ..Default::default() });
        time("actor+tag+studio+text", Query { text: Some("abc".into()), filters: vec![f(FacetKind::Actor, "Actor7"), f(FacetKind::Tag, "Tag3"), f(FacetKind::Studio, "S7")], ..Default::default() });
        let t = std::time::Instant::now(); let facets = library.facets().unwrap(); println!("facets: actors={} tags={} {} µs", facets.actors.len(), facets.tags.len(), t.elapsed().as_micros());
    }

    #[test]
    fn upgrading_an_index_backfills_facets_from_cached_rows() {
        let (_d, root, library) = lib();
        let dir = place(&root, &faceted("AAA-111", "Foo", &["Drama"], "S1", None), "AAA-111-C.mp4");
        library.reindex_dir(&dir).unwrap();
        library.conn.execute_batch("DROP TABLE movie_facet; PRAGMA user_version=3;").unwrap();
        drop(library);
        let library = Library::open(&root).unwrap();
        assert_eq!(codes(&library, vec![f(FacetKind::Tag, "Drama"), f(FacetKind::Badge, "subtitle")]), ["AAA-111"]);
        assert_eq!(library.conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap(), 4);
    }

    #[test]
    fn legacy_index_migration_preserves_rows_and_does_not_rebuild() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        fs::create_dir(root.join(".kiroku")).unwrap();
        let connection = Connection::open(Library::db_path(root)).unwrap();
        let legacy = SCHEMA.replace("  poster_version TEXT, fanart_version TEXT,\n", "");
        connection.execute_batch(&format!("{legacy} PRAGMA user_version=1;")).unwrap();
        connection.execute("INSERT INTO movie(code,dir,poster,added_at,meta_json) VALUES('OLD-001','old','old/poster.jpg',1,'{}')", []).unwrap();
        drop(connection);
        let library = Library::open(root).unwrap();
        let page = library.list(&Query::default()).unwrap();
        assert_eq!(page.total, 1, "migration must retain cached rows without scanning disk");
        assert_eq!(page.items[0].code, "OLD-001");
        assert!(page.items[0].poster_version.is_none());
        assert_eq!(library.conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap(), 4);
        drop(library);
        assert_eq!(Library::open(root).unwrap().list(&Query::default()).unwrap().total, 1);
    }

    #[test]
    fn readers_see_old_committed_index_through_rebuild_commit_cancel_and_panic() {
        for outcome in ["commit", "cancel", "panic"] {
            let (_temporary, root, writer) = lib();
            let movie = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-01-01"), "AAA-111.mp4");
            writer.reindex_dir(&movie).unwrap();
            let mut changed: Metadata = serde_json::from_slice(&fs::read(movie.join(SIDECAR)).unwrap()).unwrap();
            changed.fields.title = Some("Changed".into());
            fs::write(movie.join(SIDECAR), serde_json::to_vec(&changed).unwrap()).unwrap();
            place(&root, &meta("BBB-222", "New", "Bar", "2023-01-01"), "BBB-222.mp4");
            let reader = writer.reader_handle().open().unwrap();
            let cancel = CancellationToken::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rebuild(&cancel, &mut |_, _| {
                let page = reader.list(&Query::default()).unwrap();
                assert_eq!(page.total, 1); assert_eq!(page.items.len(), 1);
                assert_eq!(page.items[0].title.as_deref(), Some("Alpha"));
                assert_eq!(reader.facets().unwrap().total, 1);
                let detail = reader.get("AAA-111").unwrap().unwrap();
                assert_eq!(detail.card.parts, 1); assert_eq!(detail.files.len(), 1);
                assert_eq!(detail.meta.fields.title.as_deref(), Some("Alpha"));
                if outcome == "cancel" { cancel.cancel(); }
                if outcome == "panic" { panic!("injected worker failure"); }
            })));
            assert_eq!(result.is_err(), outcome == "panic");
            let page = reader.list(&Query::default()).unwrap();
            assert_eq!(page.total, if outcome == "commit" { 2 } else { 1 });
            assert_eq!(reader.get("AAA-111").unwrap().unwrap().meta.fields.title.as_deref(), Some(if outcome == "commit" { "Changed" } else { "Alpha" }));
        }
    }

    #[test]
    fn failed_reindex_rolls_back_metadata_and_file_set_as_one_unit() {
        let (_temporary, root, writer) = lib();
        let movie = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-01-01"), "AAA-111.mp4");
        writer.reindex_dir(&movie).unwrap();
        let reader = writer.reader_handle().open().unwrap();
        let mut metadata: Metadata = serde_json::from_slice(&fs::read(movie.join(SIDECAR)).unwrap()).unwrap();
        metadata.fields.title = Some("Must not leak".into());
        fs::write(movie.join(SIDECAR), serde_json::to_vec(&metadata).unwrap()).unwrap();
        writer.conn.execute_batch("CREATE TEMP TRIGGER fail_file_insert BEFORE INSERT ON file BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
        assert!(writer.reindex_dir(&movie).is_err());
        let detail = reader.get("AAA-111").unwrap().unwrap();
        assert_eq!(detail.meta.fields.title.as_deref(), Some("Alpha"));
        assert_eq!(detail.card.parts, 1); assert_eq!(detail.files.len(), 1);
        writer.conn.execute_batch("DROP TRIGGER fail_file_insert;").unwrap();
        writer.reindex_dir(&movie).unwrap();
        assert_eq!(reader.get("AAA-111").unwrap().unwrap().meta.fields.title.as_deref(), Some("Must not leak"));
    }

    #[cfg(unix)]
    #[test]
    fn root_loss_at_final_rebuild_progress_rolls_back_before_commit() {
        let (temporary, root, writer) = lib();
        let movie = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-01-01"), "AAA-111.mp4");
        writer.reindex_dir(&movie).unwrap();
        let mut changed: Metadata = serde_json::from_slice(&fs::read(movie.join(SIDECAR)).unwrap()).unwrap();
        changed.fields.title = Some("Changed".into());
        fs::write(movie.join(SIDECAR), serde_json::to_vec(&changed).unwrap()).unwrap();
        let handle = writer.reader_handle(); let reader = handle.open().unwrap();
        let offline = temporary.path().join("offline");
        let result = writer.rebuild(&CancellationToken::new(), &mut |_, _| fs::rename(&root, &offline).unwrap());
        assert!(result.is_err()); assert!(handle.open().is_err()); assert!(!root.exists());
        fs::rename(&offline, &root).unwrap();
        assert_eq!(reader.get("AAA-111").unwrap().unwrap().meta.fields.title.as_deref(), Some("Alpha"));
    }

    #[test]
    #[ignore = "explicit local read latency measurement during a held rebuild"]
    fn concurrent_read_latency_measurement() {
        use std::{sync::mpsc, time::{Duration, Instant}};
        let (_temporary, root, writer) = lib();
        for i in 0..1000 {
            let code = format!("PERF-{i:04}"); let dir = root.join(&code); fs::create_dir(&dir).unwrap();
            let metadata = meta(&code, &format!("Performance {i}"), "Foo", "2021-01-01");
            fs::write(dir.join(SIDECAR), serde_json::to_vec(&metadata).unwrap()).unwrap();
            fs::write(dir.join(format!("{code}.mp4")), b"v").unwrap(); writer.reindex_dir(&dir).unwrap();
        }
        let handle = writer.reader_handle();
        let (started_tx, started_rx) = mpsc::channel(); let (release_tx, release_rx) = mpsc::channel();
        let rebuild = std::thread::spawn(move || {
            let mut paused = false;
            writer.rebuild(&CancellationToken::new(), &mut |_, _| {
                if !paused { paused = true; started_tx.send(()).unwrap(); release_rx.recv_timeout(Duration::from_secs(30)).unwrap(); }
            }).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut latencies = vec![];
        for _ in 0..30 {
            let start = Instant::now(); let reader = handle.open().unwrap();
            let page = reader.list(&Query { limit: 120, ..Default::default() }).unwrap();
            assert_eq!(page.total, 1000); assert_eq!(page.items.len(), 120);
            assert_eq!(reader.get("PERF-0000").unwrap().unwrap().files.len(), 1);
            assert_eq!(reader.facets().unwrap().total, 1000);
            latencies.push(start.elapsed());
        }
        release_tx.send(()).unwrap(); rebuild.join().unwrap();
        latencies.sort();
        println!("1000 committed movies, writer paused inside rebuild; 30 independent connection + list(120) + detail + facets rounds: median={:?}, p95={:?}, max={:?}", latencies[15], latencies[28], latencies[29]);
        println!("Local temporary-directory measurement; verifies response while the rebuild cannot finish, not external-disk throughput.");
    }

    #[test]
    #[ignore = "explicit real-directory rebuild staging growth measurement"]
    fn rebuild_directory_growth_measurement() {
        for count in [100,1000,5000] {
            let(_temporary,root,writer)=lib();
            for i in 0..count {
                let directory=root.join(format!("AAA-{i:05}"));fs::create_dir(&directory).unwrap();
                let metadata=meta(&format!("AAA-{i:05}"),"Movie","Foo","2021-01-01");
                fs::write(directory.join(SIDECAR),serde_json::to_vec(&metadata).unwrap()).unwrap();
                fs::write(directory.join("movie.mp4"),b"v").unwrap();
            }
            let cancel=CancellationToken::new();let start=std::time::Instant::now();
            let manifest=rebuild::Directories::capture(&root,&cancel).unwrap();assert_eq!(manifest.len().unwrap(),count);
            let(bytes,cache)=manifest.measurement();assert_eq!(cache,-1024);
            println!("real directories={count}; disk manifest={bytes} bytes; SQLite page cache target=1024KiB; capture={:?}; no retained Rust path vector",start.elapsed());
            drop(manifest);let start=std::time::Instant::now();assert_eq!(writer.rebuild(&cancel,&mut|_,_|{}).unwrap(),count);
            assert_eq!(writer.reader_handle().open().unwrap().list(&Query{limit:1,..Default::default()}).unwrap().total,count);
            println!("real rebuild {count}: {:?}; measurement excludes SQLite native allocations and process RSS",start.elapsed());
        }
    }

    #[test]
    #[ignore = "explicit synthetic 10k/100k indexed-row query measurement"]
    fn large_library_query_measurement() {
        use std::time::Instant;
        let (_temporary, root, writer) = lib();
        let directory = root.join("fixture"); fs::create_dir(&directory).unwrap(); fs::write(directory.join("movie.mp4"), b"v").unwrap();
        let mut previous = 0;
        for size in [10_000, 100_000] {
            let seed = Instant::now(); let transaction = writer.conn.unchecked_transaction().unwrap();
            for i in previous..size {
                let metadata = meta(&format!("PERF-{i:06}"), &format!("{} {i:06}", if i % 1000 == 0 { "Needle" } else { "Movie" }), "Foo", "2021-01-01");
                fs::write(directory.join(SIDECAR), serde_json::to_vec(&metadata).unwrap()).unwrap(); writer.reindex_dir(&directory).unwrap();
            }
            transaction.commit().unwrap(); previous = size;
            let handle = writer.reader_handle();
            println!("synthetic index rows={size}; seed={:?}; fixture reuses one temporary directory through reindex_dir (not a filesystem rebuild benchmark)", seed.elapsed());
            for (label, query) in [
                ("first", Query { limit:120, ..Default::default() }),
                ("deep_random", Query { limit:120, offset:size-120, ..Default::default() }),
                ("contains", Query { text:Some("needle".into()), limit:120, ..Default::default() }),
            ] {
                let mut times=vec![];
                for _ in 0..10 { let start=Instant::now(); let page=handle.open().unwrap().list(&query).unwrap(); assert_eq!(page.total, if label=="contains" {size/1000} else {size}); times.push(start.elapsed()); }
                times.sort(); println!("{label}: median={:?}, max={:?}",times[5],times[9]);
            }
            let before=handle.open().unwrap().list(&Query {limit:120,offset:size-240,..Default::default()}).unwrap();assert_eq!(before.items.len(),120);
            let start=Instant::now();let next=handle.open().unwrap().list(&Query {limit:120,offset:size-120,..Default::default()}).unwrap();assert_eq!(next.items.len(),120);
            println!("deep_sequential_next={:?}",start.elapsed());
        }
    }

    #[test]
    fn disk_rebuild_keeps_the_legacy_component_order_for_duplicate_codes() {
        let(_temporary,root,writer)=lib();let mut directories=vec![root.join("a/movie"),root.join("a-dup")];
        for directory in &directories {
            fs::create_dir_all(directory).unwrap();
            fs::write(directory.join(SIDECAR),serde_json::to_vec(&meta("AAA-111","Duplicate","Foo","2021-01-01")).unwrap()).unwrap();
            fs::write(directory.join("AAA-111.mp4"),b"v").unwrap();
        }
        directories.sort();writer.rebuild(&CancellationToken::new(),&mut|_,_|{}).unwrap();
        let detail=writer.reader_handle().open().unwrap().get("AAA-111").unwrap().unwrap();
        assert_eq!(detail.dir,directories.last().unwrap().to_string_lossy());
        assert_eq!(detail.files[0].path,directories.last().unwrap().join("AAA-111.mp4").to_string_lossy());
    }

    #[test]
    fn cached_counts_and_seek_pages_match_all_sorts_filters_and_nulls() {
        let (_temporary,root,writer)=lib();
        for i in 0..120 {
            let code=format!("CACHE-{i:03}");let mut metadata=meta(&code,if i%2==0{"Alpha"}else{"alpha"},"Foo",if i%4==0{""}else{"2021-01-01"});
            if i%5==0{metadata.fields.title=None;}if i%3==0{metadata.scores.clear();}else{metadata.scores[0].score=(i%7)as f32;}
            let directory=place(&root,&metadata,&format!("{code}.mp4"));writer.reindex_dir(&directory).unwrap();
        }
        let handle=writer.reader_handle();
        for sort in [Sort::Added,Sort::Title,Sort::Year,Sort::Score] {
            for text in [None,Some("alpha".to_string())] {
                let all=handle.open().unwrap().list(&Query{sort,text:text.clone(),limit:500,..Default::default()}).unwrap();
                let mut codes=vec![];
                for offset in (0..all.total).step_by(7) {
                    let page=handle.open().unwrap().list(&Query{sort,text:text.clone(),offset,limit:7,..Default::default()}).unwrap();
                    assert_eq!(page.total,all.total);codes.extend(page.items.into_iter().map(|card|card.code));
                }
                assert_eq!(codes,all.items.into_iter().map(|card|card.code).collect::<Vec<_>>(),"{sort:?} {text:?}");
            }
        }
        for i in 0..100{handle.open().unwrap().list(&Query{text:Some(format!("missing-{i}")),..Default::default()}).unwrap();}
        let(counts,anchors,hits,seeks)=writer.read_cache.stats();assert!(counts<=32&&anchors<=64);assert!(hits>0&&seeks>0);
        assert!(handle.open().unwrap().list(&Query{offset:usize::MAX,..Default::default()}).unwrap().items.is_empty());
    }

    #[test]
    fn indexed_substrings_match_the_legacy_like_contract_for_unicode_and_controls() {
        let(_temporary,root,writer)=lib();
        for(code,title)in[("AAA-111","Alpha\0suffix"),("BBB-222","İstanbul Straße Ελληνικά 汉字测试 🙂🙂🙂 %_\\ \"hello\"")] {
            let directory=place(&root,&meta(code,title,"Foo","2021-01-01"),&format!("{code}.mp4"));writer.reindex_dir(&directory).unwrap();
        }
        for word in ["Alpha\0tail","İst","Straße","Ελλ","汉字测试","🙂🙂🙂","%_\\","\"hello\""] {
            let expected:i64=writer.conn.query_row("SELECT COUNT(*) FROM movie WHERE search LIKE ?1 ESCAPE '\\'",[like_pattern(word)],|row|row.get(0)).unwrap();
            assert_eq!(writer.reader_handle().open().unwrap().list(&Query{text:Some(word.into()),..Default::default()}).unwrap().total,expected as usize,"{word:?}");
        }
    }

    #[test]
    fn deep_seek_queries_use_index_ranges_instead_of_scanning_past_old_rows() {
        let(_temporary,_root,writer)=lib();
        for (sort,value) in [(Sort::Added,Value::Integer(-100)),(Sort::Title,Value::Text("Alpha".into())),(Sort::Year,Value::Integer(-2021)),(Sort::Score,Value::Real(-80.0))] {
            let(sql,args)=query::page_sql(&[],vec![],sort,100000,120,Some(query::Anchor{offset:100000,value,code:"AAA-111".into()}));
            let mut statement=writer.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let details=statement.query_map(params_from_iter(args.iter()),|row|row.get::<_,String>(3)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
            assert!(details.iter().any(|detail|detail.contains("SEARCH movie USING INDEX")&&detail.contains("code>?")),"{sort:?}: {details:?}");
            assert!(!details.iter().any(|detail|detail.starts_with("SCAN movie")),"{sort:?}: {details:?}");
        }
    }

    #[test]
    fn writer_callbacks_cannot_publish_uncommitted_counts_that_survive_rollback() {
        let(_temporary,root,writer)=lib();let first=place(&root,&meta("AAA-111","First","Foo","2021-01-01"),"AAA-111.mp4");writer.reindex_dir(&first).unwrap();
        let handle=writer.reader_handle();assert_eq!(handle.open().unwrap().list(&Query::default()).unwrap().total,1);
        let second=place(&root,&meta("BBB-222","Second","Foo","2021-01-01"),"BBB-222.mp4");
        let transaction=writer.conn.unchecked_transaction().unwrap();writer.reindex_dir(&second).unwrap();
        assert_eq!(writer.list(&Query::default()).unwrap().total,2);transaction.rollback().unwrap();
        fs::remove_file(first.join("AAA-111.mp4")).unwrap();writer.reindex_dir(&first).unwrap();
        assert_eq!(handle.open().unwrap().list(&Query::default()).unwrap().total,0);
        let reopened=Library::open(&root).unwrap();reopened.reindex_dir(&second).unwrap();
        assert_eq!(handle.open().unwrap().list(&Query::default()).unwrap().total,1,"other core writer commits invalidate cached totals via the persisted revision");
    }

    #[test]
    fn trigram_search_keeps_literal_and_short_term_semantics() {
        let (_temporary, root, writer) = lib();
        for (code,title) in [("AAA-111","Alpha 100%_x 汉字测试"),("BBB-222","alphabet Foo"),("CCC-333","Foo quotation \"hello\"")] {
            let directory=place(&root,&meta(code,title,"Foo","2021-01-01"),&format!("{code}.mp4"));writer.reindex_dir(&directory).unwrap();
        }
        let reader=writer.reader_handle().open().unwrap();
        for (text,total) in [("alpha",2),("100%_",1),("汉字",1),("字测试",1),("hello",1),("alpha foo",2),("quotation \"hello\"",1)] {
            assert_eq!(reader.list(&Query{text:Some(text.into()),..Default::default()}).unwrap().total,total,"{text}");
        }
        let plan:String=writer.conn.query_row("EXPLAIN QUERY PLAN SELECT rowid FROM movie_search WHERE movie_search MATCH 'alpha'",[],|row|row.get(3)).unwrap();
        assert!(plan.contains("VIRTUAL TABLE INDEX"),"{plan}");
    }

    #[test]
    fn index_search_filter_sort_and_detail() {
        let (_d, root, l) = lib();
        let dir_a = place(&root, &meta("AAA-111", "Alpha Story", "Foo", "2021-03-04"), "AAA-111.mp4");
        let dir_b = place(&root, &meta("BBB-222", "Beta 100%_x", "Bar", "2023-01-01"), "BBB-222-CD1.mp4");
        place(&root, &meta("BBB-222", "Beta 100%_x", "Bar", "2023-01-01"), "BBB-222-CD2.mp4");
        fs::write(dir_a.join("AAA-111-poster.jpg"), b"jpg").unwrap();
        assert_eq!(l.reindex_dir(&dir_a).unwrap().as_deref(), Some("AAA-111"));
        assert_eq!(l.reindex_dir(&dir_b).unwrap().as_deref(), Some("BBB-222"));

        let all = l.list(&Query::default()).unwrap();
        assert_eq!(all.total, 2);
        // 搜索:番号/标题/演员,多词 AND,大小写不敏感
        for (text, want) in [("aaa", 1), ("alpha foo", 1), ("alpha bar", 0), ("BAR", 1), ("studio", 2)] {
            assert_eq!(l.list(&Query { text: Some(text.into()), ..Default::default() }).unwrap().total, want, "{text}");
        }
        // 通配符被当作普通字符
        assert_eq!(l.list(&Query { text: Some("100%_".into()), ..Default::default() }).unwrap().total, 1);
        assert_eq!(l.list(&Query { text: Some("%".into()), ..Default::default() }).unwrap().total, 1);
        // 筛选与排序
        assert_eq!(l.list(&Query { year: Some(2023), ..Default::default() }).unwrap().items[0].code, "BBB-222");
        assert_eq!(l.list(&Query { sort: Sort::Year, ..Default::default() }).unwrap().items[0].code, "BBB-222");
        assert_eq!(l.list(&Query { sort: Sort::Title, ..Default::default() }).unwrap().items[0].code, "AAA-111");
        // 分页
        let p = l.list(&Query { limit: 1, offset: 1, sort: Sort::Title, ..Default::default() }).unwrap();
        assert_eq!((p.total, p.items.len(), p.items[0].code.as_str()), (2, 1, "BBB-222"));

        let card = all.items.iter().find(|c| c.code == "AAA-111").unwrap();
        assert!(card.poster.as_deref().unwrap().ends_with("poster.jpg") && Path::new(card.poster.as_ref().unwrap()).is_absolute());
        let d = l.get("BBB-222").unwrap().unwrap();
        assert_eq!(d.files.len(), 2);
        assert_eq!(d.files.iter().map(|f| f.part).collect::<Vec<_>>(), vec![Some(1), Some(2)]);
        assert_eq!(d.card.parts, 2);
        assert!(d.card.poster.is_none());
        assert!(l.get("NOPE-1").unwrap().is_none());

        let f = l.facets().unwrap();
        assert_eq!((f.total, f.years.clone()), (2, vec![(2023, 1), (2021, 1)]));
        assert_eq!(f.content_types, vec![("censored".to_string(), 2)]);
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_reports_missing_root_and_preserves_cached_index() {
        let (d, root, l) = lib();
        let movie = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-01-01"), "AAA-111.mp4");
        l.reindex_dir(&movie).unwrap();
        fs::rename(&root, d.path().join("disconnected")).unwrap();
        assert!(l.rebuild(&CancellationToken::new(), &mut |_,_| {}).is_err());
        assert_eq!(l.list(&Query::default()).unwrap().total, 1);
    }

    #[test]
    fn rebuild_callback_panic_does_not_leave_a_partial_transaction() {
        let (_d, root, l) = lib();
        let old = place(&root, &meta("AAA-111", "Old", "Foo", "2021-01-01"), "AAA-111.mp4");
        l.reindex_dir(&old).unwrap();
        fs::remove_dir_all(old).unwrap();
        place(&root, &meta("BBB-222", "New", "Foo", "2021-01-01"), "BBB-222.mp4");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.rebuild(&CancellationToken::new(), &mut |_,_| panic!("worker failed"))));
        assert!(result.is_err());
        assert_eq!(l.list(&Query::default()).unwrap().items[0].code, "AAA-111");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_metadata_fails_rebuild_and_preserves_movie_and_files() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root, l) = lib();
        let movie = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-01-01"), "AAA-111.mp4");
        l.reindex_dir(&movie).unwrap();
        let paths = [movie.join("kiroku.json"), movie.join("AAA-111.nfo")];
        for path in &paths { fs::set_permissions(path, fs::Permissions::from_mode(0)).unwrap(); }
        let result = l.rebuild(&CancellationToken::new(), &mut |_,_| {});
        for path in &paths { fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap(); }
        assert!(result.is_err());
        assert_eq!(l.list(&Query::default()).unwrap().total, 1);
        assert_eq!(l.get("AAA-111").unwrap().unwrap().files.len(), 1);
        assert_eq!(l.rebuild(&CancellationToken::new(), &mut |_,_| {}).unwrap(), 1);
    }

    #[test]
    fn rollback_then_reindex_removes_and_partial_removal_keeps_remaining_parts() {
        let (_d, root, l) = lib();
        let m = meta("AAA-111", "Alpha", "Foo", "2021-03-04");
        let dir = place(&root, &m, "AAA-111-CD1.mp4");
        place(&root, &m, "AAA-111-CD2.mp4");
        l.reindex_dir(&dir).unwrap();
        assert_eq!(l.get("AAA-111").unwrap().unwrap().files.len(), 2);
        // 删掉一个分片后重索引:另一个仍在
        fs::remove_file(dir.join("AAA-111-CD2.mp4")).unwrap();
        l.reindex_dir(&dir).unwrap();
        assert_eq!(l.get("AAA-111").unwrap().unwrap().files.len(), 1);
        // 视频全没了(回滚后):索引移除,不残留
        fs::remove_file(dir.join("AAA-111-CD1.mp4")).unwrap();
        assert_eq!(l.reindex_dir(&dir).unwrap(), None);
        assert!(l.get("AAA-111").unwrap().is_none());
        // 目录整个不存在也不报错
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(l.reindex_dir(&dir).unwrap(), None);
    }

    #[test]
    fn rebuild_equals_incremental_and_reads_foreign_nfo() {
        let (_d, root, l) = lib();
        let dir_a = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-03-04"), "AAA-111.mp4");
        l.reindex_dir(&dir_a).unwrap();
        // 外部工具生成的库:只有 NFO + 视频
        let foreign = root.join("Other/Some Movie (2019)");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("Some Movie.mkv"), b"v").unwrap();
        fs::write(foreign.join("movie.nfo"), "<movie><title>Some Movie</title><year>2019</year><actor><name>Zed</name></actor></movie>").unwrap();
        // 没有视频的目录不入库
        let empty = root.join("Empty");
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("movie.nfo"), "<movie><title>x</title></movie>").unwrap();

        let before = l.list(&Query { sort: Sort::Title, ..Default::default() }).unwrap();
        let mut ticks = vec![];
        let n = l.rebuild(&CancellationToken::new(), &mut |d, t| ticks.push((d, t))).unwrap();
        assert_eq!(n, 2);
        assert_eq!(ticks.last(), Some(&(3, 3)));
        let after = l.list(&Query { sort: Sort::Title, ..Default::default() }).unwrap();
        assert_eq!(after.total, 2);
        let a = after.items.iter().find(|c| c.code == "AAA-111").unwrap();
        let b = before.items.iter().find(|c| c.code == "AAA-111");
        assert_eq!((a.title.clone(), a.year, a.actors.clone()), (Some("Alpha".into()), Some(2021), vec!["Foo".to_string()]));
        assert!(b.is_none() || (b.unwrap().title == a.title && b.unwrap().year == a.year));
        let f = after.items.iter().find(|c| c.code == "Some Movie (2019)").unwrap();
        assert_eq!((f.title.as_deref(), f.year, f.actors.clone()), (Some("Some Movie"), Some(2019), vec!["Zed".to_string()]));
    }

    #[test]
    fn reopen_keeps_data_and_is_idempotent() {
        let (_d, root, l) = lib();
        let dir = place(&root, &meta("AAA-111", "Alpha", "Foo", "2021-03-04"), "AAA-111.mp4");
        l.reindex_dir(&dir).unwrap();
        l.reindex_dir(&dir).unwrap();
        drop(l);
        let l = Library::open(&root).unwrap();
        assert_eq!(l.list(&Query::default()).unwrap().total, 1);
    }
}
