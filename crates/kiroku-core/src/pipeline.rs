//! 扫描 → 刮削 → 计划 → 执行 的共享流程,CLI 与桌面客户端共用。
//! 只依赖核心模块;进度通过回调上报,取消通过 `CancellationToken`。

use crate::{
    badge::Phase,
    aggregate::aggregate,
    code::{parse_text, Code, FileInfo, Mosaic},
    journal::{Journal, JournalItem, JournalStatus},
    metadata::Metadata,
    net::Http,
    organize::{self, Exec, Op, Plan, PlanOpts},
    settings::Settings,
    source::{Source, SourceError},
    video_filter::{SkipReason, VideoFilter},
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

mod images;

pub const VIDEO_EXT: &[&str] = &["mp4", "mkv", "avi", "wmv", "mov", "ts", "m2ts", "flv", "rmvb"];

// ---------------------------------------------------------------------------
// 扫描
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanItem {
    /// 本次扫描内的序号
    pub id: u32,
    pub path: PathBuf,
    pub size: u64,
    pub info: FileInfo,
    /// None = 未识别到番号,不参与刮削
    pub code: Option<Code>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub items: Vec<ScanItem>,
    /// 不同番号的数量
    pub codes: usize,
    pub unrecognized: usize,
    /// 不算正片而跳过的视频(样片/预告、黑名单、过小)
    pub skipped: Vec<SkippedFile>,
    #[serde(flatten)]
    pub summary: ScanSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub path: PathBuf,
    pub size: u64,
    pub reason: SkipReason,
}

mod scan;
pub use scan::{scan, scan_preview, scan_summary, ScanIssue, ScanSummary};

/// 对一个文件夹的快速体检,用来在选媒体库/待整理目录时给出提醒。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirInfo {
    pub videos: usize,
    pub nfos: usize,
    /// 目录很大,只统计了一部分
    pub truncated: bool,
}

/// 最多看到第 4 层、5000 个条目就停,保证网络盘上也很快返回。
pub fn inspect_dir(root: &Path) -> DirInfo {
    let mut info = DirInfo::default();
    for (n, e) in WalkDir::new(root).max_depth(4).into_iter().filter_entry(|e| e.file_name() != ".kiroku").filter_map(Result::ok).enumerate() {
        if n >= 5000 {
            info.truncated = true;
            break;
        }
        if !e.file_type().is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("._") {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap_or("");
        if ext == "nfo" {
            info.nfos += 1;
        } else if VIDEO_EXT.contains(&ext) {
            info.videos += 1;
        }
    }
    info
}

/// 手动指定番号:用于解析不出番号、或解析错了的文件。
/// 输入按「自由文本」规则解析(`ABC-123`、`abc123`、`FC2-PPV-1234567` 等都可以),
/// 文件自身的分集/字幕信息保留。无法识别时返回错误,条目不变。
pub fn override_code(item: &mut ScanItem, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("番号不能为空".into());
    }
    let parsed = parse_text(text);
    let mut code = Code::from_info(&parsed).ok_or_else(|| format!("无法识别「{text}」这种番号格式,试试 ABC-123 或 FC2-PPV-1234567"))?;
    code.part = item.info.cd;
    code.subtitled = item.info.has_subtitle;
    // 文件名里的破解/流出跟随文件;无码/有码随新番号的类型
    if matches!(item.info.mosaic, Some(Mosaic::Cracked | Mosaic::Leaked)) {
        code.mosaic = item.info.mosaic;
    }
    item.info.mosaic = code.mosaic;
    code.definition = item.info.definition.map(String::from);
    item.info.number = Some(code.id.clone());
    item.info.content_type = code.content_type;
    item.info.matched = true;
    item.code = Some(code);
    Ok(())
}

// ---------------------------------------------------------------------------
// 刮削
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ScrapeError {
    /// 该内容类型没有启用的数据源路由
    NoRoute,
    NotFound,
    Cancelled,
    /// 每个站点的失败说明
    Failed(Vec<String>),
}

impl ScrapeError {
    pub fn message(&self) -> String {
        match self {
            Self::NoRoute => "该内容类型没有可用的数据源".into(),
            Self::NotFound => "所有数据源都没有找到".into(),
            Self::Cancelled => "已取消".into(),
            Self::Failed(v) => v.join("; "),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScrapeStatus {
    Pending,
    Scraping { site: String },
    /// 已找到,正在用 AI 汉化标题/简介
    Translating,
    Found,
    NotFound,
    NoRoute,
    Failed { message: String },
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeEvent {
    pub code: String,
    pub status: ScrapeStatus,
    pub done: usize,
    pub total: usize,
    /// 找到时附带的结果摘要,供界面边刮削边展示
    pub preview: Option<Preview>,
    /// 附加提示(如 AI 汉化失败),不影响刮削结果
    pub note: Option<String>,
}

/// 刮削结果摘要(只含展示用字段)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub title: Option<String>,
    pub original_title: Option<String>,
    /// 封面图远程地址:优先横版封面(界面用 object-position 显示右侧封面部分)
    pub cover: Option<String>,
    pub cover_is_landscape: bool,
    pub release_date: Option<String>,
    pub runtime_min: Option<u32>,
    pub studio: Option<String>,
    pub actors: Vec<String>,
    /// 0–100
    pub score: Option<f32>,
    /// 实际给出数据的站点
    pub sources: Vec<String>,
}

impl Preview {
    pub fn of(m: &Metadata) -> Self {
        let f = &m.fields;
        let (cover, landscape) = match (f.thumb_urls.first(), f.poster_urls.first()) {
            (Some(t), _) => (Some(t.clone()), true),
            (None, Some(p)) => (Some(p.clone()), false),
            _ => (None, false),
        };
        let mut sources: Vec<String> = m.provenance.values().cloned().collect();
        sources.sort();
        sources.dedup();
        Self {
            title: f.title.clone(),
            original_title: f.original_title.clone(),
            cover,
            cover_is_landscape: landscape,
            release_date: f.release_date.clone(),
            runtime_min: f.runtime_min,
            studio: f.studio.clone(),
            actors: f.actors.iter().take(4).map(|a| a.name.clone()).collect(),
            score: m.mean_score(),
            sources,
        }
    }
}

pub struct Scraper {
    pub settings: Settings,
    pub http: Http,
    /// 代理与全局不同的来源各自的 client(共享 Cookie 与同 host 间隔)
    source_http: HashMap<String, Http>,
    /// 本批内连续失败的站点暂停;Scraper 按批创建,所以下一批或重试自动恢复
    breaker: std::sync::Mutex<Breaker>,
    sources: Vec<Box<dyn Source>>,
    translator: Option<crate::translate::Translator>,
}

/// 页面里是否出现了番号的常见写法(`midv-018`、`midv018`、DMM cid `midv00018`),不区分大小写。
fn page_mentions_code(page: &str, code: &Code) -> bool {
    let key = code.search_key().to_ascii_lowercase();
    let page = page.to_ascii_lowercase();
    let mut forms = vec![key.clone(), key.replace('-', "")];
    if let Some((prefix, number)) = key.rsplit_once('-').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())) {
        forms.push(format!("{prefix}{number:0>5}"));
        forms.push(format!("{prefix}-{}", number.trim_start_matches('0')));
    }
    forms.iter().any(|f| !f.is_empty() && page.contains(f.as_str()))
}

/// 连续这么多次可重试类失败(连不上、被拦)后,本批内暂停该站。
const PAUSE_AFTER: u32 = 3;

#[derive(Default)]
struct Breaker { streak: HashMap<String, u32>, paused: HashMap<String, String> }

impl Breaker {
    fn record(&mut self, site: &str, result: &Result<crate::metadata::PartialMetadata, SourceError>) {
        match result {
            Err(e @ (SourceError::Network(_) | SourceError::Blocked(_))) => {
                let streak = self.streak.entry(site.to_string()).or_default();
                *streak += 1;
                if *streak >= PAUSE_AFTER && !self.paused.contains_key(site) {
                    log::warn!(target: "kiroku::scrape", "{site} 连续 {PAUSE_AFTER} 次失败,本批暂停:{e}");
                    self.paused.insert(site.to_string(), e.to_string());
                }
            }
            _ => { self.streak.remove(site); }
        }
    }
}

/// 代理与全局不同、或走求解器的来源各建一个 client;同一代理只建一次。
/// 走求解器时代理交给求解器出站(ADR-0007)。
pub(crate) fn source_clients(settings: &Settings, global: &Http) -> reqwest::Result<HashMap<String, Http>> {
    let global_proxy = settings.proxy.as_deref().filter(|p| !p.is_empty());
    let mut by_proxy: HashMap<Option<&str>, Http> = HashMap::new();
    let mut out = HashMap::new();
    for cfg in settings.sources.iter().filter(|c| c.enabled) {
        let proxy = settings.proxy_for(cfg);
        if let Some((url, timeout)) = settings.solver_for(cfg) {
            out.insert(cfg.id.clone(), global.with_solver(Some(crate::net::Solver::new(url, timeout, proxy)?)));
            continue;
        }
        if proxy == global_proxy { continue }
        let http = match by_proxy.get(&proxy) { Some(h) => h.clone(), None => { let h = global.with_proxy(proxy)?; by_proxy.insert(proxy, h.clone()); h } };
        out.insert(cfg.id.clone(), http);
    }
    Ok(out)
}

impl Scraper {
    pub fn new(settings: Settings) -> Result<Self, String> {
        let sources = settings.build_sources();
        Self::with_sources(settings, sources)
    }

    /// AI 汉化使用 `path` 处的译文缓存(没有启用 AI 时不起作用)。
    pub fn with_translation_cache(mut self, path: PathBuf) -> Self {
        self.translator = self.translator.map(|t| t.with_cache(path));
        self
    }

    /// Dependency injection for callers using custom source implementations.
    pub fn with_sources(settings: Settings, sources: Vec<Box<dyn Source>>) -> Result<Self, String> {
        let http = settings.build_http().map_err(|e| e.to_string())?;
        let source_http = source_clients(&settings, &http).map_err(|e| e.to_string())?;
        let translator = if settings.ai.usable() {
            Some(crate::translate::Translator::new(settings.ai.clone())?)
        } else {
            None
        };
        Ok(Self {
            settings,
            http,
            source_http,
            breaker: Default::default(),
            sources,
            translator,
        })
    }

    /// 按番号内容类型的路由顺序请求支持该番号的站,聚合后返回。
    pub async fn scrape_item(
        &self,
        code: &Code,
        cancel: &CancellationToken,
        on_site: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Metadata, ScrapeError> {
        self.scrape_cached(code, &Default::default(), true, cancel, on_site)
            .await
    }

    /// Cache only successful pre-translation results, keyed by the original code identity.
    /// Credential/proxy fingerprints invalidate identity changes without persisting secrets.
    pub async fn scrape_cached(
        &self,
        code: &Code,
        cached: &std::collections::BTreeMap<String, crate::metadata::SourceSnapshot>,
        force: bool,
        cancel: &CancellationToken,
        on_site: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Metadata, ScrapeError> {
        self.scrape_with(code, cached, force, None, cancel, on_site).await
    }

    /// 用户指定某站详情页:该站使用这一页的结果(不搜索),路线上的其他站照常查询;
    /// 发行日期冲突时以这一页为准。页面里找不到文件番号时附带提示,由调用方决定是否采用。
    pub async fn scrape_url(&self, code: &Code, url: &str, cancel: &CancellationToken) -> Result<Metadata, ScrapeError> {
        let fail = |m: String| ScrapeError::Failed(vec![m]);
        let cfg = self.settings.source_for_url(url).map_err(fail)?;
        let id = cfg.id.clone();
        if !self.settings.chains_for(code.content_type).route.contains(&id) {
            return Err(fail(format!("数据源 {id} 不在该类型影片的路线中,请先在设置里加入路线")));
        }
        let source = self.sources.iter().find(|s| s.id() == id).ok_or_else(|| fail(format!("数据源 {id} 不可用")))?;
        let http = self.source_http.get(&id).unwrap_or(&self.http);
        let fetched = tokio::select! {
            _ = cancel.cancelled() => return Err(ScrapeError::Cancelled),
            r = source.fetch_detail(url, code, http) => r,
        };
        let (mut part, page) = match fetched {
            None => return Err(fail(format!("数据源 {id} 不支持按详情页地址刮削"))),
            Some(Err(e)) => return Err(fail(format!("{id}: {e}"))),
            Some(Ok(found)) => found,
        };
        part.source_url.get_or_insert_with(|| url.to_string());
        let mut m = self.scrape_with(code, &Default::default(), true, Some((source.id(), part)), cancel, &|_| {}).await?;
        if !page_mentions_code(&page, code) {
            m.scrape_notes.insert(0, format!("详情页里没有找到番号 {},请确认是同一部影片", code.id));
        }
        Ok(m)
    }

    async fn scrape_with(
        &self,
        code: &Code,
        cached: &std::collections::BTreeMap<String, crate::metadata::SourceSnapshot>,
        force: bool,
        pinned: Option<(crate::source::SourceId, crate::metadata::PartialMetadata)>,
        cancel: &CancellationToken,
        on_site: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Metadata, ScrapeError> {
        let pinned_id = pinned.as_ref().map(|(id, _)| *id);
        let mut pinned = pinned;
        if cancel.is_cancelled() {
            return Err(ScrapeError::Cancelled);
        }
        let chains = self.settings.chains_for(code.content_type);
        if chains.route.is_empty() {
            return Err(ScrapeError::NoRoute);
        }
        let (mut parts, mut errors, mut not_found) = (vec![], vec![], 0usize);
        let mut asked = 0usize;
        let mut snapshots = cached.clone();
        // 先按路线决定每站结果从哪里来(指定页 / 缓存 / 联网),再并发请求需要联网的站;
        // 结果仍按路线顺序处理,聚合与完成先后无关。同站间隔由 Http 按 host 保证。
        enum Slot<'a> { Ready(crate::source::SourceId, crate::metadata::PartialMetadata), Fetch(&'a dyn Source, &'a crate::settings::SourceCfg) }
        let mut slots = vec![];
        let mut paused = vec![];
        for site in &chains.route {
            let Some(s) = self.sources.iter().find(|s| s.id() == site) else {
                continue;
            };
            if !s.supports(code) {
                continue;
            }
            let Some(cfg) = self
                .settings
                .sources
                .iter()
                .find(|cfg| cfg.enabled && cfg.id == *site)
            else {
                continue;
            };
            asked += 1;
            if let Some((id, part)) = pinned.take_if(|(id, _)| *id == s.id()) {
                snapshots.insert(id.into(), crate::metadata::SourceSnapshot::configured(code, cfg, self.settings.proxy_for(cfg), &part));
                slots.push(Slot::Ready(id, snapshots[id].fields.clone()));
                continue;
            }
            if !force {
                if let Some(snapshot) = cached.get(site).filter(|snapshot| {
                    snapshot.matches_config(code, cfg, self.settings.proxy_for(cfg))
                }) {
                    slots.push(Slot::Ready(s.id(), snapshot.fields.clone()));
                    continue;
                }
            }
            if let Some(reason) = self.breaker.lock().unwrap_or_else(|e| e.into_inner()).paused.get(site.as_str()) {
                paused.push(format!("{site}: 来源暂停(本批连续 {PAUSE_AFTER} 次失败:{reason}),可稍后重试未完成项"));
                continue;
            }
            slots.push(Slot::Fetch(s.as_ref(), cfg));
        }
        let fetching: Vec<_> = slots.iter().filter_map(|slot| match slot { Slot::Fetch(s, _) => Some(*s), Slot::Ready(..) => None }).collect();
        if !fetching.is_empty() {
            on_site(&fetching.iter().map(|s| s.id()).collect::<Vec<_>>().join(","));
        }
        let requests = fetching.iter().map(|s| s.fetch(code, self.source_http.get(s.id()).unwrap_or(&self.http)));
        let mut fetched = tokio::select! {
            _ = cancel.cancelled() => return Err(ScrapeError::Cancelled),
            r = futures_util::future::join_all(requests) => r,
        }.into_iter();
        for slot in slots {
            let (s, cfg) = match slot {
                Slot::Ready(id, part) => { parts.push((id, part)); continue }
                Slot::Fetch(s, cfg) => (s, cfg),
            };
            let result = fetched.next().expect("one result per fetch");
            self.breaker.lock().unwrap_or_else(|e| e.into_inner()).record(s.id(), &result);
            match result {
                Ok(p) if crate::metadata::usable_partial(&p) => {
                    snapshots.insert(
                        s.id().into(),
                        crate::metadata::SourceSnapshot::configured(
                            code,
                            cfg,
                            self.settings.proxy_for(cfg),
                            &p,
                        ),
                    );
                    log::debug!(target: "kiroku::scrape", "{} @ {}: 命中", code.id, s.id());
                    parts.push((s.id(), snapshots[s.id()].fields.clone()))
                }
                Ok(_) | Err(SourceError::NotFound) => {
                    log::debug!(target: "kiroku::scrape", "{} @ {}: 未找到", code.id, s.id());
                    not_found += 1
                }
                Err(e) => {
                    log::warn!(target: "kiroku::scrape", "{} @ {}: {e}", code.id, s.id());
                    errors.push(format!("{}: {e}", s.id()))
                }
            }
        }
        if parts.is_empty() {
            errors.extend(paused);
            return Err(if !errors.is_empty() {
                ScrapeError::Failed(errors)
            } else if asked == 0 || not_found > 0 {
                ScrapeError::NotFound
            } else {
                ScrapeError::NoRoute
            });
        }
        let (parts, mut conflicts) = crate::aggregate::drop_date_conflicts_around(parts, pinned_id);
        conflicts.splice(0..0, paused);
        for n in &conflicts { log::warn!(target: "kiroku::scrape", "{} {n}", code.id) }
        let mut result = aggregate(&code.id, &parts, &chains);
        result.raw_sources = snapshots;
        crate::localization::capture_original(&mut result);
        crate::names::project(&result, &self.settings.name_rules)
            .map(|p| Metadata { scrape_notes: conflicts, ..p.metadata })
            .map_err(|e| ScrapeError::Failed(vec![e]))
    }

    /// Reaggregate raw snapshots then translate only selected, unlocked fields.
    pub async fn refresh_item(
        &self,
        code: &Code,
        before: &Metadata,
        fields: &[String],
        replace: bool,
        force: bool,
        cancel: &CancellationToken,
    ) -> Result<Metadata, String> {
        let raw = self
            .scrape_cached(code, &before.raw_sources, force, cancel, &|_| {})
            .await
            .map_err(|e| e.message())?;
        self.adopt_selected(before, raw, fields, replace, cancel).await
    }

    /// 同 [`Self::refresh_item`],但候选来自用户指定的某站详情页(见 [`Self::scrape_url`])。
    pub async fn refresh_item_url(
        &self,
        code: &Code,
        before: &Metadata,
        fields: &[String],
        replace: bool,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<Metadata, String> {
        let raw = self.scrape_url(code, url, cancel).await.map_err(|e| e.message())?;
        self.adopt_selected(before, raw, fields, replace, cancel).await
    }

    /// 只采用所选字段(锁定字段不动),需要时 AI 汉化新采用的标题/简介。
    async fn adopt_selected(
        &self,
        before: &Metadata,
        raw: Metadata,
        fields: &[String],
        replace: bool,
        cancel: &CancellationToken,
    ) -> Result<Metadata, String> {
        let mut after = crate::refresh::merge_selected(before, &raw, fields, replace)
            .map_err(|e| e.to_string())?;
        if let Some(tr) = &self.translator {
            let selected = |f: &str| {
                fields.iter().any(|v| v == f)
                    && !before.locked_fields.contains(f)
                    && (replace
                        || !crate::refresh::nonempty(&serde_json::to_value(before).unwrap()[f]))
            };
            let title = if selected("title") {
                raw.fields
                    .title
                    .as_deref()
                    .filter(|s| crate::translate::has_kana(s))
            } else {
                None
            };
            let plot = if selected("plot") {
                raw.fields
                    .plot
                    .as_deref()
                    .filter(|s| crate::translate::has_kana(s))
            } else {
                None
            };
            if title.is_some() || plot.is_some() {
                let translated = tokio::select! { _ = cancel.cancelled() => return Err("已取消".into()), t = tr.translate(title, plot) => t.map_err(|e| format!("AI 汉化失败，尚未保存：{e}"))? };
                // Fresh adopted text versions were captured by merge_selected.
                apply_translation(&mut after, translated);
            }
        }
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        // scrape_cached already projected the candidate; merge_selected adopted only requested fields.
        Ok(after)
    }
}

/// 按番号去重后顺序刮削(同站点有 ≥5s 限速,并发没有意义)。取消后已完成的结果仍会返回。
pub async fn scrape_all(
    sc: &Scraper,
    codes: &[Code],
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ScrapeEvent) + Send + Sync),
) -> HashMap<String, Result<Metadata, ScrapeError>> {
    let mut seen = HashSet::new();
    let unique: Vec<&Code> = codes.iter().filter(|c| seen.insert(c.id.as_str())).collect();
    let total = unique.len();
    let mut out = HashMap::new();
    for (i, code) in unique.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        let on_site = |site: &str| on_event(ScrapeEvent { code: code.id.clone(), status: ScrapeStatus::Scraping { site: site.to_string() }, done: i, total, preview: None, note: None });
        let r = sc.scrape_item(code, cancel, &on_site).await;
        out.insert(code.id.clone(), finish_item(sc, &code.id, (i, total), r, cancel, on_event).await);
    }
    out
}

/// 用用户指定的详情页刮削一部影片;事件与 [`scrape_all`] 相同(done/total 为 1)。
pub async fn scrape_url_one(
    sc: &Scraper,
    code: &Code,
    url: &str,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ScrapeEvent) + Send + Sync),
) -> Result<Metadata, ScrapeError> {
    let site = sc.settings.source_for_url(url).map(|c| c.id.clone()).unwrap_or_default();
    on_event(ScrapeEvent { code: code.id.clone(), status: ScrapeStatus::Scraping { site }, done: 0, total: 1, preview: None, note: None });
    let r = sc.scrape_url(code, url, cancel).await;
    finish_item(sc, &code.id, (0, 1), r, cancel, on_event).await
}

