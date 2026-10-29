//! 用户设置:CLI 与桌面客户端共用同一份 JSON。核心不依赖 UI。

use crate::{
    aggregate::{compile_priority, Chains},
    code::ContentType,
    net::Http,
    source::Source,
    sources::{jav321::Jav321, javbus::JavBus, javdb::JavDb, dmm::Dmm, mgstage::Mgstage, fc2::Fc2, fc2ppvdb::Fc2PpvDb, avsox::Avsox, javlibrary::JavLibrary, xcity::XCity, avbase::AvBase},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::{Path, PathBuf}, time::Duration};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCfg {
    pub id: String,
    pub enabled: bool,
    /// 站点域名(镜像/域名漂移时在此修改)
    pub base: String,
    /// 登录后从浏览器复制的 Cookie(JavDB 的 FC2 等详情页要求登录);空则不发送
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cookie: String,
    /// DMM 公开数字影片 GraphQL 完整端点；空时用内置默认。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_base: String,
    /// 该站使用的代理;默认跟随全局 `proxy`,保存时省略
    #[serde(default, skip_serializing_if = "SourceProxy::is_global")]
    pub proxy: SourceProxy,
    /// 该站的文本请求改走用户的求解器(ADR-0007);默认关闭,保存时省略
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solver: bool,
}

/// 用户自己运行的 FlareSolverr v1(ADR-0007)。地址为空时这项能力不存在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverCfg {
    /// 完整地址,例如 `http://127.0.0.1:8191/v1`
    #[serde(default)]
    pub url: String,
    /// 单次求解最多等多久(秒)
    #[serde(default = "default_solver_timeout")]
    pub timeout_secs: u64,
}

fn default_solver_timeout() -> u64 {
    60
}

impl Default for SolverCfg {
    fn default() -> Self {
        Self { url: String::new(), timeout_secs: default_solver_timeout() }
    }
}

/// 单个数据源的代理选择。DMM 需要日本出口而其他站点直连时,不必再二选一。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum SourceProxy {
    #[default]
    Global,
    Direct,
    Custom { url: String },
}

impl SourceProxy {
    fn is_global(&self) -> bool { *self == Self::Global }
}

impl SourceCfg {
    pub fn effective_api_base(&self) -> &str {
        if self.id == "dmm" && self.api_base.trim().is_empty() { crate::sources::dmm::DEFAULT_API } else { self.api_base.trim() }
    }
}

/// Plex 联动设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlexCfg {
    /// 整理/撤销后通知 Plex 刷新变动的文件夹
    pub enabled: bool,
    pub url: String,
    pub token: String,
    /// Plex 友好命名(文件名带标题,分集 ` - cd1`)
    pub naming: bool,
}