/// 一部影片刮削后的共同收尾:AI 汉化(失败只记提示)、合并提示、日志和结果事件。
async fn finish_item(
    sc: &Scraper,
    id: &str,
    (i, total): (usize, usize),
    mut r: Result<Metadata, ScrapeError>,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ScrapeEvent) + Send + Sync),
) -> Result<Metadata, ScrapeError> {
    let emit = |status: ScrapeStatus, done: usize, preview: Option<Preview>, note: Option<String>| {
        on_event(ScrapeEvent { code: id.to_string(), status, done, total, preview, note })
    };
    let mut note = None;
    if let (Ok(m), Some(tr)) = (r.as_mut(), sc.translator.as_ref()) {
        let f = &m.fields;
        if f.title.as_deref().is_some_and(crate::translate::has_kana) || f.plot.as_deref().is_some_and(crate::translate::has_kana) {
            emit(ScrapeStatus::Translating, i, None, None);
            let (title, plot) = (f.title.clone(), f.plot.clone());
            let t = tokio::select! {
                _ = cancel.cancelled() => Err("已取消".to_string()),
                t = tr.translate(title.as_deref(), plot.as_deref()) => t,
            };
            match t {
                Ok(t) => apply_translation(m, t),
                Err(e) => note = Some(format!("AI 汉化失败:{e}")),
            }
        }
    }
    if let Ok(m) = &r {
        let notes: Vec<&str> = m.scrape_notes.iter().map(String::as_str).chain(note.as_deref()).collect();
        if !notes.is_empty() { note = Some(notes.join(";")) }
    }
    let status = match &r {
        Ok(_) => ScrapeStatus::Found,
        Err(ScrapeError::NotFound) => ScrapeStatus::NotFound,
        Err(ScrapeError::NoRoute) => ScrapeStatus::NoRoute,
        Err(ScrapeError::Cancelled) => ScrapeStatus::Cancelled,
        Err(e @ ScrapeError::Failed(_)) => ScrapeStatus::Failed { message: e.message() },
    };
    match &r {
        Ok(m) => log::info!(target: "kiroku::scrape", "[{}/{total}] {} 找到:{}", i + 1, id, m.fields.title.as_deref().or(m.fields.original_title.as_deref()).unwrap_or("(无标题)")),
        Err(ScrapeError::Cancelled) => log::info!(target: "kiroku::scrape", "[{}/{total}] {} 已取消", i + 1, id),
        Err(e) => log::warn!(target: "kiroku::scrape", "[{}/{total}] {} {}", i + 1, id, e.message()),
    }
    if let Some(n) = &note {
        log::warn!(target: "kiroku::scrape", "  {id} {n}");
    }
    emit(status, i + 1, r.as_ref().ok().map(Preview::of), note);
    r
}

/// Retry only missing results; successful metadata belongs to the caller and stays untouched.
pub async fn scrape_missing(sc: &Scraper, codes: &[Code], successful: &HashMap<String, Metadata>, cancel: &CancellationToken, on_event: &(dyn Fn(ScrapeEvent) + Send + Sync)) -> HashMap<String, Result<Metadata, ScrapeError>> {
    let mut unique = HashSet::new();
    let codes: Vec<_> = codes.iter().filter(|code| unique.insert(code.id.clone())).collect();
    let mut reused=0;
    if !cancel.is_cancelled() { for code in &codes { if let Some(meta)=successful.get(&code.id) { reused+=1;on_event(ScrapeEvent{code:code.id.clone(),status:ScrapeStatus::Found,done:reused,total:codes.len(),preview:Some(Preview::of(meta)),note:Some("复用已有元数据，不重新联网".into())}); } } }
    let missing: Vec<_> = codes.iter().filter(|code| !successful.contains_key(&code.id)).map(|code|(*code).clone()).collect();
    scrape_all(sc, &missing, cancel, &|mut ev| { ev.done+=reused;ev.total=codes.len();on_event(ev) }).await
}

/// 把译文写回元数据；调用方在来源聚合/字段采用时保存原文。旧元数据只恢复可靠原文。
pub fn apply_translation(m: &mut Metadata, t: crate::translate::Translated) {
    crate::localization::recover_original(m);
    if let Some(zh) = t.title.filter(|_| !m.locked_fields.contains("title")) {
        if m.fields.original_title.is_none() {m.fields.original_title=m.text_versions.original.title.clone();}
        m.text_versions.localized.entry("zh-Hans".into()).or_default().title=Some(zh.clone());
        m.text_versions.localized.entry("zh-Hans".into()).or_default().provenance.insert("title".into(),"ai".into());
        m.fields.title = Some(zh);m.provenance.insert("title".into(), "ai".into());
    }
    if let Some(zh) = t.plot.filter(|_| !m.locked_fields.contains("plot")) {
        m.text_versions.localized.entry("zh-Hans".into()).or_default().plot=Some(zh.clone());
        m.text_versions.localized.entry("zh-Hans".into()).or_default().provenance.insert("plot".into(),"ai".into());
        m.fields.plot = Some(zh);m.provenance.insert("plot".into(), "ai".into());
    }
}

// ---------------------------------------------------------------------------
// 计划
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Conflict {
    InvalidTemplate,
    /// 目标文件已存在
    TargetExists,
    /// 本批内另一个文件已占用同一目标
    DuplicateInBatch,
    InvalidLibrary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDetail { pub kind: Conflict, pub path: PathBuf }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub item_id: u32,
    pub code: String,
    pub title: Option<String>,
    pub from: PathBuf,
    pub to: PathBuf,
    /// 随该项一并写出的文件名(NFO、海报…)
    pub extras: Vec<String>,
    pub conflict: Option<Conflict>,
    /// 冲突的实际产物路径与原因(含共享元数据)。
    pub conflicts: Vec<ConflictDetail>,
    /// 默认勾选:有冲突的默认不选
    pub selected: bool,
    /// 作为同分片新版本并存(ADR-0008),默认不选
    pub new_version: bool,
    #[serde(skip)]
    pub ops: Vec<Op>,
    #[serde(skip)]
    pub root: PathBuf,
    /// Ordered candidates per target; kept out of legacy operation journals.
    #[serde(skip)]
    pub image_candidates: HashMap<PathBuf, Vec<ImageCandidate>>,
    pub notices: Vec<String>,
    #[serde(skip)]
    pub append_preview: Option<crate::append::AppendPreview>,
    #[serde(skip)]
    pub root_guard: Option<RootGuard>,
}

mod root_guard;
pub use root_guard::RootGuard;

pub fn build_plans(items: &[(&ScanItem, &Code)], metas: &HashMap<String, Metadata>, root: &Path, tpl: &str, opts: PlanOpts) -> Vec<PlanItem> {
    build_plans_cancellable(items,metas,root,tpl,opts,&CancellationToken::new()).expect("fresh planning token cannot be cancelled")
}

pub fn build_plans_cancellable(items:&[(&ScanItem,&Code)],metas:&HashMap<String,Metadata>,root:&Path,tpl:&str,opts:PlanOpts,cancel:&CancellationToken)->io::Result<Vec<PlanItem>> {
    crate::append::check_cancel(cancel)?;
    let requested:Vec<_>=items.iter().filter(|(_,code)|metas.contains_key(&code.id)).map(|(_,code)|code.id.as_str()).collect();
    if requested.is_empty(){return Ok(vec![]);}
    let initially_missing=opts.create_library&&std::fs::symlink_metadata(root).is_err_and(|error|error.kind()==io::ErrorKind::NotFound);

    // 同一目标被多个文件争用(漏网的样片、广告、重复下载)时,大文件优先:按体积从大到小分配目标与元数据,
    // 输出仍保持原顺序。否则谁先被扫到谁占位,正片反而被判成批内重复。
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(items[i].0.size));
    let mut first_of: HashMap<&str, (usize, u8)> = HashMap::new();
    for &i in &order {
        let code = items[i].1;
        let part = code.part.unwrap_or(0);
        first_of.entry(code.id.as_str()).and_modify(|e| if part < e.1 { *e = (i, part) }).or_insert((i, part));
    }
    // 角标按同一番号的全部分片聚合:任一分片带中字/破解…即打标,清晰度取最高档
    let mut phases: HashMap<&str, Phase> = HashMap::new();
    for (_, code) in items {
        phases.entry(code.id.as_str()).or_default().merge(&Phase::of(code));
    }
    let (mut root_guard,root_error)=match RootGuard::capture(root,opts.create_library){Ok(g)=>(Some(g),None),Err(e)=>(None,Some(e.to_string()))};
    let discovery=if let Some(error)=&root_error{Err(io::Error::other(error.clone()))}else{crate::append::Targets::capture(root,requested,cancel)};
    let (targets,mut discovery_error)=match discovery{
        Ok(targets)=>(Some(targets),None),
        Err(error) if cancel.is_cancelled()=>return Err(error),
        Err(error) if initially_missing&&error.kind()==io::ErrorKind::NotFound=>(None,None),
        Err(error)=>(None,Some(error.to_string())),
    };
    if initially_missing&&targets.is_some(){match RootGuard::capture(root,false){Ok(guard)=>root_guard=Some(guard),Err(error)=>discovery_error=Some(error.to_string())}}
    let mut subtitle_dirs: HashMap<PathBuf, crate::subtitle::Directory> = HashMap::new();
    if opts.badges.external_subtitle {
        for (item, code) in items {
            let Some(dir) = item.path.parent() else { continue };
            let subs = subtitle_dirs.entry(dir.to_path_buf()).or_insert_with(|| crate::subtitle::Directory::read(dir)).siblings(&item.path, code);
            if crate::subtitle::any_chinese(&subs) {
                phases.entry(code.id.as_str()).or_default().subtitle = true;
            }
        }
    }
    let mut taken: HashSet<PathBuf> = HashSet::new();
    // 同一个字幕可能被多个视频认领(同号的 -C 与 -UC 版本、同目录的分片):只归第一个(即体积最大的)
    let mut subs_claimed: HashSet<PathBuf> = HashSet::new();
    let mut out = vec![];
    for &i in &order {
        let (item, code) = items[i];
        let Some(meta) = metas.get(&code.id) else { continue };
        crate::append::check_cancel(cancel)?;
        let discovery=if let Some(error)=&discovery_error{Err(io::Error::other(error.clone()))}else if let Some(snapshot)=&targets{snapshot.get(&code.id,cancel).map(|paths|paths.to_vec())}else{
            root_guard.as_ref().map(RootGuard::verify).transpose().and_then(|_|match std::fs::symlink_metadata(root){Err(error)if error.kind()==io::ErrorKind::NotFound=>Ok(vec![]),Err(error)=>Err(error),Ok(_)=>Err(io::Error::other("新库路径已出现，请重新生成计划"))})
        };
        let dirs=match discovery{Ok(paths)=>paths,Err(error) if cancel.is_cancelled()=>return Err(error),Err(error)=>{
            let message=format!("无法确认已有影片，请先检查媒体库：{error}");out.push((i,PlanItem{item_id:item.id,code:code.id.clone(),title:meta.fields.title.clone(),from:item.path.clone(),to:root.join(item.path.file_name().unwrap_or_default()),extras:vec![],conflict:Some(Conflict::InvalidLibrary),conflicts:vec![ConflictDetail{kind:Conflict::InvalidLibrary,path:root.to_path_buf()}],selected:false,new_version:false,ops:vec![],root:root.to_path_buf(),image_candidates:Default::default(),notices:vec![message],append_preview:None,root_guard:root_guard.clone()}));continue;
        }};
        {
            if !dirs.is_empty() {
                let preview = crate::append::preview_from_targets(root, &code.id, std::slice::from_ref(&item.path), None, None, cancel, &dirs);
                let (ops, guard, messages) = match preview {
                    Ok(mut p) => {let _=p.set_mode(opts.mode);(p.ops.clone(), Some(p.clone()), p.conflicts.clone())},
                    Err(e) => (vec![], None, vec![e.to_string()]),
                };
                let to = ops.first().map(|o| o.target().to_path_buf()).unwrap_or_else(|| dirs[0].join(item.path.file_name().unwrap_or_default()));
                let mut conflicts: Vec<_> = messages.iter().map(|_| ConflictDetail { kind: Conflict::TargetExists, path: to.clone() }).collect();
                for op in &ops { if !taken.insert(op.target().to_path_buf()) { conflicts.push(ConflictDetail { kind: Conflict::DuplicateInBatch, path: op.target().to_path_buf() }); } }
                let conflict = conflicts.first().map(|c| c.kind);
                let new_version=guard.as_ref().is_some_and(|p|!p.new_versions.is_empty());
                out.push((i, PlanItem { item_id:item.id, code:code.id.clone(), title:meta.fields.title.clone(), from:item.path.clone(), to, extras:vec![], selected:conflict.is_none() && !ops.is_empty() && !new_version, new_version, conflict, conflicts, notices: { let mut n=messages; if let Some(p)=&guard{for v in &p.new_versions{n.push(format!("库中已有该分片的其他版本，将作为新版本并存：{}；确认后请手动勾选",v.file_name().unwrap_or_default().to_string_lossy()));}n.extend(p.skipped.clone());if !p.ops.is_empty(){n.push("追加到已有影片，保留元数据和海报".into());}}n }, ops, root:root.to_path_buf(), image_candidates:Default::default(), append_preview:guard, root_guard:root_guard.clone() }));
                continue;
            }
        }
        let with_meta = first_of.get(code.id.as_str()).is_some_and(|(first, _)| *first == i);
        let companions = item.path.parent().map(|dir| subtitle_dirs.entry(dir.to_path_buf()).or_insert_with(|| crate::subtitle::Directory::read(dir)).siblings(&item.path, code)).unwrap_or_default();
        let naming=organize::preview_paths_with_subtitles(&item.path,code,meta,tpl,&opts,&companions);
        let mut naming_error=naming.as_ref().err().cloned();
        let mut notices=naming.map(|(_,_,warnings)|warnings).unwrap_or_else(|message|vec![message]);
        notices.extend(crate::localization::project(meta,&opts.nfo_language).warnings);
        notices.extend(crate::names::warnings(meta));
        let mut ops = if naming_error.is_some(){vec![]}else{organize::plan_with_subtitles(&item.path, code, meta, root, tpl, with_meta, &opts, &phases[code.id.as_str()], &companions).ops};
        if ops.is_empty() && naming_error.is_none(){let message="无法在 240 UTF-8 字节内保留番号、分片及字幕扩展名".to_string();notices.push(message.clone());naming_error=Some(message);}
        ops.retain(|op| match op {
            op if op.source().is_some_and(|from|from != item.path) => subs_claimed.insert(op.source().unwrap().to_path_buf()),
            _ => true,
        });
        let to = ops.iter().find(|o|o.source().is_some()).map(|o|o.target().to_path_buf()).unwrap_or_default();
        let mut conflicts: Vec<_> = ops.iter().filter_map(|op| {
            let path = op.target();
            let kind = if !taken.insert(path.to_path_buf()) { Some(Conflict::DuplicateInBatch) }
                else if organize::occupied(path) { Some(Conflict::TargetExists) } else { None };
            kind.map(|kind| ConflictDetail { kind, path: path.to_path_buf() })
        }).collect();
        if naming_error.is_some(){conflicts.push(ConflictDetail{kind:Conflict::InvalidTemplate,path:item.path.clone()});}
        let conflict = conflicts.first().map(|c| c.kind);
        let extras = ops
            .iter()
            .filter_map(|o| match o {
                Op::WriteFile { path, .. } | Op::Download { path, .. } => path.file_name().map(|n| n.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
        out.push((i, PlanItem {
            item_id: item.id,
            code: code.id.clone(),
            title: meta.fields.title.clone().or_else(|| meta.fields.original_title.clone()),
            from: item.path.clone(),
            to,
            extras,
            selected: conflict.is_none(),
            new_version: false,
            conflict,
            conflicts,
            notices,
            image_candidates: image_candidates(&ops, meta),
            ops,
            root: root.to_path_buf(),
            append_preview: None,
            root_guard: root_guard.clone(),
        }));
    }
    out.sort_by_key(|(i, _)| *i);
    let mut out: Vec<PlanItem> = out.into_iter().map(|(_, p)| p).collect();
    for p in &mut out {if opts.mode!=crate::placement::OrganizeMode::Move{p.notices.push(format!("{:?}：保留源文件，不清理原文件夹",opts.mode));
        let logical=p.ops.iter().filter_map(|op|op.source()).filter_map(|path|std::fs::metadata(path).ok()).map(|m|m.len()).sum::<u64>();
        p.notices.push(format!("媒体逻辑大小 {} 字节；预计新增独立媒体空间 {} 字节",logical,if opts.mode==crate::placement::OrganizeMode::Copy{logical}else{0}));}}
    // One metadata owner per code requires one final directory on a new import.
    // Explicit append has its own target-selection protocol and remains independent.
    let mut directories: HashMap<String, HashSet<PathBuf>> = HashMap::new();
    for p in &out { if p.append_preview.is_none() { if let Some(dir)=p.to.parent() {
        directories.entry(p.code.clone()).or_default().insert(dir.to_path_buf());
    } } }
    for p in &mut out { if p.append_preview.is_none() && directories.get(&p.code).is_some_and(|dirs|dirs.len()>1) {
        p.conflicts.insert(0,ConflictDetail{kind:Conflict::InvalidTemplate,path:p.to.clone()});
        p.notices.push("同一番号的分片必须使用同一目标目录，请修改目录模板".into());
        p.ops.clear();
    } }
    // 后续分片也依赖同组元数据;不可通过取消首片绕过其产物冲突。
    let shared: Vec<_> = out.iter().flat_map(|p| p.conflicts.iter().filter(|c| p.ops.iter().any(|op|
        op.source().is_none() && op.target() == c.path
    )).map(move |c| (p.code.clone(), p.to.parent().map(Path::to_path_buf), c.clone()))).collect();
    for p in &mut out {
        for (code, dir, conflict) in &shared {
            if *code == p.code && dir.as_deref() == p.to.parent() && !p.conflicts.iter().any(|c| c.path == conflict.path) {
                p.conflicts.push(conflict.clone());
            }
        }
        p.conflict = p.conflicts.first().map(|c| c.kind);
        // 新版本并存(ADR-0008)需要用户明确勾选
        p.selected = p.conflict.is_none() && !p.new_version;
    }
    crate::append::check_cancel(cancel)?;
    let validity=root_guard.as_ref().map(RootGuard::verify).transpose().and_then(|_|{
        if let Some(snapshot)=&targets{snapshot.verify()}else if initially_missing{match std::fs::symlink_metadata(root){Err(error)if error.kind()==io::ErrorKind::NotFound=>Ok(()),Err(error)=>Err(error),Ok(_)=>Err(io::Error::other("新库路径已出现，请重新生成计划"))}}else{Ok(())}
    });
    if let Err(error)=validity{for plan in &mut out{plan.selected=false;plan.conflict=Some(Conflict::InvalidLibrary);plan.conflicts.push(ConflictDetail{kind:Conflict::InvalidLibrary,path:root.to_path_buf()});plan.ops.clear();plan.append_preview=None;plan.notices.push(error.to_string());}}
    Ok(out)
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------

/// A candidate carries its own transform: portrait fallback must not inherit landscape cropping.
#[derive(Debug, Clone)]
pub struct ImageCandidate { pub url: String, pub crop: Option<organize::Crop> }

fn image_candidates(ops: &[Op], meta: &Metadata) -> HashMap<PathBuf, Vec<ImageCandidate>> {
    ops.iter().filter_map(|op| {
        let Op::Download { path, url, crop, .. } = op else { return None };
        let mut candidates = vec![];
        if crate::assets::is_poster(path) {
            candidates.extend(meta.fields.thumb_urls.iter().map(|url| ImageCandidate { url: url.clone(), crop: Some(organize::Crop::PosterRight) }));
            candidates.extend(meta.fields.poster_urls.iter().map(|url| ImageCandidate { url: url.clone(), crop: None }));
        } else if crate::assets::is_fanart(path) {
            candidates.extend(meta.fields.thumb_urls.iter().map(|url| ImageCandidate { url: url.clone(), crop: None }));
        } else { candidates.push(ImageCandidate { url: url.clone(), crop: *crop }); }
        Some((path.clone(), candidates))
    }).collect()
}

/// Missing artwork only: existing files (including damaged artwork) are never replaced.
/// The caller reads disk metadata and passes the canonical movie directory.
pub fn build_image_repair(meta: &Metadata, dir: &Path, root: &Path) -> io::Result<PlanItem> {
    let dir = dir.canonicalize()?;
    let root = root.canonicalize()?;
    if !dir.starts_with(&root) { return Err(io::Error::other("影片目录不在媒体库内")); }
    let assets = crate::assets::Paths::for_movie(&dir, meta)?;
    let mut ops = vec![];
    let mut notices = vec![];
    for base in [&assets.poster, &assets.fanart] {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            let path = base.with_extension(ext);
            if organize::occupied(&path) {
                let valid = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()).is_some();
                if !valid { notices.push(format!("{} 已存在但不是有效图片，保留原文件；请使用受控替换修复", path.display())); }
            }
        }
    }
    if let Some(url) = meta.fields.thumb_urls.first().or(meta.fields.poster_urls.first()) {
        for (path, crop) in [(&assets.poster, (!meta.fields.thumb_urls.is_empty()).then_some(organize::Crop::PosterRight)), (&assets.fanart, None)] {
            if path == &assets.poster && meta.locked_fields.contains("poster_urls") { notices.push("海报已锁定，补图跳过海报；请使用裁剪海报明确保存".into()); continue; }
            if path == &assets.fanart && meta.fields.thumb_urls.is_empty() { continue; }
            // Alternate image extensions are also existing artwork.
            let exists = ["jpg", "jpeg", "png", "webp"].iter().any(|ext| organize::occupied(&path.with_extension(ext)));
            if !exists { ops.push(Op::Download { url: url.clone(), path: path.clone(), crop, stamp: None }); }
        }
    }
    if ops.is_empty() && notices.is_empty() { notices.push("没有可补的缺失图片，或磁盘元数据没有图片候选".into()); }
    let extras = ops.iter().map(|op| op.target().file_name().unwrap().to_string_lossy().into_owned()).collect();
    Ok(PlanItem { item_id: 0, code: meta.code.clone(), title: meta.fields.title.clone(), from: dir.clone(), to: assets.poster, extras, conflict: None, conflicts: vec![], selected: true, new_version: false, image_candidates: image_candidates(&ops, meta), ops, root:root.clone(), notices, append_preview: None, root_guard:Some(RootGuard::capture(&root,false)?) })
}

/// Image counts are output tasks (including failed tasks), not candidate requests
/// or byte estimates. Shared URLs can serve several outputs from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplyPhase { Images, Organize, Index }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyProgress {
    pub phase: ApplyPhase,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub code: Option<String>,
}

async fn prepare_images(http: &Http, plans: &mut [PlanItem], cancel: &CancellationToken,
    on_progress: &(dyn Fn(ApplyProgress) + Send + Sync), memory_budget: usize,
) -> (images::PreparedImages, HashMap<u32, Vec<String>>) {
    // Serial requests already bound concurrency to one. Artwork must not inherit
    // the scraper's 5s host interval; retry backoff/Retry-After still apply.
    let http = http.for_images();
    let total = plans.iter().flat_map(|p| &p.ops).filter(|op| matches!(op, Op::Download { .. })).count();
    let (mut done, mut failed) = (0, 0);
    on_progress(ApplyProgress { phase: ApplyPhase::Images, done, total, failed, code: None });
    let mut images = images::PreparedImages::new(memory_budget);
    let mut warnings: HashMap<u32, Vec<String>> = HashMap::new();
    for plan in plans {
        warnings.entry(plan.item_id).or_default().extend(plan.notices.clone());
        for op in &mut plan.ops {
            let Op::Download { url, path, crop, .. } = op else { continue };
            if cancel.is_cancelled() { return (images, warnings); }
            on_progress(ApplyProgress { phase: ApplyPhase::Images, done, total, failed, code: Some(plan.code.clone()) });
            let candidates = plan.image_candidates.get(path).cloned().unwrap_or_else(|| vec![ImageCandidate { url: url.clone(), crop: *crop }]);
            let is_poster = crate::assets::is_poster(path);
            let mut valid = vec![];
            for candidate in candidates {
                if cancel.is_cancelled() { return (images, warnings); }
                if !images.contains(&candidate.url) {
                    let result = if candidate.url.trim().is_empty() { Err("候选 URL 为空".into()) } else {
                        let response = tokio::select! { _ = cancel.cancelled() => return (images, warnings), response = http.get_image_bytes(&candidate.url, crate::net::IMAGE_RESPONSE_LIMIT) => response };
                        match response {
                            Ok((200, bytes)) => Ok(bytes),
                            Ok((status, _)) => Err(format!("HTTP {status}")),
                            Err(e) => Err(e.to_string()),
                        }
                    };
                    // Normalization, hashing and spill I/O stay off the async runtime.
                    let url = candidate.url.clone();
                    images = tokio::task::spawn_blocking(move || {
                        let prepared = result.and_then(|bytes| crate::artwork::PreparedImage::normalize(&bytes)
                            .map_err(|e| format!("无效图片: {e}")));
                        images.insert(url, prepared);
                        images
                    }).await.unwrap_or_else(|error| {
                        warnings.entry(plan.item_id).or_default().push(format!("图片准备失败: {error}"));
                        images::PreparedImages::new(memory_budget)
                    });
                }
                match images.info(&candidate.url) {
                    Ok(((w,h), _)) => {
                        valid.push((candidate, w, h));
                        if !is_poster { break; }
                    }
                    Err(error) => {
                        let message = format!("{} 候选 {}: {error}", path.display(), candidate.url);
                        log::warn!(target: "kiroku::organize", "{message}");
                        warnings.entry(plan.item_id).or_default().push(message);
                    }
                }
            }
            let dims = valid.iter().map(|(c,w,h)|(c.crop.is_some(),*w,*h)).collect::<Vec<_>>();
            if let Some(index) = if is_poster {crate::artwork::preferred(&dims)} else {(!valid.is_empty()).then_some(0)} {
                let chosen = &valid[index].0;
                *url = chosen.url.clone(); *crop = chosen.crop;
                images.select(url);
                log::info!(target: "kiroku::organize", "图片 {} 使用 {}", path.display(), url);
            } else { failed += 1; warnings.entry(plan.item_id).or_default().push(format!("{}: 无有效图片候选", path.display())); }
            done += 1;
            on_progress(ApplyProgress { phase: ApplyPhase::Images, done, total, failed, code: Some(plan.code.clone()) });
        }
        // Imports retain the actual unprocessed original. Repair deliberately never rewrites metadata.
        let selected = plan.ops.iter().find_map(|op| match op {Op::Download{path,url,..} if crate::assets::is_poster(path) => images.info(url).ok().map(|(_,hash)|(path.parent().unwrap().to_path_buf(),url.clone(),hash.to_owned())),_=>None});
        if let Some((dir,url,hash))=selected {
            if let Some(Op::WriteFile{content,..})=plan.ops.iter_mut().find(|op|matches!(op,Op::WriteFile{path,..} if path.file_name().is_some_and(|n|n==organize::SIDECAR))) {
                if let Ok(mut meta)=serde_json::from_str::<Metadata>(content){
                    let original=crate::artwork::original_name(&hash);
                    meta.artwork=Some(crate::artwork::Artwork{original:original.clone(),digest:hash,source_url:Some(url.clone()),crop:None,phase:meta.artwork_phase.clone().unwrap_or_default()});
                    *content=serde_json::to_string_pretty(&meta).expect("metadata");
                    plan.ops.push(Op::Download{url,path:dir.join(original),crop:None,stamp:None});
                }
            }
        }
    }
    (images, warnings)
}

/// Compatibility facade for callers that only need prefetched bytes.
pub async fn prefetch_images(http: &Http, plans: &[&PlanItem], cancel: &CancellationToken) -> HashMap<String, Vec<u8>> {
    let images = prepare_images(http, &mut plans.iter().map(|p| (*p).clone()).collect::<Vec<_>>(), cancel, &|_| {}, images::IMAGE_MEMORY_BUDGET).await.0;
    tokio::task::spawn_blocking(move || images.into_legacy()).await.unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyEvent {
    pub code: String,
    pub item_id: u32,
    pub done: usize,
    pub total: usize,
    pub ok: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedItem {
    pub item_id: u32,
    pub code: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug)]
pub struct ApplyOutcome {
    pub journal: Journal,
    pub applied: Vec<AppliedItem>,
    /// (番号, 原因)
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
    /// 移到废纸篓的原文件夹
    pub cleaned: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// 整理后清理原文件夹
// ---------------------------------------------------------------------------

/// 目录(含子目录)里是否还有正片视频;样片/预告、黑名单、过小的视频不算(与扫描同一套规则),不阻止清理。
fn has_main_video(dir: &Path, filter: &VideoFilter) -> bool {
    WalkDir::new(dir).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()).any(|e| {
        let name = e.file_name().to_string_lossy();
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        VIDEO_EXT.contains(&ext.as_str()) && filter.skip(&name, std::fs::metadata(e.path()).ok().map(|m| m.len())).is_none()
    })
}

/// 把整理后空出来的原文件夹交给 `trash`(通常是移到系统废纸篓)。
/// 只处理位于某个待整理根目录**之内**的文件夹(根目录本身永远保留);
/// 里面还剩正片视频的不动;清掉子目录后若上级也空了,继续向上直到根目录。
pub fn cleanup_sources(moved_from: &[PathBuf], roots: &[PathBuf], filter: &VideoFilter, trash: &dyn Fn(&Path) -> io::Result<()>) -> Vec<PathBuf> {
    let inside_root = |d: &Path| roots.iter().any(|r| d != r.as_path() && d.starts_with(r));
    let mut dirs: Vec<PathBuf> = moved_from.iter().filter_map(|f| f.parent().map(Path::to_path_buf)).collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    let mut trashed = vec![];
    let mut seen = HashSet::new();
    for d in dirs {
        let mut cur = Some(d);
        while let Some(dir) = cur {
            if !seen.insert(dir.clone()) || !inside_root(&dir) || !dir.is_dir() || has_main_video(&dir, filter) {
                break;
            }
            if trash(&dir).is_err() {
                break;
            }
            trashed.push(dir.clone());
            cur = dir.parent().map(Path::to_path_buf);
        }
    }
    // 只报告最外层(内层随外层一起进了废纸篓)
    let all = trashed.clone();
    trashed.retain(|d| !all.iter().any(|o| o != d && d.starts_with(o)));
    trashed
}

/// 移到系统废纸篓。macOS 用 NSFileManager,不需要控制 Finder 的权限。
pub fn trash_to_system(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path).map_err(io::Error::other)
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(io::Error::other)
    }
}