impl Default for PlexCfg {
    fn default() -> Self {
        Self { enabled: false, url: "http://127.0.0.1:32400".into(), token: String::new(), naming: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub organize_mode: crate::placement::OrganizeMode,
    #[serde(default)]
    pub nfo_language: crate::localization::NfoLanguage,
    #[serde(default)]
    pub name_rules: crate::names::NameRules,
    pub sources: Vec<SourceCfg>,
    /// 内容类型 → 有序站点路由。路由是资格真值:该类型只会请求这里列出的站(且需启用)
    #[serde(default = "default_routes")]
    pub routes: BTreeMap<ContentType, Vec<String>>,
    /// 字段 → 提前尝试的站点;与该类型路由求交后前置,其余路由站保序接上
    #[serde(default)]
    pub field_priority: BTreeMap<String, Vec<String>>,
    /// 字段 → 取值时跳过的站点;与 field_priority 冲突时以此为准
    #[serde(default)]
    pub field_blacklist: BTreeMap<String, Vec<String>>,
    /// 同一 host 请求最小间隔(秒);JavDB PoC 结论 ≥ 5
    pub min_interval_secs: u64,
    pub proxy: Option<String>,
    pub dir_template: String,
    /// 视频文件名模板(不含扩展名),默认保留分集与中字/破解/流出标记;「Plex 友好命名」开启时不生效
    #[serde(default = "default_file_template")]
    pub file_template: String,
    /// 整理时同时下载剧照(最多 12 张);每张图都受同站点限速,会明显变慢
    #[serde(default)]
    pub download_extrafanart: bool,
    /// 整理后把空出来的原文件夹移到废纸篓(只限待整理目录之内,根目录保留)
    #[serde(default = "yes")]
    pub cleanup_source: bool,
    #[serde(default)]
    pub retention: crate::maintenance::RetentionPolicy,
    #[serde(default)]
    pub plex: PlexCfg,
    /// Jellyfin / Emby 联动(ADR-0010),与 Plex 可同时启用
    #[serde(default)]
    pub media_server: crate::media_server::MediaServerCfg,
    /// 运行日志输出到哪里(终端 / App 内 / 文件)
    #[serde(default)]
    pub log: crate::logging::LogCfg,
    /// 扫描时哪些视频不算正片(样片/预告、广告黑名单、过小)
    #[serde(default)]
    pub scan: crate::video_filter::ScanCfg,
    /// 封面角标:按文件名后缀在海报四角标「中字」「破解」「流出」「无码」
    #[serde(default)]
    pub badges: crate::badge::BadgeCfg,
    /// AI 汉化(日文标题/简介 → 简体中文)
    #[serde(default)]
    pub ai: crate::translate::AiCfg,
    /// 媒体库根目录(整理产物与索引所在);未配置时客户端走首次引导
    #[serde(default)]
    pub library_root: Option<String>,
    /// 上次使用的待整理目录
    #[serde(default)]
    pub source_dirs: Vec<String>,
    /// 长任务(批量刮削、整理、重建)结束时窗口不在前台就发系统通知
    #[serde(default = "yes")]
    pub notify_on_finish: bool,
    #[serde(default)]
    pub solver: SolverCfg,
}

fn default_file_template() -> String {
    crate::template::DEFAULT_FILE_TEMPLATE.into()
}

fn yes() -> bool {
    true
}

/// 只列已实现的站;amane 默认有码路由为 dmm→javdb→javbus→official,
/// jav321 在 amane 中是有码/素人的垫后源。新站实现后在此补齐。
fn default_routes() -> BTreeMap<ContentType, Vec<String>> {
    use ContentType::*;
    BTreeMap::from([
        (Censored, vec!["javdb".to_string(), "javbus".to_string(), "jav321".to_string()]),
            (Amateur, vec!["javdb".to_string(), "javbus".to_string(), "jav321".to_string()]),
            (Uncensored, vec!["javdb".to_string(), "javbus".to_string()]),
            (Fc2, vec!["javdb".to_string()]),
            (Chinese, vec!["javdb".to_string()]),
            (Western, vec!["javdb".to_string()]),
            (Hentai, vec!["javdb".to_string()]),
    ])
}

impl Default for Settings {
    fn default() -> Self {
        // 日文标题/原标题/简介/时长以 DMM 系的 Jav321 为准(日文、简介更全、时长精确;JavDB 中文译题质量不稳)
        let field_priority = ["title", "original_title", "plot", "runtime_min"]
            .into_iter()
            .map(|f| (f.to_string(), vec!["jav321".to_string()]))
            .collect();
        Self {
            sources: vec![
                SourceCfg { id: "javdb".into(), enabled: true, base: "https://javdb.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "javbus".into(), enabled: true, base: "https://www.javbus.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "jav321".into(), enabled: true, base: "https://www.jav321.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "dmm".into(), enabled: false, base: "https://www.dmm.co.jp".into(), cookie: String::new(), api_base: crate::sources::dmm::DEFAULT_API.into(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "mgstage".into(), enabled: false, base: "https://www.mgstage.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "fc2".into(), enabled: false, base: "https://adult.contents.fc2.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "fc2ppvdb".into(), enabled: false, base: "https://fc2ppvdb.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "avsox".into(), enabled: false, base: "https://avsox.click".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "javlibrary".into(), enabled: false, base: "https://www.javlibrary.com".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "xcity".into(), enabled: false, base: "https://xcity.jp".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
                SourceCfg { id: "avbase".into(), enabled: false, base: "https://www.avbase.net".into(), cookie: String::new(), api_base: String::new(), proxy: SourceProxy::Global, solver: false },
            ],
            routes: default_routes(),
            name_rules: Default::default(),
            field_priority,
            field_blacklist: BTreeMap::new(),
            min_interval_secs: 5,
            proxy: None,
            dir_template: "{actor}/{code}".into(),
            nfo_language: Default::default(),
            file_template: default_file_template(),
            download_extrafanart: false,
            organize_mode: Default::default(),
            cleanup_source: true,
            retention: Default::default(),
            plex: PlexCfg::default(),
            media_server: Default::default(),
            log: Default::default(),
            scan: Default::default(),
            badges: Default::default(),
            ai: crate::translate::AiCfg::default(),
            library_root: None,
            source_dirs: vec![],
            notify_on_finish: true,
            solver: SolverCfg::default(),
        }
    }
}

impl Settings {
    /// `<系统配置目录>/kiroku/settings.json`
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                let home = std::env::var_os("HOME").or_else(|| std::env::var_os("APPDATA"))?;
                let home = PathBuf::from(home);
                Some(if cfg!(target_os = "macos") { home.join("Library/Application Support") }
                     else if cfg!(windows) { home } else { home.join(".config") })
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("kiroku").join("settings.json")
    }

    /// 文件不存在 → 默认值;存在但损坏 → 报错(不静默覆盖用户配置)。
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice::<Self>(&b).map(Self::with_new_builtin_sources).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// 旧设置文件里没有后来新增的内置数据源:补上(可选新源默认禁用,路由保持用户原值)。
    fn with_new_builtin_sources(mut self) -> Self {
        for d in Self::default().sources {
            if !self.sources.iter().any(|s| s.id == d.id) {
                self.sources.push(d);
            }
        }
        self
    }

    /// 先写临时文件再 rename,避免写一半崩溃留下损坏配置。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        // 文件里有 API Key、Plex Token、站点 Cookie:Unix 上只允许本人读写(0600)
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        io::Write::write_all(&mut opts.open(&tmp)?, &body)?;
        #[cfg(unix)]
        std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        std::fs::rename(tmp, path)
    }

    /// 校验:引用的数据源必须存在;字段名合法;间隔不得为 0;模板非空。
    pub fn validate(&self) -> Result<(), String> {
        self.name_rules.validate()?;
        self.ai.validate()?;
        if let Some(c) = self.sources.iter().find(|c| matches!(&c.proxy, SourceProxy::Custom { url } if reqwest::Proxy::all(url.trim()).is_err() || url.trim().is_empty())) {
            return Err(format!("数据源 {} 的自定义代理地址无效", c.id));
        }
        let solver = self.solver.url.trim();
        if !solver.is_empty() && reqwest::Url::parse(solver).map_or(true, |u| !matches!(u.scheme(), "http" | "https")) {
            return Err("求解器地址必须是 http(s) 地址".into());
        }
        if !solver.is_empty() && self.solver.timeout_secs == 0 {
            return Err("求解器超时必须大于 0".into());
        }
        let known: Vec<&str> = self.sources.iter().map(|s| s.id.as_str()).collect();
        let check = |ids: &[String], what: &str| {
            ids.iter().find(|i| !known.contains(&i.as_str())).map_or(Ok(()), |i| Err(format!("{what} 引用了未知数据源 {i}")))
        };
        for (ct, ids) in &self.routes {
            check(ids, &format!("路由 {ct:?}"))?;
        }
        for (what, map) in [("字段优先级", &self.field_priority), ("字段屏蔽", &self.field_blacklist)] {
            for (f, ids) in map {
                if !crate::aggregate::FIELDS.contains(&f.as_str()) {
                    return Err(format!("{what}:未知字段 {f}"));
                }
                check(ids, &format!("{what} {f}"))?;
            }
        }
        if self.min_interval_secs == 0 {
            return Err("请求间隔不能为 0".into());
        }
        self.scan.validate()?;
        if !(crate::badge::SCALE_MIN..=crate::badge::SCALE_MAX).contains(&self.badges.scale) {
            return Err(format!("角标大小须在 {}%~{}% 之间", crate::badge::SCALE_MIN * 100.0, crate::badge::SCALE_MAX * 100.0));
        }
        if self.dir_template.trim().is_empty() {
            return Err("目录模板不能为空".into());
        }
        crate::template::validate_legacy_template(&self.file_template,true)?;
        crate::template::validate_legacy_template(&self.dir_template,false)?;
        Ok(())
    }

    /// 某内容类型编译后的字段链;路由里未启用的站会被剔除。
    pub fn chains_for(&self, ct: ContentType) -> Chains {
        let enabled: Vec<&str> = self.sources.iter().filter(|s| s.enabled).map(|s| s.id.as_str()).collect();
        let route: Vec<String> = self.routes.get(&ct).into_iter().flatten().filter(|s| enabled.contains(&s.as_str())).cloned().collect();
        compile_priority(&route, &self.field_priority, &self.field_blacklist)
    }

    /// 由设置得出整理计划选项。
    pub fn plan_opts(&self) -> crate::organize::PlanOpts {
        crate::organize::PlanOpts {
            mode: self.organize_mode,
            nfo_language: self.nfo_language,
            create_library: false,
            extrafanart: self.download_extrafanart,
            plex_naming: self.plex.naming,
            file_template: self.file_template.clone(),
            badges: self.badges,
            stamp_dir: Some(crate::badge::user_dir(&Self::default_path())),
        }
    }

    /// 扫描与清理共用的正片过滤器。
    pub fn video_filter(&self) -> crate::video_filter::VideoFilter {
        crate::video_filter::VideoFilter::new(&self.scan)
    }

    /// 按设置实例化启用的数据源。
    pub fn build_sources(&self) -> Vec<Box<dyn Source>> {
        self.sources
            .iter()
            .filter(|c| c.enabled)
            .filter_map(|c| -> Option<Box<dyn Source>> {
                match c.id.as_str() {
                    "dmm" => Some(Box::new(Dmm { base: c.base.clone(), api_base: c.effective_api_base().into(), cookie: c.cookie.trim().into() })),
                    "mgstage" => Some(Box::new(Mgstage { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "fc2" => Some(Box::new(Fc2 { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "fc2ppvdb" => Some(Box::new(Fc2PpvDb { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "javlibrary" => Some(Box::new(JavLibrary { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "xcity" => Some(Box::new(XCity { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "avbase" => Some(Box::new(AvBase { base: c.base.clone() })),
                    "avsox" => Some(Box::new(Avsox { base: c.base.clone(), cookie: c.cookie.trim().into() })),
                    "javdb" => Some(Box::new(JavDb { base: c.base.clone(), cookie: c.cookie.trim().to_string() })),
                    "jav321" => Some(Box::new(Jav321 { base: c.base.clone() })),
                    "javbus" => Some(Box::new(JavBus { base: c.base.clone() })),
                    _ => None,
                }
            })
            .collect()
    }

    pub fn build_http(&self) -> reqwest::Result<Http> {
        Http::new(Duration::from_secs(self.min_interval_secs), self.proxy.as_deref().filter(|p| !p.is_empty()))
    }

    /// 用户给出的详情页 URL 属于哪个已启用来源:只接受 http(s),host 与来源 `base` 相同(忽略 www.)。
    /// 不接受任意地址,避免把请求发往配置之外的站点。
    pub fn source_for_url(&self, url: &str) -> Result<&SourceCfg, String> {
        let host = |u: &str| reqwest::Url::parse(u).ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_ascii_lowercase()));
        let want = host(url).ok_or_else(|| "请输入 http(s) 开头的详情页地址".to_string())?;
        let cfg = self.sources.iter().find(|c| host(&c.base).as_deref() == Some(want.as_str()))
            .ok_or_else(|| format!("{want} 不是已配置的数据源域名"))?;
        if cfg.enabled { Ok(cfg) } else { Err(format!("数据源 {} 未启用", cfg.id)) }
    }

    /// 该来源要走的求解器(地址, 超时);来源没开或地址为空时为 None。
    pub fn solver_for(&self, cfg: &SourceCfg) -> Option<(&str, std::time::Duration)> {
        let url = self.solver.url.trim();
        (cfg.solver && !url.is_empty()).then(|| (url, std::time::Duration::from_secs(self.solver.timeout_secs)))
    }

    /// 该站实际使用的代理:跟随全局、直连或自定义地址。
    pub fn proxy_for<'a>(&'a self, cfg: &'a SourceCfg) -> Option<&'a str> {
        match &cfg.proxy {
            SourceProxy::Global => self.proxy.as_deref().filter(|p| !p.is_empty()),
            SourceProxy::Direct => None,
            SourceProxy::Custom { url } => Some(url.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/settings.json");
        assert_eq!(Settings::load(&p).unwrap(), Settings::default());
        let mut s = Settings::default();
        s.proxy = Some("http://127.0.0.1:7890".into());
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).unwrap(), s);
    }

    #[test]
    fn detail_urls_resolve_only_to_enabled_sources_on_the_same_host() {
        let mut s = Settings::default();
        assert_eq!(s.source_for_url("https://javdb.com/v/BnMY9").unwrap().id, "javdb");
        assert_eq!(s.source_for_url("https://www.javdb.com/v/BnMY9").unwrap().id, "javdb", "www prefix is the same site");
        assert_eq!(s.source_for_url("http://www.jav321.com/video/smpl00001").unwrap().id, "jav321");
        for bad in ["https://evil.example/v/BnMY9", "https://javdb.com.evil.example/v/x", "javascript:alert(1)", "file:///etc/passwd", "not a url"] {
            assert!(s.source_for_url(bad).is_err(), "{bad} must be rejected");
        }
        assert!(s.source_for_url("https://www.mgstage.com/product/product_detail/300MNO-001/").unwrap_err().contains("未启用"));
        s.sources.iter_mut().find(|c| c.id == "javdb").unwrap().base = "https://javdb565.com".into();
        assert_eq!(s.source_for_url("https://javdb565.com/v/x").unwrap().id, "javdb", "mirror domains follow the configured base");
        assert!(s.source_for_url("https://javdb.com/v/x").is_err());
    }

    #[test]
    fn each_source_resolves_its_own_proxy_and_old_files_follow_global() {
        let old: SourceCfg = serde_json::from_str(r#"{"id":"javdb","enabled":true,"base":"https://javdb.com"}"#).unwrap();
        assert_eq!(old.proxy, SourceProxy::Global);
        let mut s = Settings { proxy: Some("http://global:1".into()), ..Settings::default() };
        let cfg = |s: &Settings, id: &str| s.sources.iter().find(|c| c.id == id).unwrap().clone();
        assert_eq!(s.proxy_for(&cfg(&s, "javdb")), Some("http://global:1"));
        s.sources.iter_mut().find(|c| c.id == "javdb").unwrap().proxy = SourceProxy::Direct;
        s.sources.iter_mut().find(|c| c.id == "dmm").unwrap().proxy = SourceProxy::Custom { url: "http://jp:2".into() };
        assert_eq!(s.proxy_for(&cfg(&s, "javdb")), None);
        assert_eq!(s.proxy_for(&cfg(&s, "dmm")), Some("http://jp:2"));
        assert_eq!(s.proxy_for(&cfg(&s, "jav321")), Some("http://global:1"));
        // Global mode is omitted on save so old readers and files stay unchanged.
        let json = serde_json::to_value(&s).unwrap();
        let saved = |id: &str| json["sources"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone();
        assert!(saved("jav321").get("proxy").is_none());
        assert_eq!(saved("dmm")["proxy"], serde_json::json!({"mode":"custom","url":"http://jp:2"}));
        assert_eq!(serde_json::from_value::<Settings>(json).unwrap(), s);
        s.sources.iter_mut().find(|c| c.id == "dmm").unwrap().proxy = SourceProxy::Custom { url: " ".into() };
        assert!(s.validate().unwrap_err().contains("dmm"));
    }

    #[test]
    fn old_settings_file_without_routes_still_loads() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        // 旧版 schema:有 policy、没有 routes
        std::fs::write(&p, r#"{"sources":[{"id":"javdb","enabled":true,"base":"https://javdb.com"}],
            "policy":{"default":["javdb"],"per_field":{}},"min_interval_secs":7,"proxy":null,"dir_template":"{code}"}"#).unwrap();
        let s = Settings::load(&p).unwrap();
        assert_eq!(s.min_interval_secs, 7);
        assert_eq!(s.routes, default_routes());
        assert!(s.notify_on_finish, "旧设置缺省开启完成通知");
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private_and_keeps_cookie() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        let mut s = Settings::default();
        s.sources[0].cookie = "a=1".into();
        s.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Settings::load(&p).unwrap().sources[0].cookie, "a=1");
        // 覆盖已存在、权限宽松的旧文件后也收紧
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        s.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        assert!(Settings::load(&p).is_err());
    }

    #[test]
    fn validation() {
        let mut s = Settings::default();
        assert!(s.validate().is_ok());
        s.field_priority.insert("title".into(), vec!["nope".into()]);
        assert!(s.validate().unwrap_err().contains("nope"));
        let mut s = Settings::default();
        s.routes.insert(ContentType::Fc2, vec!["ghost".into()]);
        assert!(s.validate().unwrap_err().contains("ghost"));
        let mut s = Settings::default();
        s.field_blacklist.insert("not_a_field".into(), vec![]);
        assert!(s.validate().is_err());
        let mut s = Settings::default();
        s.min_interval_secs = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn chains_follow_route_enabled_and_priority() {
        let mut s = Settings::default();
        // 默认:有码路由 javdb→javbus→jav321,标题提前 jav321
        let c = s.chains_for(ContentType::Censored);
        assert_eq!(c.route, ["javdb", "javbus", "jav321"]);
        assert_eq!(c.chain("title"), ["jav321", "javdb", "javbus"]);
        assert_eq!(c.chain("studio"), ["javdb", "javbus", "jav321"]);
        // 无码路由里没有 jav321:prefer 与 route 求交后无效
        assert_eq!(s.chains_for(ContentType::Uncensored).chain("title"), ["javdb", "javbus"]);
        // 关闭 jav321 后从所有链剔除
        s.sources.iter_mut().find(|x| x.id == "jav321").unwrap().enabled = false;
        assert_eq!(s.chains_for(ContentType::Censored).chain("title"), ["javdb", "javbus"]);
        // 屏蔽优先于提前
        let mut s = Settings::default();
        s.field_blacklist.insert("title".into(), vec!["jav321".into()]);
        assert_eq!(s.chains_for(ContentType::Censored).chain("title"), ["javdb", "javbus"]);
        // 路由缺失的类型没有任何站
        s.routes.remove(&ContentType::Western);
        assert!(s.chains_for(ContentType::Western).route.is_empty());
    }

    #[test]
    fn default_builds_all_builtin_sources() {
        assert_eq!(Settings::default().build_sources().len(), 3);
    }

    #[test]
    fn old_settings_gain_new_builtin_sources() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        let mut s = Settings::default();
        s.sources.retain(|x| x.id != "javbus"); // 模拟升级前只有两个源
        s.save(&p).unwrap();
        let loaded = Settings::load(&p).unwrap();
        assert!(loaded.sources.iter().any(|x| x.id == "javbus" && x.enabled));
        assert!(loaded.validate().is_ok());
    }
}