/// 逐项执行计划。每项完成后立即把日志落盘,所以崩溃/取消后也能回滚已完成的部分。
/// 「移到废纸篓」的实现;正式用 [`trash_to_system`],测试可换成直接删除。
pub type TrashFn = fn(&Path) -> io::Result<()>;

/// 整理后清理原文件夹的参数。
pub struct Cleanup<'a> {
    /// 待整理根目录:只清理其中的子文件夹,根目录本身保留
    pub roots: &'a [PathBuf],
    pub trash: TrashFn,
    /// 哪些视频不算正片(不阻止清理)
    pub filter: &'a VideoFilter,
}

/// Recheck each artwork stem independently, including dangling symlinks.
fn check_artwork_target(path: &Path) -> io::Result<()> {
    for ext in ["jpg", "jpeg", "png", "webp"] {
        let alternative = path.with_extension(ext);
        if organize::occupied(&alternative) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("图片已被占用，保留原素材: {}", alternative.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
fn execute_artwork_repair(journal: &mut Journal, plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>, cancel: &dyn Fn() -> bool) -> Exec {
    execute_artwork_repair_images(journal, plan, &mut |url| fetch(url).map(crate::artwork::ImageInput::Unchecked), cancel)
}

fn execute_artwork_repair_images(journal: &mut Journal, plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<crate::artwork::ImageInput>, cancel: &dyn Fn() -> bool) -> Exec {
    let mut out = Exec::default();
    for op in &plan.ops {
        if cancel() { break; }
        if let Err(error) = check_artwork_target(op.target()) { out.error = Some(error); break; }
        // Only this operation's stem is guarded. Already committed images are not conflicts.
        let part = journal.execute_images_observed(&Plan { ops: vec![op.clone()] }, fetch, cancel, &mut |point| {
            if point == crate::journal::Checkpoint::Staged { check_artwork_target(op.target()) } else { Ok(()) }
        });
        out.done.extend(part.done); out.warnings.extend(part.warnings);
        if part.error.is_some() { out.error = part.error; break; }
    }
    out
}

/// `cleanup` 给出时,整理完把这些待整理根目录之内空出来的原文件夹交给 `TrashFn`。
pub async fn apply(
    plans: Vec<PlanItem>,
    http: &Http,
    root: &Path,
    cleanup: Option<Cleanup<'_>>,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ApplyEvent) + Send + Sync),
) -> ApplyOutcome {
    apply_with_progress(plans, http, root, cleanup, cancel, on_event, &|_| {}).await
}

/// Same durable apply path with preparation progress for desktop and CLI.
pub async fn apply_with_progress(
    plans: Vec<PlanItem>,
    http: &Http,
    root: &Path,
    cleanup: Option<Cleanup<'_>>,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ApplyEvent) + Send + Sync),
    on_progress: &(dyn Fn(ApplyProgress) + Send + Sync),
) -> ApplyOutcome {
    apply_with_image_budget(plans, http, root, cleanup, cancel, on_event, on_progress, images::IMAGE_MEMORY_BUDGET).await
}

/// Keep one owned history across the worker boundary. Retaining an Arc here
/// also recovers progress if a worker panics or is cancelled before starting.
async fn with_blocking_journal<T: Send + 'static>(
    journal: Journal,
    work: impl FnOnce(&mut Journal) -> T + Send + 'static,
) -> (Journal, io::Result<T>) {
    let shared = Arc::new(std::sync::Mutex::new(journal));
    let worker = Arc::clone(&shared);
    let result = tokio::task::spawn_blocking(move || {
        let mut journal = worker.lock().unwrap_or_else(|e| e.into_inner());
        work(&mut journal)
    }).await.map_err(|e| io::Error::other(e.to_string()));
    // Await completion before reclaiming ownership; no worker can still hold
    // its Arc or lock. A poisoned mutex still contains the recorded history.
    let journal = Arc::try_unwrap(shared).expect("journal worker completed")
        .into_inner().unwrap_or_else(|e| e.into_inner());
    (journal, result)
}

async fn apply_with_image_budget(
    mut plans: Vec<PlanItem>,
    http: &Http,
    root: &Path,
    cleanup: Option<Cleanup<'_>>,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(ApplyEvent) + Send + Sync),
    on_progress: &(dyn Fn(ApplyProgress) + Send + Sync),
    memory_budget: usize,
) -> ApplyOutcome {
    // A rejected naming preview cannot acquire files or even create a journal on disk.
    if plans.iter().any(|p| p.conflicts.iter().any(|c|c.kind==Conflict::InvalidTemplate)) {
        let failed=plans.iter().map(|p|(p.code.clone(),"目录或文件模板无效，请重新预览".into())).collect();
        return ApplyOutcome{journal:Journal::new(root),applied:vec![],failed,cancelled:false,cleaned:vec![]};
    }
    // Delay initial group metadata until all retained media records can be written together.
    let last:HashMap<String,usize>=plans.iter().enumerate().filter(|(_,p)|p.ops.iter().any(Op::retained)).map(|(i,p)|(p.code.clone(),i)).collect();
    for i in 0..plans.len(){if let Some(&end)=last.get(&plans[i].code){if end!=i && plans[i].append_preview.is_none(){let mut media=vec![];let mut metadata=vec![];for op in std::mem::take(&mut plans[i].ops){if op.source().is_some(){media.push(op)}else{metadata.push(op)}}plans[i].ops=media;plans[end].ops.extend(metadata);}}}
    let retains_sources=plans.iter().flat_map(|p|&p.ops).any(Op::retained);
    let cleanup=cleanup.filter(|_|!retains_sources);
    let root_guards:Vec<_>=plans.iter().filter_map(|p|p.root_guard.clone()).collect();
    let save_journal=|journal:&Journal|->io::Result<PathBuf>{for guard in &root_guards{guard.verify()?;}journal.save()};
    log::info!(target: "kiroku::organize", "开始整理 {} 项 → {}{}", plans.len(), root.display(), if cleanup.is_some() { ",完成后清理原文件夹" } else { "" });
    if let Some(error)=plans.iter().filter_map(|p|p.root_guard.as_ref()).find_map(|g|g.verify().err()){
        let mut journal=Journal::new(root);journal.status=JournalStatus::Partial;
        let failed=plans.iter().map(|p|(p.code.clone(),format!("媒体库不可用，未执行：{error}"))).collect();
        for (i,p) in plans.iter().enumerate(){on_event(ApplyEvent{code:p.code.clone(),item_id:p.item_id,done:i+1,total:plans.len(),ok:false,error:Some(error.to_string()),warnings:vec![]});}
        return ApplyOutcome{journal,applied:vec![],failed,cancelled:false,cleaned:vec![]};
    }
    let (images, mut image_warnings) = prepare_images(http, &mut plans, cancel, on_progress, memory_budget).await;
    let images = Arc::new(images);
    let guards=root_guards.clone();
    let initialized=tokio::task::spawn_blocking(move||guards.iter().try_for_each(RootGuard::initialize)).await.map_err(|error|io::Error::other(error.to_string())).and_then(|result|result);
    if let Err(error)=initialized {
        let mut journal=Journal::new(root);journal.status=JournalStatus::Partial;
        let failed=plans.iter().map(|plan|(plan.code.clone(),format!("媒体库不可用，未执行：{error}"))).collect();
        for (index,plan)in plans.iter().enumerate(){on_event(ApplyEvent{code:plan.code.clone(),item_id:plan.item_id,done:index+1,total:plans.len(),ok:false,error:Some(error.to_string()),warnings:vec![]});}
        let _=tokio::task::spawn_blocking(move||drop(images)).await;
        return ApplyOutcome{journal,applied:vec![],failed,cancelled:cancel.is_cancelled(),cleaned:vec![]};
    }
    let total = plans.len();
    if !cancel.is_cancelled() {
        on_progress(ApplyProgress { phase: ApplyPhase::Organize, done: 0, total, failed: 0, code: None });
    }
    let mut journal = Journal::new(root);
    let (mut applied, mut failed, mut cancelled) = (vec![], vec![], false);

    for (i, p) in plans.into_iter().enumerate() {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let (imgs, c) = (images.clone(), cancel.clone());
        log::info!(target: "kiroku::organize", "[{}/{total}] {}", i + 1, p.code);
        let mut plan = Plan { ops: p.ops.clone() };

        if !journal.items.iter().any(|j| j.code == p.code) {
            if let Some(dir) = p.to.parent() { journal.items.push(JournalItem { code: p.code.clone(), dir: dir.to_path_buf() }); }
        }
        let mut append_guard = p.append_preview.clone();
        let root_guard=p.root_guard.clone();
        let (returned_journal, result) = with_blocking_journal(journal, move |journal_bg| {
            let mut replay = images::PlanReplay::new(&imgs, memory_budget);
            let mut fetch = |u: &str| replay.load(u).map(crate::artwork::ImageInput::Prepared);
            let manifest_validation=append_guard.as_mut().map(|g|g.accept_manifest_updates(journal_bg)).transpose();
            let validation = manifest_validation.and_then(|_|root_guard.as_ref().map(|g|g.verify()).transpose().and_then(|_|append_guard.as_ref().map(|g|g.validate_files_cancellable(&c)).transpose()));
            let validation=validation.and_then(|_|crate::placement::bind_plan_checked(&mut plan,journal_bg,&||c.is_cancelled()))
                .and_then(|_|root_guard.as_ref().map(|g|g.verify()).transpose().map(|_|()));
            let ex = if let Err(e) = validation { Exec { error: Some(e), ..Default::default() } }
            else if plan.ops.iter().all(|op| matches!(op, Op::Download { .. })) {
                execute_artwork_repair_images(journal_bg, &plan, &mut fetch, &|| c.is_cancelled())
            } else { crate::placement::execute_images(journal_bg,&plan, &mut fetch, &|| c.is_cancelled()) };
            ex
        }).await;
        journal = returned_journal;
        let ex = match result {
            Ok(ex) => ex,
            Err(e) => {
                if let Ok(j) = Journal::load(&journal.path()) { journal = j; }
                Exec { error: Some(io::Error::other(e.to_string())), ..Default::default() }
            }
        };

        let moved = ex.done.iter().any(|o|o.source().is_some());
        if moved {
            applied.push(AppliedItem { item_id: p.item_id, code: p.code.clone(), from: p.from.clone(), to: p.to.clone() });
            if let Some(dir) = p.to.parent().filter(|_| !journal.items.iter().any(|j| j.code == p.code)) {
                journal.items.push(JournalItem { code: p.code.clone(), dir: dir.to_path_buf() });
            }
        }
        let halt = ex.error.is_some();
        let image_only = p.ops.iter().all(|op| matches!(op, Op::Download { .. }));
        let incomplete_images = image_only && ex.done.len() < p.ops.len() && !cancel.is_cancelled();
        let error = ex.error.filter(|e| !(cancel.is_cancelled() && e.kind()==io::ErrorKind::Interrupted)).map(|e| e.to_string()).or_else(|| incomplete_images.then(|| "图片未全部修复：无有效候选或下载失败".into()));
        if let Some(e) = &error {
            failed.push((p.code.clone(), e.clone()));
        }
        if cancel.is_cancelled() && error.is_none() && !moved {
            cancelled = true;
            log::info!(target: "kiroku::organize", "  已取消");
        }
        on_event(ApplyEvent {
            code: p.code,
            item_id: p.item_id,
            done: i + 1,
            total,
            ok: error.is_none() && !cancelled,
            error,
            warnings: image_warnings.remove(&p.item_id).unwrap_or_default().into_iter().chain(ex.warnings).collect(),
        });
        if halt { break; }
    }
    // Remove job scratch files on a blocking worker, including cancellation/failure.
    let _ = tokio::task::spawn_blocking(move || drop(images)).await;
    cancelled |= cancel.is_cancelled();
    journal.status = if cancelled || !failed.is_empty() || journal.pending.is_some() { JournalStatus::Partial } else { JournalStatus::Applied };
    let mut cleaned = vec![];
    match save_journal(&journal) {
        Ok(path) => log::debug!(target: "kiroku::organize", "整理日志已保存 {}", path.display()),
        Err(e) => {
            log::error!(target: "kiroku::organize", "整理日志保存失败: {e}");
            journal.status = JournalStatus::Partial;
            if !failed.iter().any(|(code,_)| code == "整理日志") { failed.push(("整理日志".into(), e.to_string())); }
        }
    }
    if cleanup.is_some() && !(!applied.is_empty() && failed.is_empty() && !cancelled && journal.pending.is_none()) {
        log::info!(target: "kiroku::organize", "本批有失败、取消或没有移动任何文件,不清理原文件夹");
    }
    if let Some(Cleanup { roots, trash, filter }) = cleanup.filter(|_| !applied.is_empty() && failed.is_empty() && !cancelled && journal.pending.is_none()) {
        let (froms, roots, filter) = (applied.iter().map(|a| a.from.clone()).collect::<Vec<_>>(), roots.to_vec(), filter.clone());
        let cleanup_cancel = cancel.clone();
        let protected_library = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        cleaned = tokio::task::spawn_blocking(move || cleanup_sources(&froms, &roots, &filter, &|path| {
            let candidate = path.canonicalize()?;
            if protected_library.starts_with(&candidate) { return Err(io::Error::other("媒体库及其上级目录禁止清理")); }
            if cleanup_cancel.is_cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "清理已取消")); }
            trash(path)
        })).await.unwrap_or_default();
        journal.trashed = cleaned.clone();
        for d in &cleaned {
            log::info!(target: "kiroku::organize", "原文件夹已移到废纸篓: {}", d.display());
        }
    }
    cancelled |= cancel.is_cancelled();
    if cancelled { journal.status = JournalStatus::Partial; }
    if let Err(e) = save_journal(&journal) {
        journal.status = JournalStatus::Partial;
        if !failed.iter().any(|(code,_)| code == "整理日志") { failed.push(("整理日志".into(), e.to_string())); }
    }
    log::info!(target: "kiroku::organize", "整理结束:成功 {} 项,失败 {} 项{}", applied.len(), failed.len(), if cancelled { ",已取消" } else { "" });
    ApplyOutcome { journal, applied, failed, cancelled, cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        code::{parse_code, parse_file_info},
        metadata::{Actor, Gender, PartialMetadata},
    };
    use std::{fs, time::Duration};

    #[tokio::test]
    async fn blocking_journal_keeps_ownership_and_recovers_completed_progress_on_panic() {
        let root = tempfile::tempdir().unwrap();
        let mut j = Journal::new(root.path());
        j.push_ops([Op::MoveFile { from: root.path().join("a"), to: root.path().join("b") }]);
        let address = j.ops.as_ptr() as usize;
        let (j, result) = with_blocking_journal(j, move |j| {
            assert_eq!(j.ops.as_ptr() as usize, address, "the entire operation history must not be cloned for each item");
            j.push_ops([Op::MoveFile { from: j.root.join("c"), to: j.root.join("d") }]);
            panic!("worker failed after recording progress");
        }).await;
        assert!(result.is_err());
        assert_eq!(j.ops.len(), 2, "in-memory progress survives a failed worker even before the first save");
        assert_eq!(j.ops[1].target(), root.path().join("d"));
    }

    fn meta(code: &str) -> Metadata {
        Metadata {
            code: code.into(),
            fields: PartialMetadata {
                title: Some(format!("{code} title")),
                actors: vec![Actor::new("Foo", Gender::Female)],
                release_date: Some("2024-01-02".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn batch_plans_enumerate_each_source_directory_once_and_match_individual_plans() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let root = temp.path().join("library");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&root).unwrap();
        let mut items = vec![];
        let mut metas = HashMap::new();
        for i in 0..20 {
            let name = format!("ABC-{i:05}.mp4");
            let path = source.join(&name);
            fs::write(&path, b"v").unwrap();
            fs::write(source.join(format!("ABC-{i:05}.chs.srt")), b"s").unwrap();
            let code = parse_code(&name).unwrap();
            metas.insert(code.id.clone(), meta(&code.id));
            items.push(ScanItem { id: i, path, size: 1, info: parse_file_info(&name), code: Some(code) });
        }
        let pairs: Vec<_> = items.iter().map(|item| (item, item.code.as_ref().unwrap())).collect();
        let (plans, reads) = crate::subtitle::directory_reads::measure(|| build_plans(&pairs, &metas, &root, "{code}", PlanOpts::default()));
        assert_eq!(reads, 1, "a batch must capture its source directory once, regardless of media count");
        for (plan, (item, code)) in plans.iter().zip(&pairs) {
            let individual = organize::plan_with(&item.path, code, &metas[&code.id], &root, "{code}", true, PlanOpts::default());
            assert_eq!(plan.ops, individual.ops, "batch snapshots preserve standalone naming and subtitle operations");
        }
        fs::write(source.join("ABC-00000.en.srt"), b"new subtitle").unwrap();
        let refreshed = build_plans(&pairs, &metas, &root, "{code}", PlanOpts::default());
        assert!(refreshed[0].ops.iter().any(|op| op.source().is_some_and(|p| p.ends_with("ABC-00000.en.srt"))), "a new batch must capture fresh disk state");
    }

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"v").unwrap();
    }

    fn scanned(roots: &[PathBuf], exclude: Option<&Path>) -> ScanResult {
        scan(roots, exclude, &VideoFilter::default(), &CancellationToken::new(), &mut |_| {})
    }

    #[test]
    fn scan_flags_unrecognized_skips_library_and_junk() {
        let d = tempfile::tempdir().unwrap();
        let (inp, lib) = (d.path().join("in"), d.path().join("in/lib"));
        touch(&inp.join("SMPL-001.mp4"));
        touch(&inp.join("noise.mp4"));
        touch(&inp.join("readme.txt"));
        touch(&inp.join("._SMPL-002.mp4"));
        touch(&lib.join("ABC-123/ABC-123.mp4")); // 库内文件不应被扫到
        let r = scanned(&[inp.clone(), inp.clone()], Some(&lib)); // 重叠根目录也不应重复
        assert_eq!(r.items.len(), 2);
        assert_eq!((r.codes, r.unrecognized), (1, 1));
        assert!(r.items.iter().any(|i| i.code.as_ref().is_some_and(|c| c.id == "SMPL-001")));
        assert!(r.items.iter().any(|i| i.code.is_none() && i.path.ends_with("noise.mp4")));
    }

    /// 复现:同一文件夹里的样片、预告片、广告都会解析成该文件夹的番号,样片先被扫到时会抢走正片的位置。
    #[test]
    fn scan_skips_extras_blacklist_and_small_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("in/SMPL-001");
        let write = |n: &str, bytes: usize| { fs::create_dir_all(&dir).unwrap(); fs::write(dir.join(n), vec![0u8; bytes]).unwrap() };
        write("SMPL-001-C.mp4", 4096);
        write("SMPL-001-trailer.mp4", 4096);
        write("SMPL-001 sample.mp4", 4096);
        write("最新地址.mp4", 4096);
        write("tiny.mp4", 10);
        let root = [d.path().join("in")];
        let names = |r: &ScanResult| { let mut v: Vec<_> = r.items.iter().map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned()).collect(); v.sort(); v };
        // 默认:只跳过样片/预告片
        let r = scanned(&root, None);
        assert_eq!(names(&r), ["SMPL-001-C.mp4", "tiny.mp4", "最新地址.mp4"]);
        assert_eq!(r.skipped.len(), 2);
        assert!(r.skipped.iter().all(|s| s.reason == SkipReason::Extra && s.size == 4096));
        // 黑名单(广告)
        let cfg = crate::video_filter::ScanCfg { blacklist: vec!["最新地址".into(), "^tiny".into()], ..Default::default() };
        let r = scan(&root, None, &VideoFilter::new(&cfg), &CancellationToken::new(), &mut |_| {});
        assert_eq!(names(&r), ["SMPL-001-C.mp4"]);
        assert_eq!(r.skipped.iter().filter(|s| s.reason == SkipReason::Blacklisted).count(), 2);
        // 体积阈值
        let cfg = crate::video_filter::ScanCfg { min_video_mb: 1, ..Default::default() };
        let r = scan(&root, None, &VideoFilter::new(&cfg), &CancellationToken::new(), &mut |_| {});
        assert!(r.items.is_empty() && r.skipped.iter().filter(|s| s.reason == SkipReason::TooSmall).count() == 3);
        // 关闭全部规则时与旧行为一致
        assert_eq!(scan(&root, None, &VideoFilter::none(), &CancellationToken::new(), &mut |_| {}).items.len(), 5);
    }

    #[test]
    fn largest_file_wins_a_contested_target() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let item = |id: u32, name: &str, size: u64| ScanItem { id, path: d.path().join(name), size, info: parse_file_info(name), code: parse_code(name) };
        // 漏网的样片先于正片出现、且更小
        let items = [item(0, "SMPL-001 clip.mp4", 10), item(1, "SMPL-001.mp4", 5_000)];
        let pairs: Vec<_> = items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect();
        let metas = HashMap::from([("SMPL-001".to_string(), meta("SMPL-001"))]);
        let plans = build_plans(&pairs, &metas, &lib, "{code}", PlanOpts::default());
        assert_eq!(plans.iter().map(|p| p.item_id).collect::<Vec<_>>(), [0, 1], "输出保持原顺序");
        let (clip, main) = (&plans[0], &plans[1]);
        assert!(main.selected && main.conflict.is_none(), "大文件拿到目标");
        assert!(main.extras.iter().any(|e| e.ends_with(".nfo")), "元数据也跟着大文件");
        assert_eq!((clip.selected, clip.conflict), (false, Some(Conflict::DuplicateInBatch)));
    }

    #[test]
    fn a_shared_subtitle_goes_to_one_video_only() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("in");
        fs::create_dir_all(&dir).unwrap();
        for f in ["SMPL-001-C.mp4", "SMPL-001-UC.mp4", "SMPL-001.chs.srt"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        let item = |id: u32, name: &str, size: u64| ScanItem { id, path: dir.join(name), size, info: parse_file_info(name), code: parse_code(name) };
        let items = [item(0, "SMPL-001-C.mp4", 10), item(1, "SMPL-001-UC.mp4", 5_000)];
        let pairs: Vec<_> = items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect();
        let plans = build_plans(&pairs, &HashMap::from([("SMPL-001".to_string(), meta("SMPL-001"))]), &{let lib=d.path().join("lib");fs::create_dir_all(&lib).unwrap();lib}, "{code}", PlanOpts::default());
        let subs = |p: &PlanItem| p.ops.iter().filter_map(|o| match o {
            Op::MoveFile { to, .. } if to.extension().is_some_and(|e| e == "srt") => Some(to.file_name().unwrap().to_string_lossy().into_owned()),
            _ => None,
        }).collect::<Vec<_>>();
        assert!(plans.iter().all(|p| p.selected && p.conflict.is_none()), "两个版本都能整理");
        assert!(subs(&plans[0]).is_empty());
        assert_eq!(subs(&plans[1]), ["SMPL-001-UC.chs.srt"], "字幕归体积大的版本");
    }

    #[test]
    fn scan_ignores_code_like_names_above_the_root() {
        let d = tempfile::tempdir().unwrap();
        // 根目录及其上级都长得像番号,但文件本身没有
        let root = d.path().join("claude-501/Season-01");
        touch(&root.join("noise.mp4"));
        touch(&root.join("MOVX-123/video.mp4")); // 根目录以下的目录名仍可补番号
        let r = scanned(&[root.clone()], None);
        let by = |n: &str| r.items.iter().find(|i| i.path.ends_with(n)).unwrap();
        assert!(by("noise.mp4").code.is_none(), "不应从根目录以上的目录名推出番号");
        assert_eq!(by("video.mp4").code.as_ref().map(|c| c.id.as_str()), Some("MOVX-123"));
    }

    #[test]
    fn inspect_dir_counts_videos_and_nfos_and_skips_index_dir() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.mp4"));
        touch(&d.path().join("sub/b.MKV"));
        touch(&d.path().join("sub/movie.nfo"));
        touch(&d.path().join("._junk.mp4"));
        touch(&d.path().join(".kiroku/x.mp4")); // 索引目录不算
        touch(&d.path().join("notes.txt"));
        let i = inspect_dir(d.path());
        assert_eq!((i.videos, i.nfos, i.truncated), (2, 1, false));
        assert_eq!(inspect_dir(&d.path().join("missing")).videos, 0);
    }

    #[test]
    fn override_code_rescues_unrecognized_files_and_keeps_part() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("random movie-CD2.mp4"));
        let mut r = scanned(&[d.path().to_path_buf()], None);
        let item = &mut r.items[0];
        assert!(item.code.is_none());
        override_code(item, " smpl-001 ").unwrap();
        let c = item.code.as_ref().unwrap();
        assert_eq!((c.id.as_str(), c.part), ("SMPL-001", Some(2)), "分集来自文件名,番号来自手动输入");
        assert!(item.info.matched);
        // 能改错:换成 FC2
        override_code(item, "FC2-PPV-1234567").unwrap();
        assert_eq!(item.code.as_ref().unwrap().id, "FC2-1234567");
        // 无法识别 / 空输入:报错且不改动
        assert!(override_code(item, "不是番号").is_err());
        assert!(override_code(item, "  ").is_err());
        assert_eq!(item.code.as_ref().unwrap().id, "FC2-1234567");
    }

    #[tokio::test]
    async fn new_library_plan_rejects_a_root_created_by_another_owner_before_apply() {
        let temporary=tempfile::tempdir().unwrap();let input=temporary.path().join("input");fs::create_dir(&input).unwrap();touch(&input.join("ABC-123.mp4"));let root=temporary.path().join("new-library");
        let scan=scanned(&[input],None);let pairs=scan.items.iter().map(|item|(item,item.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans=build_plans(&pairs,&HashMap::from([("ABC-123".into(),meta("ABC-123"))]),&root,"{code}",PlanOpts{create_library:true,..Default::default()});assert!(plans[0].selected);
        fs::create_dir_all(root.join("different-layout")).unwrap();fs::write(root.join("different-layout/kiroku.json"),serde_json::to_vec(&meta("ABC-123")).unwrap()).unwrap();
        let outcome=apply(plans,&http(),&root,None,&CancellationToken::new(),&|_|{}).await;
        assert!(outcome.applied.is_empty());assert!(!outcome.failed.is_empty());assert!(scan.items[0].path.exists());assert!(!root.join("ABC-123").exists());assert!(!root.join(".kiroku").exists());
    }

    #[test]
    fn full_library_batch_discovers_once_and_keeps_append_and_conflict_semantics() {
        let temporary=tempfile::tempdir().unwrap();let root=temporary.path().join("library");let source=temporary.path().join("source");fs::create_dir(&root).unwrap();fs::create_dir(&source).unwrap();
        for i in 0..100 {
            let code=format!("ABC-{i:05}");let directory=root.join(&code);fs::create_dir(&directory).unwrap();
            fs::write(directory.join("kiroku.json"),serde_json::to_vec(&meta(&code)).unwrap()).unwrap();fs::write(directory.join(format!("{code}-CD1.mp4")),b"old").unwrap();
            if i<20{fs::write(source.join(format!("{code}-CD2.mp4")),b"new").unwrap();}
        }
        let scan=scanned(&[source],None);let pairs=scan.items.iter().map(|item|(item,item.code.as_ref().unwrap())).collect::<Vec<_>>();let metas=pairs.iter().map(|(_,code)|(code.id.clone(),meta(&code.id))).collect();
        let (reference,old_counts)=crate::append::discovery_stats::measure(||pairs.iter().flat_map(|pair|build_plans(std::slice::from_ref(pair),&metas,&root,"{code}",PlanOpts::default())).collect::<Vec<_>>());
        let (plans,counts)=crate::append::discovery_stats::measure(||build_plans(&pairs,&metas,&root,"{code}",PlanOpts::default()));
        assert_eq!(old_counts.0,20);assert_eq!(counts.0,1);assert_eq!(counts.2,100);
        assert_eq!(plans.len(),reference.len());for(plan,previous)in plans.iter().zip(&reference){assert_eq!(plan.ops,previous.ops);assert_eq!(serde_json::to_value(&plan.conflicts).unwrap(),serde_json::to_value(&previous.conflicts).unwrap());assert_eq!(plan.selected,previous.selected);assert!(plan.append_preview.is_some());}
        let extra=root.join("duplicate");fs::create_dir(&extra).unwrap();fs::write(extra.join("kiroku.json"),serde_json::to_vec(&meta(&pairs[0].1.id)).unwrap()).unwrap();fs::write(extra.join("movie.mp4"),b"duplicate").unwrap();
        let refreshed=build_plans(&pairs,&metas,&root,"{code}",PlanOpts::default());assert!(!refreshed[0].selected);assert!(refreshed[0].notices.iter().any(|notice|notice.contains("找到 2 个")));
        let cancelled=CancellationToken::new();cancelled.cancel();assert_eq!(build_plans_cancellable(&pairs,&metas,&root,"{code}",PlanOpts::default(),&cancelled).unwrap_err().kind(),io::ErrorKind::Interrupted);
        fs::write(root.join("ABC-00099/kiroku.json"),b"bad metadata").unwrap();assert!(build_plans(&pairs,&metas,&root,"{code}",PlanOpts::default()).iter().all(|plan|plan.conflict==Some(Conflict::InvalidLibrary)&&!plan.selected));
    }

    #[test]
    fn build_plans_detects_conflicts_and_places_meta_once() {
        let d = tempfile::tempdir().unwrap();
        let (inp, lib) = (d.path().join("in"), d.path().join("lib"));
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("ABC-123-CD2.mp4")); // CD1 缺席:元数据仍要写
        touch(&inp.join("abc-123-cd3.mp4"));
        touch(&inp.join("XYZ-999.mp4"));
        touch(&inp.join("sub/XYZ-999.mp4")); // 同码同名 → 批内重名
        touch(&lib.join("Foo/EXI-001/EXI-001.mp4")); // 目标已存在
        touch(&inp.join("EXI-001.mp4"));
        let r = scanned(&[inp.clone()], Some(&lib));
        let pairs: Vec<(&ScanItem, &Code)> = r.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
        let metas: HashMap<_, _> = ["ABC-123", "XYZ-999", "EXI-001"].iter().map(|c| (c.to_string(), meta(c))).collect();
        let plans = build_plans(&pairs, &metas, &lib, "{actor}/{code}", PlanOpts::default());
        assert_eq!(plans.len(), 5);

        let abc: Vec<_> = plans.iter().filter(|p| p.code == "ABC-123").collect();
        let with_meta: Vec<_> = abc.iter().filter(|p| p.extras.iter().any(|e| e.ends_with(".nfo"))).collect();
        assert_eq!(with_meta.len(), 1, "元数据只写一份");
        assert!(with_meta[0].from.ends_with("ABC-123-CD2.mp4"), "落在序号最小的分片上");
        assert!(abc.iter().all(|p| p.conflict.is_none() && p.selected));

        let xyz: Vec<_> = plans.iter().filter(|p| p.code == "XYZ-999").collect();
        assert_eq!(xyz.iter().filter(|p| p.conflict == Some(Conflict::DuplicateInBatch)).count(), 1);
        let exi = plans.iter().find(|p| p.code == "EXI-001").unwrap();
        assert_eq!(exi.conflict, Some(Conflict::TargetExists));
        assert!(!exi.selected);
    }

    #[test]
    fn plans_block_shared_metadata_conflicts_for_all_parts() {
        let d = tempfile::tempdir().unwrap();
        let inp = d.path().join("in");
        let lib = d.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("ABC-123-CD1.mp4"));
        touch(&inp.join("ABC-123-CD2.mp4"));
        touch(&lib.join("ABC-123/movie.nfo"));
        let r = scanned(&[inp], Some(&lib));
        let pairs: Vec<_> = r.items.iter().filter_map(|it| it.code.as_ref().map(|c| (it,c))).collect();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(),meta("ABC-123"))]), &lib, "{code}", PlanOpts::default());
        assert!(plans.iter().all(|p| p.conflict == Some(Conflict::InvalidLibrary) && !p.selected));
    }

    fn http() -> Http {
        Http::new(Duration::from_millis(1), None).unwrap()
    }

    #[test]
    fn scan_reports_missing_root_instead_of_successful_empty() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("disconnected");
        let result = scan(&[missing.clone()], None, &VideoFilter::default(), &CancellationToken::new(), &mut |_| {});
        let result = serde_json::to_value(result).unwrap();
        assert_eq!(result["complete"], false, "inaccessible sources must never look like a complete empty scan");
        assert_eq!(result["errorsTotal"], 1);
        assert_eq!(result["errors"][0]["path"], missing.to_string_lossy().as_ref());
    }

    #[tokio::test]
    async fn spilled_batch_keeps_originals_crops_progress_cancel_and_rollback() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let mut encoded = io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(80, 40).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        let normalized = crate::artwork::normalize(encoded.get_ref()).unwrap();
        for cancel_stage in ["never", "images", "organize"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/shared", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(AtomicUsize::new(0));
            let (stop_bg, requests_bg, body) = (stop.clone(), requests.clone(), encoded.get_ref().clone());
            let server = std::thread::spawn(move || {
                while !stop_bg.load(Ordering::Relaxed) {
                    if let Ok((mut stream, _)) = listener.accept() {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let mut req = [0; 2048]; stream.read(&mut req).unwrap();
                        requests_bg.fetch_add(1, Ordering::Relaxed);
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                        stream.write_all(&body).unwrap();
                    } else { std::thread::sleep(Duration::from_millis(2)); }
                }
            });
            let d = tempfile::tempdir().unwrap();
            let (input, root) = (d.path().join("input"), d.path().join("lib"));
            fs::create_dir_all(&root).unwrap();
            let mut metas = HashMap::new();
            for id in ["ABC-121", "ABC-122", "ABC-123"] {
                touch(&input.join(format!("{id}.mp4")));
                let mut metadata = meta(id);
                metadata.fields.thumb_urls = vec![url.clone()];
                metas.insert(id.into(), metadata);
            }
            let scan = scanned(&[input.clone()], None);
            let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
            let plans = build_plans(&pairs, &metas, &root, "{code}", PlanOpts::default());
            let expected = plans.iter().flat_map(|p| &p.ops).filter_map(|op| match op {
                Op::Download { path, crop, stamp, .. } => Some((path.clone(), organize::process_image(normalized.clone(), *crop, stamp.as_ref()).unwrap())),
                _ => None,
            }).collect::<Vec<_>>();
            let token = CancellationToken::new();
            let progress = std::sync::Mutex::new(vec![]);
            let out = apply_with_image_budget(plans, &http(), &root, None, &token,
                &|event| { if cancel_stage == "organize" && event.done == 1 { token.cancel(); } },
                &|event| {
                    if cancel_stage == "images" && event.phase == ApplyPhase::Images && event.done == event.total { token.cancel(); }
                    progress.lock().unwrap().push(event);
                }, 0).await;
            stop.store(true, Ordering::Relaxed); server.join().unwrap();
            assert!(out.failed.is_empty(), "{:?}", out.failed);
            assert_eq!(requests.load(Ordering::Relaxed), 1, "shared URLs stay deduplicated across disk batches");
            let progress = progress.into_inner().unwrap();
            let downloads = progress.iter().filter(|e| e.phase == ApplyPhase::Images).collect::<Vec<_>>();
            assert!(downloads.iter().all(|e| e.total == 6 && e.failed == 0));
            assert_eq!(downloads.last().unwrap().done, 6);
            assert_eq!(out.applied.len(), match cancel_stage { "never" => 3, "images" => 0, _ => 1 });
            assert_eq!(out.cancelled, cancel_stage != "never");
            for applied in &out.applied {
                let dir = applied.to.parent().unwrap();
                let metadata: Metadata = serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
                let original = metadata.artwork.unwrap();
                assert_eq!(fs::read(dir.join(original.original)).unwrap(), normalized);
                assert_eq!(original.digest, crate::journal::replacement::digest(&normalized));
                for (path, bytes) in expected.iter().filter(|(p, _)| p.parent() == Some(dir)) {
                    assert_eq!(fs::read(path).unwrap(), *bytes, "crop/stamp must stay byte-identical");
                }
            }
            // Reopen the durable journal after the ephemeral image cache is gone.
            let mut journal = Journal::load(&out.journal.path()).unwrap();
            assert!(journal.rollback().is_empty());
            for item in &scan.items { assert_eq!(fs::read(&item.path).unwrap(), b"v"); }
            assert!(expected.iter().all(|(path, _)| !path.exists()));
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_streamed_image_before_media_moves() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (started, received) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let mut request = [0; 4096]; stream.read(&mut request).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nx\r\n").unwrap();
            let _ = started.send(());
            let _ = released.recv_timeout(std::time::Duration::from_secs(3));
        });
        let d = tempfile::tempdir().unwrap(); let input = d.path().join("input"); let root = d.path().join("library");
        fs::create_dir(&input).unwrap(); fs::create_dir(&root).unwrap(); touch(&input.join("ABC-123.mp4"));
        let mut metadata = meta("ABC-123"); metadata.fields.thumb_urls = vec![format!("http://{address}/image")];
        let scan = scanned(&[input], None);
        let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), metadata)]), &root, "{code}", PlanOpts::default());
        let cancel = CancellationToken::new(); let http = http();
        let (result, _) = tokio::join!(
            tokio::time::timeout(std::time::Duration::from_secs(2), apply_with_image_budget(plans, &http, &root, None, &cancel, &|_| {}, &|_| {}, 0)),
            async { received.await.unwrap(); cancel.cancel(); }
        );
        release.send(()).unwrap(); server.join().unwrap();
        let mut out = result.expect("cancellation must not wait for response EOF");
        assert!(out.cancelled && out.applied.is_empty()); assert!(out.failed.is_empty());
        assert_eq!(fs::read(&scan.items[0].path).unwrap(), b"v");
        assert!(out.journal.rollback().is_empty()); assert!(!root.join("ABC-123").exists());
    }

    #[tokio::test]
    async fn oversized_candidates_use_fallback_and_keep_originals_and_rollback() {
        use std::io::{Read, Write};
        let mut small = io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(80, 40).write_to(&mut small, image::ImageFormat::Png).unwrap();
        let small = small.into_inner();
        let normalized = crate::artwork::normalize(&small).unwrap();
        let mut huge = io::Cursor::new(vec![]);
        image::DynamicImage::ImageLuma8(image::GrayImage::new(4097, 4097))
            .write_to(&mut huge, image::ImageFormat::Png).unwrap();
        let huge = huge.into_inner();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096]; let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                if request.contains(" /bytes ") {
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", crate::net::IMAGE_RESPONSE_LIMIT + 1);
                } else {
                    let bytes = if request.contains(" /pixels ") { &huge } else { &small };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                    let _ = stream.write_all(bytes);
                }
            }
        });
        let d = tempfile::tempdir().unwrap(); let input = d.path().join("input");
        fs::create_dir(&input).unwrap(); let root = d.path().join("library");
        fs::create_dir(&root).unwrap();
        touch(&input.join("ABC-123.mp4"));
        let mut metadata = meta("ABC-123");
        metadata.fields.thumb_urls = ["bytes", "pixels", "small"].map(|path| format!("http://{address}/{path}")).to_vec();
        let scan = scanned(&[input], None);
        let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), metadata)]), &root, "{code}", PlanOpts::default());
        let warnings = std::sync::Mutex::new(vec![]);
        let out = apply_with_image_budget(plans, &http(), &root, None, &CancellationToken::new(),
            &|event| warnings.lock().unwrap().extend(event.warnings), &|_| {}, 0).await;
        server.join().unwrap();
        assert!(out.failed.is_empty(), "{:?}", out.failed); assert_eq!(out.applied.len(), 1);
        let warnings = warnings.into_inner().unwrap().join("\n");
        assert!(warnings.contains("图片响应超过")); assert!(warnings.contains("图片像素"));
        let dir = out.applied[0].to.parent().unwrap();
        let metadata: Metadata = serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
        assert_eq!(fs::read(dir.join(metadata.artwork.unwrap().original)).unwrap(), normalized);
        let mut journal = Journal::load(&out.journal.path()).unwrap();
        assert!(journal.rollback().is_empty()); assert_eq!(fs::read(&scan.items[0].path).unwrap(), b"v");
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn prefetch_rejects_html_even_when_http_succeeds() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0; 2048];
            stream.read(&mut req).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n<html").unwrap();
        });
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in");
        touch(&input.join("ABC-123.mp4"));
        let scan = scanned(&[input], None);
        let mut m = meta("ABC-123");
        m.fields.poster_urls = vec![url.clone()];
        let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), m)]), d.path(), "{code}", PlanOpts::default());
        let images = prefetch_images(&http(), &plans.iter().collect::<Vec<_>>(), &CancellationToken::new()).await;
        server.join().unwrap();
        assert!(!images.contains_key(&url), "HTML must never be written as a poster");
    }

    #[tokio::test]
    async fn apply_falls_back_from_invalid_cover_and_records_successful_url() {
        use std::io::{Read, Write};
        let mut bytes = std::io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(4, 6).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_bg = stop.clone();
        let server = std::thread::spawn(move || {
            while !stop_bg.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    // macOS accepted sockets inherit O_NONBLOCK from the listener.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut req = [0; 2048];
                    let n = stream.read(&mut req).unwrap();
                    let request = String::from_utf8_lossy(&req[..n]);
                    let good = request.contains("/good");
                    let status = if request.contains("/missing") { "404 Not Found" } else { "200 OK" };
                    let body: &[u8] = if good { &bytes } else { b"<html>blocked</html>" };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                    stream.write_all(body).unwrap();
                } else { std::thread::sleep(Duration::from_millis(2)); }
            }
        });
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in");
        let root = d.path().join("lib");
        fs::create_dir_all(&root).unwrap();
        touch(&input.join("ABC-123.mp4"));
        let scan = scanned(&[input], None);
        let mut m = meta("ABC-123");
        m.fields.thumb_urls = vec![String::new(), format!("{base}/missing"), format!("{base}/bad")];
        m.fields.poster_urls = vec![format!("{base}/good")];
        let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), m)]), &root, "{code}", PlanOpts::default());
        let progress = std::sync::Mutex::new(vec![]);
        let out = apply_with_progress(plans, &http(), &root, None, &CancellationToken::new(), &|_| {},
            &|event| progress.lock().unwrap().push(event)).await;
        stop.store(true, std::sync::atomic::Ordering::Relaxed); server.join().unwrap();
        let progress = progress.into_inner().unwrap();
        let downloads = progress.iter().filter(|e| e.phase == ApplyPhase::Images).collect::<Vec<_>>();
        assert_eq!(downloads.first().unwrap().done, 0, "progress starts before network IO");
        assert!(downloads.iter().all(|e| e.total == 2), "poster and fanart are two tasks, candidates/cache do not inflate the total");
        assert_eq!(downloads.last().unwrap().done, 2);
        assert_eq!(downloads.last().unwrap().failed, 1, "failed fanart counts as processed, not successfully downloaded");
        assert_eq!(progress.last().unwrap().phase, ApplyPhase::Organize);
        let poster = fs::read(root.join("ABC-123/ABC-123-poster.jpg")).unwrap();
        let image = image::load_from_memory(&poster).expect("fallback must produce a real image");
        assert_eq!((image.width(), image.height()), (4, 6));
        assert!(!root.join("ABC-123/ABC-123-fanart.jpg").exists(), "invalid landscape must never become fanart");
        let retained: Metadata = serde_json::from_slice(&fs::read(root.join("ABC-123/kiroku.json")).unwrap()).unwrap();
        let artwork = retained.artwork.expect("normal import keeps the chosen uncropped original");
        assert_eq!(artwork.source_url.as_deref(), Some(format!("{base}/good").as_str()));
        assert_eq!(fs::read(root.join("ABC-123").join(artwork.original)).unwrap(), poster);
        assert!(out.journal.ops.iter().any(|op| matches!(op, Op::Download { url, .. } if url.ends_with("/good"))));
    }

    #[tokio::test]
    async fn cancel_during_image_progress_preserves_source_and_stops_downloads() {
        let d = tempfile::tempdir().unwrap();
        let from = d.path().join("input/ABC-123.mp4");
        let root = d.path().join("lib");
        fs::create_dir_all(&root).unwrap();
        touch(&from);
        let scanned = scanned(&[from.parent().unwrap().into()], None);
        let mut metadata = meta("ABC-123");
        metadata.fields.poster_urls = vec!["http://127.0.0.1:9/not-requested.jpg".into()];
        let pairs = scanned.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), metadata)]), &root, "{code}", PlanOpts::default());
        let cancel = CancellationToken::new();
        let events = std::sync::Mutex::new(vec![]);
        let out = apply_with_progress(plans, &http(), &root, None, &cancel, &|_| {}, &|event| {
            events.lock().unwrap().push(event);
            cancel.cancel();
        }).await;
        assert!(out.cancelled);
        assert!(out.applied.is_empty());
        assert!(out.journal.ops.is_empty());
        assert!(from.exists());
        assert_eq!(events.lock().unwrap().len(), 1);
        assert_eq!(events.lock().unwrap()[0].done, 0);
    }

    #[tokio::test]
    async fn retry_queries_only_missing_codes_once_and_keeps_success_after_cancel() {
        struct FakeSource(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
        #[async_trait::async_trait]
        impl Source for FakeSource {
            fn id(&self) -> &'static str { "fixture" }
            fn supports(&self, _: &Code) -> bool { true }
            async fn fetch(&self, code: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
                self.0.lock().unwrap().push(code.id.clone());
                Err(SourceError::NotFound)
            }
        }
        let calls = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut settings = Settings::default();
        settings.routes.insert(crate::code::ContentType::Censored, vec!["fixture".into()]);
        settings.sources.push(crate::settings::SourceCfg { id: "fixture".into(), enabled: true, base: String::new(), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false });
        let sc = Scraper { settings, http: http(), source_http: HashMap::new(), breaker: Default::default(), sources: vec![Box::new(FakeSource(calls.clone()))], translator: None };
        let codes = ["AAA-111.mp4", "BBB-222.mp4", "CCC-333-CD1.mp4", "CCC-333-CD2.mp4"].map(|name| parse_code(name).unwrap());
        let successes = HashMap::from([("AAA-111".into(), meta("AAA-111")), ("BBB-222".into(), meta("BBB-222"))]);
        let before = successes.clone();
        let result = scrape_missing(&sc, &codes, &successes, &CancellationToken::new(), &|_| {}).await;
        assert_eq!(*calls.lock().unwrap(), vec!["CCC-333"], "successful movies must never be requested during retry");
        assert_eq!(result.len(), 1); assert_eq!(successes, before);
        let cancel = CancellationToken::new(); cancel.cancel();
        let result = scrape_missing(&sc, &codes, &successes, &cancel, &|_| {}).await;
        assert!(result.is_empty()); assert_eq!(successes, before);
        let result = scrape_missing(&sc, &codes, &successes, &CancellationToken::new(), &|_| {}).await;
        assert_eq!(result.len(), 1); assert_eq!(*calls.lock().unwrap(), vec!["CCC-333", "CCC-333"]);
    }

    #[tokio::test]
    async fn a_user_given_detail_url_pins_that_source_and_warns_on_code_mismatch() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let _ = stream.read(&mut [0; 4096]);
            let body = include_str!("../tests/fixtures/jav321/detail_XYZ-018_48xyz.html");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        struct Other;
        #[async_trait::async_trait]
        impl Source for Other {
            fn id(&self) -> &'static str { "javdb" }
            fn supports(&self, _: &Code) -> bool { true }
            async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
                Ok(PartialMetadata { title: Some("2022 title".into()), release_date: Some("2022-01-01".into()), ..Default::default() })
            }
        }
        let mut settings = Settings { min_interval_secs: 0, ..Settings::default() };
        settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap().base = base.clone();
        let sources: Vec<Box<dyn Source>> = vec![Box::new(Other), Box::new(crate::sources::jav321::Jav321 { base: base.clone() })];
        let sc = Scraper::with_sources(settings.clone(), sources).unwrap();
        let cancel = CancellationToken::new();
        let url = format!("{base}/video/48xyz00018");
        let m = sc.scrape_url(&parse_code("XYZ-018.mp4").unwrap(), &url, &cancel).await.unwrap();
        assert_eq!(m.fields.title.as_deref(), Some("Sample Title 2010"), "the chosen page is authoritative");
        assert_eq!(m.source_urls.get("jav321").map(String::as_str), Some(url.as_str()));
        assert!(m.scrape_notes.iter().any(|n| n.contains("javdb")), "the other source is the one dropped: {:?}", m.scrape_notes);
        let m = sc.scrape_url(&parse_code("ABC-001.mp4").unwrap(), &url, &cancel).await.unwrap();
        assert!(m.scrape_notes.iter().any(|n| n.contains("ABC-001")), "page does not mention the file's code: {:?}", m.scrape_notes);
        // DMM 只有搜索 API:按地址刮削在发请求前就明确拒绝
        settings.sources.iter_mut().find(|c| c.id == "dmm").unwrap().enabled = true;
        settings.routes.get_mut(&crate::code::ContentType::Censored).unwrap().push("dmm".into());
        let dmm = Scraper::with_sources(settings.clone(), settings.build_sources().into_iter().filter(|s| s.id() == "dmm").collect()).unwrap();
        let err = dmm.scrape_url(&parse_code("XYZ-018.mp4").unwrap(), "https://www.dmm.co.jp/digital/videoa/-/detail/=/cid=xyz00018/", &cancel).await.unwrap_err();
        assert!(err.message().contains("不支持"), "{}", err.message());
        assert!(sc.scrape_url(&parse_code("XYZ-018.mp4").unwrap(), "https://evil.example/x", &cancel).await.is_err());
    }

    #[tokio::test]
    async fn scraping_one_code_by_url_reports_through_scrape_events() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let _ = stream.read(&mut [0; 4096]);
            let body = include_str!("../tests/fixtures/jav321/detail_ABC-001.html");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        let mut settings = Settings { min_interval_secs: 0, ..Settings::default() };
        settings.routes.insert(crate::code::ContentType::Censored, vec!["jav321".into()]);
        settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap().base = base.clone();
        let sc = Scraper::with_sources(settings, vec![Box::new(crate::sources::jav321::Jav321 { base: base.clone() })]).unwrap();
        let events = std::sync::Mutex::new(vec![]);
        let code = parse_code("ABC-001.mp4").unwrap();
        let r = scrape_url_one(&sc, &code, &format!("{base}/video/abc00001"), &CancellationToken::new(), &|e| events.lock().unwrap().push(e)).await;
        assert!(r.unwrap().fields.title.is_some());
        let events = events.into_inner().unwrap();
        assert!(matches!(events.first().map(|e| &e.status), Some(ScrapeStatus::Scraping { site }) if site == "jav321"));
        let last = events.last().unwrap();
        assert!(matches!(last.status, ScrapeStatus::Found) && last.done == 1 && last.total == 1 && last.preview.is_some());
        let err = scrape_url_one(&sc, &code, "https://evil.example/x", &CancellationToken::new(), &|e| { if let ScrapeStatus::Failed { message } = &e.status { assert!(message.contains("evil.example")) } }).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn selected_fields_can_be_refreshed_from_a_user_given_detail_page() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let _ = stream.read(&mut [0; 4096]);
            let body = include_str!("../tests/fixtures/jav321/detail_ABC-001.html");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        let mut settings = Settings { min_interval_secs: 0, ..Settings::default() };
        settings.routes.insert(crate::code::ContentType::Censored, vec!["jav321".into()]);
        settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap().base = base.clone();
        let sc = Scraper::with_sources(settings, vec![Box::new(crate::sources::jav321::Jav321 { base: base.clone() })]).unwrap();
        let mut before = meta("ABC-001");
        before.fields.title = Some("old".into());
        before.fields.studio = Some("kept".into());
        let after = sc.refresh_item_url(&parse_code("ABC-001.mp4").unwrap(), &before, &["title".into()], true, &format!("{base}/video/abc00001"), &CancellationToken::new()).await.unwrap();
        let expected = crate::sources::jav321::parse_detail(include_str!("../tests/fixtures/jav321/detail_ABC-001.html")).unwrap().title;
        assert_eq!(after.fields.title, expected);
        assert_eq!(after.fields.studio.as_deref(), Some("kept"), "unselected fields stay");
        assert!(sc.refresh_item_url(&parse_code("ABC-001.mp4").unwrap(), &before, &["title".into()], true, "https://evil.example/x", &CancellationToken::new()).await.is_err());
    }

    #[tokio::test]
    async fn sites_for_one_movie_are_queried_concurrently_but_aggregated_in_route_order() {
        struct Slow(&'static str, u64, std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
        #[async_trait::async_trait]
        impl Source for Slow {
            fn id(&self) -> &'static str { self.0 }
            fn supports(&self, _: &Code) -> bool { true }
            async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
                tokio::time::sleep(Duration::from_millis(self.1)).await;
                self.2.lock().unwrap().push(self.0);
                Ok(PartialMetadata { title: Some(format!("{} title", self.0)), studio: Some(format!("{} studio", self.0)), ..Default::default() })
            }
        }
        let finished = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut settings = Settings::default();
        settings.routes.insert(crate::code::ContentType::Censored, vec!["a".into(), "b".into(), "c".into()]);
        for id in ["a", "b", "c"] { settings.sources.push(crate::settings::SourceCfg { id: id.into(), enabled: true, base: format!("https://{id}.example"), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false }); }
        // The first route site is the slowest: it must still win every scalar field.
        let sc = Scraper::with_sources(settings, vec![Box::new(Slow("a", 400, finished.clone())), Box::new(Slow("b", 100, finished.clone())), Box::new(Slow("c", 200, finished.clone()))]).unwrap();
        let sites = std::sync::Mutex::new(vec![]);
        let start = std::time::Instant::now();
        let m = sc.scrape_item(&parse_code("ABC-123.mp4").unwrap(), &CancellationToken::new(), &|site| sites.lock().unwrap().push(site.to_string())).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(650), "total follows the slowest site: {:?}", start.elapsed());
        assert_eq!(*finished.lock().unwrap(), vec!["b", "c", "a"], "sites really ran at the same time");
        assert_eq!(m.fields.title.as_deref(), Some("a title"));
        assert_eq!(m.provenance.get("studio").map(String::as_str), Some("a"));
        assert_eq!(sites.into_inner().unwrap(), vec!["a,b,c".to_string()], "progress names every site being queried");

        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(50)).await; stop.cancel() });
        let start = std::time::Instant::now();
        let r = sc.scrape_item(&parse_code("ABC-124.mp4").unwrap(), &cancel, &|_| {}).await;
        assert!(matches!(r, Err(ScrapeError::Cancelled)) && start.elapsed() < Duration::from_millis(250));
    }

    /// 按顺序给出结果的假来源:用完序列后一直返回最后一项。
    struct Scripted(&'static str, std::sync::Mutex<std::collections::VecDeque<&'static str>>, std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait::async_trait]
    impl Source for Scripted {
        fn id(&self) -> &'static str { self.0 }
        fn supports(&self, _: &Code) -> bool { true }
        async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
            self.2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut script = self.1.lock().unwrap();
            let next = if script.len() > 1 { script.pop_front().unwrap() } else { script[0] };
            match next {
                "ok" => Ok(PartialMetadata { title: Some(format!("{} title", self.0)), ..Default::default() }),
                "missing" => Err(SourceError::NotFound),
                "blocked" => Err(SourceError::Blocked("status 403".into())),
                _ => Err(SourceError::Network("connection refused".into())),
            }
        }
    }
    async fn breaker_run(script: &[&'static str], movies: usize) -> (usize, HashMap<String, Result<Metadata, ScrapeError>>, Vec<Option<String>>) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut settings = Settings::default();
        settings.routes.insert(crate::code::ContentType::Censored, vec!["a".into(), "b".into()]);
        for id in ["a", "b"] { settings.sources.push(crate::settings::SourceCfg { id: id.into(), enabled: true, base: format!("https://{id}.example"), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false }); }
        let other = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sc = Scraper::with_sources(settings, vec![
            Box::new(Scripted("a", std::sync::Mutex::new(script.iter().copied().collect()), calls.clone())),
            Box::new(Scripted("b", std::sync::Mutex::new(["missing"].into_iter().collect()), other)),
        ]).unwrap();
        let codes: Vec<_> = (0..movies).map(|i| parse_code(&format!("ABC-{:03}.mp4", i + 1)).unwrap()).collect();
        let notes = std::sync::Mutex::new(vec![]);
        let out = scrape_all(&sc, &codes, &CancellationToken::new(), &|e| if !matches!(e.status, ScrapeStatus::Scraping { .. } | ScrapeStatus::Translating) { notes.lock().unwrap().push(e.note) }).await;
        (calls.load(std::sync::atomic::Ordering::SeqCst), out, notes.into_inner().unwrap())
    }

    #[tokio::test]
    async fn a_site_that_keeps_failing_is_paused_for_the_rest_of_the_batch() {
        let (calls, out, _) = breaker_run(&["down"], 6).await;
        assert_eq!(calls, 3, "after three consecutive failures the site is no longer requested");
        for code in ["ABC-004", "ABC-005", "ABC-006"] {
            let Err(ScrapeError::Failed(messages)) = &out[code] else { panic!("{code}: {:?}", out[code].as_ref().map(|_| ())) };
            assert!(messages.iter().any(|m| m.contains("a") && m.contains("来源暂停")), "{messages:?}");
        }
        let (calls, _, _) = breaker_run(&["blocked"], 5).await;
        assert_eq!(calls, 3, "403 and similar blocks count as failures");
    }

    #[tokio::test]
    async fn not_found_and_successes_never_pause_a_site() {
        assert_eq!(breaker_run(&["missing"], 5).await.0, 5);
        assert_eq!(breaker_run(&["down", "down", "ok", "down", "down", "ok"], 6).await.0, 6, "a success resets the count");
    }

    #[tokio::test]
    async fn a_paused_site_is_reported_on_movies_found_elsewhere() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut settings = Settings::default();
        settings.routes.insert(crate::code::ContentType::Censored, vec!["a".into(), "b".into()]);
        for id in ["a", "b"] { settings.sources.push(crate::settings::SourceCfg { id: id.into(), enabled: true, base: format!("https://{id}.example"), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false }); }
        let sc = Scraper::with_sources(settings, vec![
            Box::new(Scripted("a", std::sync::Mutex::new(["down"].into_iter().collect()), calls.clone())),
            Box::new(Scripted("b", std::sync::Mutex::new(["ok"].into_iter().collect()), std::sync::Arc::new(Default::default()))),
        ]).unwrap();
        let codes: Vec<_> = (0..4).map(|i| parse_code(&format!("ABC-{:03}.mp4", i + 1)).unwrap()).collect();
        let notes = std::sync::Mutex::new(HashMap::new());
        let out = scrape_all(&sc, &codes, &CancellationToken::new(), &|e| if let Some(n) = e.note { notes.lock().unwrap().insert(e.code, n); }).await;
        assert!(out.values().all(Result::is_ok));
        assert!(notes.into_inner().unwrap().get("ABC-004").is_some_and(|n| n.contains("来源暂停")));
    }

    #[tokio::test]
    async fn conflicting_release_dates_drop_the_odd_source_but_keep_its_snapshot() {
        struct Dated(&'static str, &'static str);
        #[async_trait::async_trait]
        impl Source for Dated {
            fn id(&self) -> &'static str { self.0 }
            fn supports(&self, _: &Code) -> bool { true }
            async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
                Ok(PartialMetadata { title: Some(format!("{} title", self.0)), release_date: Some(self.1.into()), ..Default::default() })
            }
        }
        let mut settings = Settings::default();
        settings.routes.insert(crate::code::ContentType::Censored, vec!["a".into(), "b".into()]);
        settings.field_priority.insert("title".into(), vec!["b".into()]);
        for id in ["a", "b"] { settings.sources.push(crate::settings::SourceCfg { id: id.into(), enabled: true, base: format!("https://{id}.example"), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false }); }
        let sc = Scraper::with_sources(settings, vec![Box::new(Dated("a", "2022-01-01")), Box::new(Dated("b", "2010-01-06"))]).unwrap();
        let notes = std::sync::Mutex::new(vec![]);
        let out = scrape_all(&sc, &[parse_code("MOVX-018.mp4").unwrap()], &CancellationToken::new(), &|e| if let Some(n) = e.note { notes.lock().unwrap().push(n) }).await;
        let m = out["MOVX-018"].as_ref().unwrap();
        assert_eq!(m.fields.title.as_deref(), Some("a title"), "b is preferred for title but its date conflicts");
        assert!(m.raw_sources.contains_key("b"), "the dropped result stays available for manual reselection");
        let notes = notes.into_inner().unwrap();
        assert!(notes.iter().any(|n| n.contains("来源结果不一致") && n.contains("b")), "{notes:?}");
    }

    #[tokio::test]
    async fn changing_one_source_proxy_only_refetches_that_source() {
        struct Counted(&'static str, std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
        #[async_trait::async_trait]
        impl Source for Counted {
            fn id(&self) -> &'static str { self.0 }
            fn supports(&self, _: &Code) -> bool { true }
            async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
                self.1.lock().unwrap().push(self.0);
                Ok(PartialMetadata { title: Some(format!("{} title", self.0)), ..Default::default() })
            }
        }
        let calls = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut settings = Settings { proxy: Some("http://127.0.0.1:9".into()), ..Settings::default() };
        settings.routes.insert(crate::code::ContentType::Censored, vec!["a".into(), "b".into()]);
        for id in ["a", "b"] { settings.sources.push(crate::settings::SourceCfg { id: id.into(), enabled: true, base: format!("https://{id}.example"), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false }); }
        let sources = |calls: &std::sync::Arc<_>| -> Vec<Box<dyn Source>> { vec![Box::new(Counted("a", std::sync::Arc::clone(calls))), Box::new(Counted("b", std::sync::Arc::clone(calls)))] };
        let code = parse_code("ABC-123.mp4").unwrap();
        let first = Scraper::with_sources(settings.clone(), sources(&calls)).unwrap().scrape_item(&code, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["a", "b"]);
        calls.lock().unwrap().clear();
        settings.sources.iter_mut().find(|c| c.id == "a").unwrap().proxy = crate::settings::SourceProxy::Direct;
        let sc = Scraper::with_sources(settings, sources(&calls)).unwrap();
        sc.scrape_cached(&code, &first.raw_sources, false, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["a"], "only the source whose proxy changed is fetched again");
    }

    fn png_bytes() -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(4, 6).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[tokio::test]
    async fn repair_writes_only_missing_artwork_and_rollback_preserves_original_files() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let bytes = png_bytes();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0; 2048]; stream.read(&mut req).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        let original = [("ABC-123.mp4", b"video".to_vec()), ("movie.nfo", b"original nfo".to_vec()), ("kiroku.json", b"original sidecar".to_vec()), ("fanart.jpg", png_bytes())];
        for (name, bytes) in &original { fs::write(dir.join(name), bytes).unwrap(); }
        let mut meta = meta("ABC-123"); meta.fields.thumb_urls = vec![url];
        let lib = crate::library::Library::open(d.path()).unwrap();
        // Keep real metadata on disk for incremental cache verification.
        fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        let sidecar_before = fs::read(dir.join("kiroku.json")).unwrap();
        lib.reindex_dir(&dir).unwrap(); assert!(lib.get("ABC-123").unwrap().unwrap().card.poster.is_none());
        let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
        assert_eq!(plan.ops.len(), 1); assert!(plan.ops.iter().all(|op| matches!(op, Op::Download { .. })));
        let mut out = apply(vec![plan], &http(), d.path(), None, &CancellationToken::new(), &|_| {}).await;
        server.join().unwrap();
        assert!(out.failed.is_empty()); assert!(dir.join("poster.jpg").exists());
        lib.reindex_dir(&dir).unwrap(); assert!(lib.get("ABC-123").unwrap().unwrap().card.poster.is_some());
        assert!(out.journal.rollback().is_empty());
        assert!(!dir.join("poster.jpg").exists());
        for (name, bytes) in &original { if *name != "kiroku.json" { assert_eq!(fs::read(dir.join(name)).unwrap(), *bytes); } }
        assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), sidecar_before);
    }

    #[test]
    fn repair_preserves_damaged_and_alternate_artwork_and_rejects_outside_directory() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("poster.png"), b"damaged").unwrap();
        let mut meta = meta("ABC-123"); meta.fields.poster_urls = vec!["candidate".into()];
        let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
        assert!(plan.ops.is_empty()); assert!(plan.notices.iter().any(|s| s.contains("不是有效图片")));
        assert_eq!(fs::read(dir.join("poster.png")).unwrap(), b"damaged");
        let outside = tempfile::tempdir().unwrap(); assert!(build_image_repair(&meta, outside.path(), d.path()).is_err());
    }

    #[test]
    fn repair_rechecks_alternative_immediately_before_publish_and_keeps_committed_other_stem() {
        let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        let ops = ["poster", "fanart"].map(|stem| Op::Download { path: dir.join(format!("{stem}.jpg")), url: stem.into(), crop: None, stamp: None });
        let mut journal = Journal::new(d.path());
        let alternative = dir.join("fanart.webp"); let bytes = png_bytes();
        let ex = execute_artwork_repair(&mut journal, &Plan { ops: ops.to_vec() }, &mut |url| {
            if url == "fanart" { fs::write(&alternative, &bytes)?; }
            Ok(bytes.clone())
        }, &|| false);
        assert_eq!(ex.done.len(), 1); assert_eq!(journal.ops.len(), 1);
        assert_eq!(ex.error.unwrap().kind(), io::ErrorKind::AlreadyExists);
        assert!(dir.join("poster.jpg").exists()); assert!(!dir.join("fanart.jpg").exists());
        assert_eq!(fs::read(&alternative).unwrap(), bytes);
        assert!(journal.rollback().is_empty());
        assert!(!dir.join("poster.jpg").exists()); assert_eq!(fs::read(alternative).unwrap(), bytes);
    }

    #[test]
    fn repair_can_publish_both_stems_without_treating_its_first_image_as_a_conflict() {
        let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        let ops = ["poster", "fanart"].map(|stem| Op::Download { path: dir.join(format!("{stem}.jpg")), url: stem.into(), crop: None, stamp: None });
        let mut journal = Journal::new(d.path());
        let ex = execute_artwork_repair(&mut journal, &Plan { ops: ops.to_vec() }, &mut |_| Ok(png_bytes()), &|| false);
        assert!(ex.error.is_none()); assert_eq!(ex.done.len(), 2); assert_eq!(journal.ops.len(), 2);
    }

    #[tokio::test]
    async fn repair_conflicts_with_any_artwork_format_created_during_download() {
        use std::io::{Read, Write};
        for stem in ["poster", "fanart"] {
            for ext in ["png", "jpg", "jpeg", "webp"] {
                let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
                // Only the tested target is missing.
                let other = if stem == "poster" { "fanart" } else { "poster" };
                fs::write(dir.join(format!("{other}.jpg")), png_bytes()).unwrap();
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let mut meta = meta("ABC-123"); meta.fields.thumb_urls = vec![format!("http://{}/image", listener.local_addr().unwrap())];
                let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
                let alternative = dir.join(format!("{stem}.{ext}")); let created = alternative.clone();
                let bytes = png_bytes(); let original = bytes.clone();
                let server = std::thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap(); let mut req = [0;2048]; stream.read(&mut req).unwrap();
                    fs::write(created, &bytes).unwrap();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap(); stream.write_all(&bytes).unwrap();
                });
                let out = apply(vec![plan], &http(), d.path(), None, &CancellationToken::new(), &|_| {}).await;
                server.join().unwrap();
                assert!(!out.failed.is_empty(), "{stem}.{ext} appeared during download; must report conflict");
                assert!(out.failed[0].1.contains("占用"));
                assert!(out.journal.ops.is_empty());
                if ext != "jpg" { assert!(!dir.join(format!("{stem}.jpg")).exists()); }
                assert_eq!(fs::read(alternative).unwrap(), original);
                assert_eq!(fs::read(dir.join("ABC-123.mp4")).unwrap(), b"video");
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repair_conflicts_with_dangling_alternative_artwork_symlink() {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
            let mut meta = meta("ABC-123"); meta.fields.poster_urls = vec![String::new()];
            let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
            let alternative = dir.join(format!("poster.{ext}")); let destination = dir.join("absent");
            std::os::unix::fs::symlink(&destination, &alternative).unwrap();
            let out = apply(vec![plan], &http(), d.path(), None, &CancellationToken::new(), &|_| {}).await;
            assert!(!out.failed.is_empty()); assert!(out.failed[0].1.contains("占用"));
            assert!(out.journal.ops.is_empty()); assert_eq!(fs::read_link(alternative).unwrap(), destination);
        }
    }

    #[tokio::test]
    async fn repair_reports_exhausted_candidates_as_failure_without_writing() {
        let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
        let mut meta = meta("ABC-123"); meta.fields.poster_urls = vec![String::new()];
        let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
        let out = apply(vec![plan], &http(), d.path(), None, &CancellationToken::new(), &|_| {}).await;
        assert_eq!(out.failed.len(), 1, "failed artwork must not look like a successful repair");
        assert_eq!(out.journal.status, JournalStatus::Partial);
        assert!(out.journal.ops.is_empty()); assert!(!dir.join("poster.jpg").exists());
        assert_eq!(fs::read(dir.join("ABC-123.mp4")).unwrap(), b"video");
    }

    #[tokio::test]
    async fn repair_protects_artwork_that_appears_after_planning_and_cancellation_writes_nothing() {
        let d = tempfile::tempdir().unwrap(); let dir = d.path().join("ABC-123"); fs::create_dir_all(&dir).unwrap();
        let mut meta = meta("ABC-123"); meta.fields.poster_urls = vec![String::new()];
        let plan = build_image_repair(&meta, &dir, d.path()).unwrap();
        fs::write(dir.join("poster.jpg"), png_bytes()).unwrap();
        let before = fs::read(dir.join("poster.jpg")).unwrap();
        let out = apply(vec![plan.clone()], &http(), d.path(), None, &CancellationToken::new(), &|_| {}).await;
        assert!(!out.failed.is_empty()); assert!(out.journal.ops.is_empty());
        assert_eq!(fs::read(dir.join("poster.jpg")).unwrap(), before);
        let cancel = CancellationToken::new(); cancel.cancel();
        let out = apply(vec![plan], &http(), d.path(), None, &cancel, &|_| {}).await;
        assert!(out.cancelled); assert!(out.journal.ops.is_empty());
        assert_eq!(fs::read(dir.join("poster.jpg")).unwrap(), before);
    }

    #[tokio::test]
    async fn apply_writes_journal_and_rollback_restores() {
        let d = tempfile::tempdir().unwrap();
        let (inp, lib) = (d.path().join("in"), d.path().join("lib"));
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("ABC-123.mp4"));
        let r = scanned(&[inp.clone()], Some(&lib));
        let pairs: Vec<_> = r.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
        let metas = HashMap::from([("ABC-123".to_string(), meta("ABC-123"))]);
        let plans = build_plans(&pairs, &metas, &lib, "{actor}/{code}", PlanOpts::default());
        let out = apply(plans, &http(), &lib, None, &CancellationToken::new(), &|_| {}).await;
        assert!(out.failed.is_empty() && !out.cancelled);
        assert_eq!(out.applied.len(), 1);
        assert!(lib.join("Foo/ABC-123/ABC-123.mp4").exists() && lib.join("Foo/ABC-123/ABC-123.nfo").exists());
        assert!(lib.join("Foo/ABC-123").join(organize::SIDECAR).exists());

        let mut j = Journal::list(&lib).remove(0);
        assert_eq!(j.status, JournalStatus::Applied);
        assert!(j.rollback().is_empty());
        assert!(inp.join("ABC-123.mp4").exists());
        assert!(!lib.join("Foo").exists(), "空目录被清理");
        assert_eq!(Journal::list(&lib)[0].status, JournalStatus::RolledBack);
    }

    #[tokio::test]
    async fn apply_stops_before_moving_when_journal_is_unwritable() {
        let d = tempfile::tempdir().unwrap();
        let inp = d.path().join("in");
        let lib = d.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("ABC-123.mp4"));
        touch(&lib.join(".kiroku")); // 日志目录被普通文件占用
        let r = scanned(&[inp.clone()], Some(&lib));
        let pairs: Vec<_> = r.items.iter().filter_map(|it| it.code.as_ref().map(|c| (it,c))).collect();
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(),meta("ABC-123"))]), &lib, "{code}", PlanOpts::default());
        let out = apply(plans, &http(), &lib, None, &CancellationToken::new(), &|_| {}).await;
        assert!(!out.failed.is_empty());
        assert_eq!(fs::read(inp.join("ABC-123.mp4")).unwrap(), b"v");
        assert!(!lib.join("ABC-123/ABC-123.mp4").exists());
    }

    #[tokio::test]
    async fn dropout_during_retained_binding_never_recreates_library() {
        use std::io::Write;
        let t=tempfile::tempdir().unwrap();let root=t.path().join("library");fs::create_dir(&root).unwrap();
        let from=t.path().join("ABC-123.mp4");let mut f=fs::File::create(&from).unwrap();
        for _ in 0..2048 {f.write_all(&[9;65536]).unwrap();}drop(f);
        let code=crate::code::parse_code("ABC-123.mp4").unwrap();
        let metas=HashMap::from([(code.id.clone(),Metadata{code:code.id.clone(),..Default::default()})]);
        let scan=scanned(&[t.path().to_path_buf()],Some(&root));let pairs=scan.items.iter().map(|item|(item,item.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans=build_plans(&pairs,&metas,&root,"{code}",PlanOpts{mode:crate::placement::OrganizeMode::Copy,..Default::default()});
        let (tx,rx)=tokio::sync::oneshot::channel();let tx=std::sync::Mutex::new(Some(tx));let cancel=CancellationToken::new();
        let moved=t.path().join("detached");
        let client=http();let progress=|event:ApplyProgress|{
            if event.phase==ApplyPhase::Organize && event.done==0 {if let Some(tx)=tx.lock().unwrap().take(){let _=tx.send(());}}
        };
        let (out,())=tokio::join!(apply_with_progress(plans,&client,&root,None,&cancel,&|_|{},&progress),async {rx.await.unwrap();tokio::time::sleep(std::time::Duration::from_millis(20)).await;fs::rename(&root,&moved).unwrap();});
        assert!(out.applied.is_empty());assert!(from.exists());assert!(!root.exists(),"lost library must never be recreated during proof binding");
    }

    #[tokio::test]
    async fn cancelling_large_media_read_is_reported_as_cancel_without_failure() {
        use std::io::Write;
        let t=tempfile::tempdir().unwrap();let root=t.path().join("lib");fs::create_dir(&root).unwrap();
        let from=t.path().join("ABC-123.mp4");let mut f=fs::File::create(&from).unwrap();for _ in 0..2048{f.write_all(&[12;65536]).unwrap();}drop(f);
        let scan=scanned(&[t.path().to_path_buf()],Some(&root));let pairs=scan.items.iter().map(|item|(item,item.code.as_ref().unwrap())).collect::<Vec<_>>();
        let plans=build_plans(&pairs,&HashMap::from([("ABC-123".into(),meta("ABC-123"))]),&root,"{code}",PlanOpts::default());
        let cancel=CancellationToken::new();let client=http();let (tx,rx)=tokio::sync::oneshot::channel();let tx=std::sync::Mutex::new(Some(tx));
        let progress=|event:ApplyProgress|{if event.phase==ApplyPhase::Organize && event.done==0 {if let Some(tx)=tx.lock().unwrap().take(){let _=tx.send(());}}};
        let (out,())=tokio::join!(apply_with_progress(plans,&client,&root,None,&cancel,&|_|{},&progress),async {rx.await.unwrap();tokio::time::sleep(std::time::Duration::from_millis(20)).await;cancel.cancel();});
        assert!(out.cancelled);assert!(out.failed.is_empty(),"{:?}",out.failed);assert!(out.applied.is_empty());assert!(from.exists());assert!(out.journal.pending.is_none());
    }

    #[tokio::test]
    async fn apply_cancel_midway_still_writes_journal() {
        let d = tempfile::tempdir().unwrap();
        let (inp, lib) = (d.path().join("in"), d.path().join("lib"));
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("AAA-111.mp4"));
        touch(&inp.join("BBB-222.mp4"));
        let r = scanned(&[inp.clone()], Some(&lib));
        let pairs: Vec<_> = r.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
        let metas: HashMap<_, _> = ["AAA-111", "BBB-222"].iter().map(|c| (c.to_string(), meta(c))).collect();
        let plans = build_plans(&pairs, &metas, &lib, "{actor}/{code}", PlanOpts::default());
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        // 第一项完成后取消
        let out = apply(plans, &http(), &lib, None, &cancel, &move |_| c2.cancel()).await;
        assert!(out.cancelled);
        assert_eq!(out.applied.len(), 1, "只完成了第一项");
        assert_eq!(out.journal.status, JournalStatus::Partial);
        let saved = Journal::list(&lib);
        assert_eq!(saved.len(), 1, "取消也必须有日志");
        assert!(!saved[0].ops.is_empty());
        // 另一个文件没被动
        assert_eq!(fs::read_dir(&inp).unwrap().count(), 1);
    }

    #[test]
    fn preview_prefers_landscape_cover_and_lists_sources() {
        let mut m = meta("ABC-123");
        m.fields.poster_urls = vec!["p.jpg".into()];
        m.fields.thumb_urls = vec!["t.jpg".into()];
        m.provenance.insert("title".into(), "jav321".into());
        m.provenance.insert("actors".into(), "javdb".into());
        m.provenance.insert("plot".into(), "jav321".into());
        let p = Preview::of(&m);
        assert_eq!((p.cover.as_deref(), p.cover_is_landscape), (Some("t.jpg"), true));
        assert_eq!(p.sources, vec!["jav321", "javdb"]);
        assert_eq!(p.actors, vec!["Foo"]);
        m.fields.thumb_urls.clear();
        let p = Preview::of(&m);
        assert_eq!((p.cover.as_deref(), p.cover_is_landscape), (Some("p.jpg"), false));
    }

    /// 测试用「废纸篓」:直接删除并记录
    fn fake_trash(log: &std::sync::Mutex<Vec<PathBuf>>) -> impl Fn(&Path) -> io::Result<()> + '_ {
        move |p: &Path| {
            log.lock().unwrap().push(p.to_path_buf());
            fs::remove_dir_all(p)
        }
    }

    #[test]
    fn a_file_root_never_trashes_the_folder_it_came_from() {
        let d = tempfile::tempdir().unwrap();
        let downloads = d.path().join("Downloads");
        touch(&downloads.join("info.txt"));
        let file = downloads.join("ABC-123.mp4");
        let log = std::sync::Mutex::new(vec![]);
        // The video has been moved away; only junk is left in its folder.
        assert!(cleanup_sources(&[file.clone()], &[file], &VideoFilter::default(), &fake_trash(&log)).is_empty());
        assert!(log.lock().unwrap().is_empty() && downloads.join("info.txt").exists());
    }

    #[test]
    fn cleanup_only_trashes_emptied_folders_inside_roots() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("Downloads");
        // A:整理后只剩垃圾文件和样片 → 清理
        touch(&root.join("A/info.txt"));
        touch(&root.join("A/sample.mp4"));
        // B:还剩一部没整理的正片 → 保留
        touch(&root.join("B/OTHER-001.mp4"));
        // C/sub:子目录清掉后,C 也空了 → 一并清理,只报告最外层
        touch(&root.join("C/sub/readme.url"));
        // 根目录下的散文件:根目录永远保留
        let moved = vec![root.join("A/ABC-123.mp4"), root.join("B/ABC-124.mp4"), root.join("C/sub/ABC-125.mp4"), root.join("ABC-126.mp4")];
        let log = std::sync::Mutex::new(vec![]);
        let cleaned = cleanup_sources(&moved, &[root.clone()], &VideoFilter::default(), &fake_trash(&log));
        let got: std::collections::BTreeSet<_> = cleaned.into_iter().collect();
        assert_eq!(got, [root.join("A"), root.join("C")].into_iter().collect(), "只报告最外层,B 和根目录保留");
        assert!(!root.join("A").exists() && !root.join("C").exists());
        assert!(root.join("B/OTHER-001.mp4").exists());
        assert!(root.exists());
        // 根目录之外的路径不碰
        let outside = d.path().join("elsewhere/X");
        touch(&outside.join("x.txt"));
        assert!(cleanup_sources(&[outside.join("v.mp4")], &[root.clone()], &VideoFilter::default(), &fake_trash(&log)).is_empty());
        assert!(outside.exists());
    }

    #[tokio::test]
    async fn apply_with_cleanup_records_trashed_in_journal() {
        let d = tempfile::tempdir().unwrap();
        let (inp, lib) = (d.path().join("in"), d.path().join("lib"));
        fs::create_dir_all(&lib).unwrap();
        touch(&inp.join("pack/ABC-123.mp4"));
        touch(&inp.join("pack/ABC-123.chs.srt"));
        touch(&inp.join("pack/torrent.txt"));
        let r = scanned(&[inp.clone()], Some(&lib));
        let pairs: Vec<_> = r.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
        let metas = HashMap::from([("ABC-123".to_string(), meta("ABC-123"))]);
        let plans = build_plans(&pairs, &metas, &lib, "{actor}/{code}", PlanOpts::default());
        let roots = [inp.clone()];
        let out = apply(plans, &http(), &lib, Some(Cleanup { roots: &roots, trash: |p: &Path| fs::remove_dir_all(p), filter: &VideoFilter::default() }), &CancellationToken::new(), &|_| {}).await;
        assert_eq!(out.cleaned, vec![inp.join("pack")]);
        assert!(!inp.join("pack").exists(), "空出来的原文件夹进了废纸篓");
        assert!(lib.join("Foo/ABC-123/ABC-123.chs.srt").exists(), "字幕先跟随视频移走了");
        assert_eq!(Journal::list(&lib)[0].trashed, vec![inp.join("pack")]);
    }

    #[test]
    fn an_external_chinese_subtitle_marks_the_movie_as_subtitled_when_enabled() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in");
        touch(&input.join("ABC-123.mp4"));
        touch(&input.join("ABC-123.chs.srt"));
        let library = d.path().join("lib");
        fs::create_dir_all(&library).unwrap();
        let r = scanned(&[input], None);
        let pairs: Vec<_> = r.items.iter().filter_map(|it| it.code.as_ref().map(|code| (it, code))).collect();
        let mut m = meta("ABC-123");
        m.fields.thumb_urls = vec!["https://x.invalid/cover.jpg".into()];
        let metas = HashMap::from([("ABC-123".into(), m)]);
        let stamped = |external_subtitle: bool| {
            let badges = crate::badge::BadgeCfg { enabled: true, external_subtitle, ..Default::default() };
            build_plans(&pairs, &metas, &library, "{code}", PlanOpts { badges, ..Default::default() })[0].ops.iter()
                .find_map(|op| match op { Op::Download { stamp: Some(s), .. } => Some(s.marks.iter().any(|m| m.stem == "subtitle")), _ => None })
                .unwrap_or(false)
        };
        assert!(stamped(true));
        assert!(!stamped(false), "the option turns the rule off");
        assert!(crate::badge::BadgeCfg::default().external_subtitle, "on by default");
    }

    #[tokio::test]
    async fn cleanup_never_trashes_an_ancestor_of_the_library() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("sources");
        let library = input.join("pack/library");
        fs::create_dir_all(&library).unwrap();
        touch(&input.join("pack/downloads/ABC-123.mp4"));
        let r = scanned(&[input.clone()], Some(&library));
        let pairs: Vec<_> = r.items.iter().filter_map(|it| it.code.as_ref().map(|code| (it,code))).collect();
        let mut m = meta("ABC-123"); m.fields.title = Some("sample".into());
        let plans = build_plans(&pairs, &HashMap::from([("ABC-123".into(), m)]), &library, "{code}", PlanOpts { plex_naming: true, ..Default::default() });
        let roots = [input];
        let outcome = apply(plans, &http(), &library, Some(Cleanup { roots: &roots, trash: |p| fs::remove_dir_all(p), filter: &VideoFilter::default() }), &CancellationToken::new(), &|_| {}).await;
        assert!(outcome.failed.is_empty());
        assert!(library.join("ABC-123/ABC-123 sample.mp4").exists());
        assert!(!outcome.cleaned.contains(&d.path().join("sources/pack")));
    }

    #[test]
    fn translation_keeps_original_title() {
        let mut m = meta("ABC-123");
        m.fields.title = Some("海辺の約束".into());
        m.fields.plot = Some("あらすじ".into());
        crate::localization::capture_original(&mut m);
        apply_translation(&mut m, crate::translate::Translated { title: Some("海边的约定".into()), plot: None });
        assert_eq!(m.fields.title.as_deref(), Some("海边的约定"));
        assert_eq!(m.fields.original_title.as_deref(), Some("海辺の約束"), "日文原题保留");
        assert_eq!(m.fields.plot.as_deref(), Some("あらすじ"), "没译的字段不动");
        assert_eq!(m.provenance["title"], "ai");
    }

    #[test]
    fn code_from_info_matches_parse_code() {
        let i = parse_file_info("/x/abc123-cd2.mp4");
        assert_eq!(Code::from_info(&i), parse_code("/x/abc123-cd2.mp4"));
    }
}
