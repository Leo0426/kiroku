//! AI 汉化:把刮削到的日文标题、简介翻译成简体中文。
//!
//! 通过 OpenAI Chat Completions 或 Anthropic Messages 直接发 HTTP。
//! Anthropic 官方预设保留以下默认行为,自定义 provider 可调整:
//! - 用 `output_config.format`(JSON Schema)约束输出,保证能解析;
//! - 翻译是简单任务,`effort` 默认 `low` 以节省费用(Claude Opus 5.5 的思考无法关闭,只能调 effort);
//! - 默认带 `fallbacks: "default"`:模型因安全策略拒绝时,服务端自动换合适的模型重跑同一请求;
//! - 读取内容前先检查 `stop_reason`(`refusal` / `max_tokens` 时不使用输出)。
//!
//! 请求体构造与响应解析是纯函数,便于测试;网络部分在 [`Translator::translate`]。

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::Mutex, time::Duration};

mod config;
mod protocol;
pub use config::{AiCfg, AiProtocol, AiProvider, OutputFormat};
pub use protocol::{build_request, parse_response};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Translated {
    pub title: Option<String>,
    pub plot: Option<String>,
}

/// 是否含假名(平假名/片假名)。只有日文才需要翻译;已经是中文的直接跳过,省钱。
pub fn has_kana(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}'))
}

/// 把 HTTP 错误翻成人话(状态码 → 原因),附上 API 返回的 message。
fn http_error(status: u16, body: &str) -> String {
    let msg = serde_json::from_str::<Value>(body).ok().and_then(|v| v["error"]["message"].as_str().map(String::from)).unwrap_or_default();
    let what = match status {
        400 => "请求无效",
        401 => "API Key 无效",
        403 => "没有权限",
        404 => "模型不存在或地址错误",
        413 => "内容过长",
        429 => "请求太频繁,被限流",
        500..=599 => "服务暂时不可用",
        _ => "请求失败",
    };
    if msg.is_empty() { format!("{what}({status})") } else { format!("{what}({status}):{msg}") }
}

/// 提示词或输出格式变化时递增,使旧译文缓存全部失效。
const PROMPT_VERSION: u32 = 2;
/// 译文缓存最多保留的条目数(标题、简介各算一条)。
const CACHE_ENTRIES: usize = 4096;

/// `<设置目录>/cache/translations.json`
pub fn cache_path(settings_path: &Path) -> PathBuf {
    settings_path.parent().unwrap_or(Path::new(".")).join("cache").join("translations.json")
}

/// 原文、协议、端点、模型、生成选项、提示词版本和字段共同决定译文;不含密钥。
fn cache_key(cfg: &AiProvider, field: &str, text: &str) -> String {
    use sha2::Digest;
    let identity = serde_json::to_string(&(PROMPT_VERSION, cfg.protocol, cfg.endpoint().ok().map(|u| u.to_string()), cfg.model.trim(), &cfg.effort, cfg.output_format, cfg.anthropic_fallback, field, text)).expect("plain tuple");
    format!("{:x}", sha2::Sha256::digest(identity.as_bytes()))
}

#[derive(Serialize, Deserialize)]
struct CacheEntry { text: String, seq: u64 }

#[derive(Serialize, Deserialize, Default)]
struct CacheFile { entries: HashMap<String, CacheEntry> }

/// 译文缓存:只是省钱的缓存,不是磁盘真相。文件损坏或读不了就当空的;写入走临时文件 + rename。
pub struct TranslationCache { path: PathBuf, max: usize, file: CacheFile, next: u64 }

impl TranslationCache {
    pub fn load(path: PathBuf, max: usize) -> Self {
        let file: CacheFile = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let next = file.entries.values().map(|e| e.seq).max().map_or(0, |m| m + 1);
        Self { path, max, file, next }
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.file.entries.get(key).map(|e| e.text.as_str())
    }
    /// 超出上限时淘汰最早写入的条目。
    pub fn insert(&mut self, key: String, text: String) {
        self.file.entries.insert(key, CacheEntry { text, seq: self.next });
        self.next += 1;
        while self.file.entries.len() > self.max {
            let Some(oldest) = self.file.entries.iter().min_by_key(|(_, e)| e.seq).map(|(k, _)| k.clone()) else { break };
            self.file.entries.remove(&oldest);
        }
    }
    pub fn save(&self) -> std::io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".translations-{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(&self.file).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &self.path)
    }
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.file.entries.clear();
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

pub struct Translator {
    cfg: AiCfg,
    provider: AiProvider,
    client: reqwest::Client,
    cache: Option<Mutex<TranslationCache>>,
}

impl Translator {
    pub fn new(cfg: AiCfg) -> Result<Self, String> {
        cfg.validate()?;
        let provider = cfg.selected()?.clone();
        provider.validate()?;
        // 低 effort 的翻译通常几秒到十几秒;给足余量
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
        Ok(Self { cfg, provider, client, cache: None })
    }

    /// 启用译文缓存(桌面与 CLI 使用设置目录下的文件;测试默认不启用,不碰真实目录)。
    pub fn with_cache(mut self, path: PathBuf) -> Self {
        self.cache = Some(Mutex::new(TranslationCache::load(path, CACHE_ENTRIES)));
        self
    }

    fn cached(&self, field: &str, text: Option<&str>) -> Option<String> {
        let cache = self.cache.as_ref()?.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&cache_key(&self.provider, field, text?)).map(String::from)
    }

    /// 只翻译设置里勾选、且含日文假名的字段;都不需要时直接返回空结果,不发请求。
    pub async fn translate(&self, title: Option<&str>, plot: Option<&str>) -> Result<Translated, String> {
        let title = title.filter(|t| self.cfg.title && has_kana(t));
        let plot = plot.filter(|p| self.cfg.plot && has_kana(p));
        if title.is_none() && plot.is_none() {
            return Ok(Translated::default());
        }
        let (cached_title, cached_plot) = (self.cached("title", title), self.cached("plot", plot));
        let (title, plot) = (title.filter(|_| cached_title.is_none()), plot.filter(|_| cached_plot.is_none()));
        if title.is_none() && plot.is_none() {
            log::debug!(target: "kiroku::scrape", "AI 汉化命中译文缓存");
            return Ok(Translated { title: cached_title, plot: cached_plot });
        }
        let fresh = self.request(title, plot).await?;
        if let Some(cache) = &self.cache {
            let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
            for (field, source, out) in [("title", title, &fresh.title), ("plot", plot, &fresh.plot)] {
                if let (Some(source), Some(out)) = (source, out) { cache.insert(cache_key(&self.provider, field, source), out.clone()) }
            }
            if let Err(e) = cache.save() { log::warn!(target: "kiroku::scrape", "译文缓存写入失败: {e}") }
        }
        Ok(Translated { title: cached_title.or(fresh.title), plot: cached_plot.or(fresh.plot) })
    }

    /// 按所选协议请求;只翻译传入的字段。
    async fn request(&self, title: Option<&str>, plot: Option<&str>) -> Result<Translated, String> {
        let url = self.provider.endpoint()?;
        let mut request = self.client.post(url).header("content-type", "application/json");
        request = match self.provider.protocol {
            AiProtocol::Anthropic => {
                let r = request.header("x-api-key", self.provider.api_key.trim()).header("anthropic-version", "2023-06-01");
                if self.provider.anthropic_fallback { r.header("anthropic-beta", "server-side-fallback-2026-07-01") } else { r }
            },
            AiProtocol::Openai => request.bearer_auth(self.provider.api_key.trim()),
        };
        let r = request.body(build_request(&self.provider, title, plot).to_string()).send().await
            .map_err(|e| if e.is_timeout() { "请求超时".to_string() } else { "连不上 AI 服务，请检查地址、网络和 API Key 格式".to_string() })?;
        let status = r.status().as_u16();
        let body = r.text().await.map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(http_error(status, &body));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("响应不是 JSON:{e}"))?;
        let mut t = parse_response(self.provider.protocol, &v)?;
        if (title.is_some() && t.title.is_none()) || (plot.is_some() && t.plot.is_none()) {
            return Err("模型没有返回请求字段的译文".into());
        }
        // 没请求翻译的字段不回填
        if title.is_none() {
            t.title = None;
        }
        if plot.is_none() {
            t.plot = None;
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的 Messages API:每个请求计数并记下请求体,按请求里的非空字段回译文。
    fn fake_api() -> (String, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let log = seen.clone();
        std::thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut bytes = vec![];
            let mut buf = [0; 8192];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 { break }
                bytes.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&bytes).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").and_then(|v| v.parse::<usize>().ok())).unwrap_or(0);
                    if bytes.len() >= end + 4 + len { break }
                }
            }
            let text = String::from_utf8_lossy(&bytes).to_string();
            let request: Value = serde_json::from_str(&text[text.find("\r\n\r\n").unwrap() + 4..]).unwrap();
            let input: Value = serde_json::from_str(request["messages"][0]["content"].as_str().unwrap().split_once('\n').unwrap().1).unwrap();
            log.lock().unwrap().push(input.clone());
            let tr = |k: &str| input[k].as_str().filter(|s| !s.is_empty()).map(|s| format!("译:{s}")).unwrap_or_default();
            let body = json!({"stop_reason":"end_turn","content":[{"type":"text","text": json!({"title": tr("title"), "plot": tr("plot")}).to_string()}]}).to_string();
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        (address, seen)
    }
    fn cfg(base_url: &str) -> AiCfg {
        let mut c = AiCfg { enabled: true, ..Default::default() };
        c.providers[0].api_key = "k".into();
        c.providers[0].base_url = base_url.into();
        c
    }

    #[tokio::test]
    async fn cached_fields_are_not_sent_again_and_only_misses_are_requested() {
        let (url, seen) = fake_api();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/translations.json");
        let t = Translator::new(cfg(&url)).unwrap().with_cache(path.clone());
        assert_eq!(t.translate(Some("タイトル"), Some("あらすじ")).await.unwrap(), Translated { title: Some("译:タイトル".into()), plot: Some("译:あらすじ".into()) });
        assert_eq!(seen.lock().unwrap().len(), 1);
        // A new process reads the same file.
        let again = Translator::new(cfg(&url)).unwrap().with_cache(path.clone());
        assert_eq!(again.translate(Some("タイトル"), Some("あらすじ")).await.unwrap().title.as_deref(), Some("译:タイトル"));
        assert_eq!(seen.lock().unwrap().len(), 1, "both fields came from the cache");
        assert_eq!(again.translate(Some("タイトル"), Some("べつのあらすじ")).await.unwrap(), Translated { title: Some("译:タイトル".into()), plot: Some("译:べつのあらすじ".into()) });
        let last = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!((last["title"].as_str(), last["plot"].as_str()), (Some(""), Some("べつのあらすじ")), "only the miss is sent");
    }

    #[tokio::test]
    async fn model_or_prompt_changes_miss_and_broken_files_are_ignored() {
        let (url, seen) = fake_api();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("translations.json");
        Translator::new(cfg(&url)).unwrap().with_cache(path.clone()).translate(Some("タイトル"), None).await.unwrap();
        let mut other = cfg(&url);
        other.providers[0].model = "other-model".into();
        Translator::new(other).unwrap().with_cache(path.clone()).translate(Some("タイトル"), None).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2, "a different model is a different key");
        assert_ne!(cache_key(&cfg(&url).providers[0], "title", "タイトル"), cache_key(&cfg(&url).providers[0], "plot", "タイトル"));
        std::fs::write(&path, b"{broken").unwrap();
        let t = Translator::new(cfg(&url)).unwrap().with_cache(path.clone());
        assert_eq!(t.translate(Some("タイトル"), None).await.unwrap().title.as_deref(), Some("译:タイトル"));
        assert!(serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).is_ok(), "rebuilt as valid JSON");
    }

    #[tokio::test]
    async fn provider_endpoint_changes_miss_but_key_rotation_reuses_cache() {
        let (first, seen_first) = fake_api();
        let (second, seen_second) = fake_api();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("translations.json");
        Translator::new(cfg(&first)).unwrap().with_cache(path.clone()).translate(Some("海辺の町"), None).await.unwrap();
        Translator::new(cfg(&second)).unwrap().with_cache(path.clone()).translate(Some("海辺の町"), None).await.unwrap();
        assert_eq!(seen_first.lock().unwrap().len(), 1);
        assert_eq!(seen_second.lock().unwrap().len(), 1, "same model at a new service must not reuse old translations");
        let mut rotated = cfg(&second);
        rotated.providers[0].api_key = "rotated-key".into();
        Translator::new(rotated).unwrap().with_cache(path.clone()).translate(Some("海辺の町"), None).await.unwrap();
        assert_eq!(seen_second.lock().unwrap().len(), 1);
        let base = cfg(&second).providers.remove(0);
        let mut changed = base.clone();
        changed.protocol = AiProtocol::Openai;
        assert_ne!(cache_key(&base,"title","海辺の町"),cache_key(&changed,"title","海辺の町"));
        changed = base.clone();
        changed.output_format = OutputFormat::Prompt;
        assert_ne!(cache_key(&base,"title","海辺の町"),cache_key(&changed,"title","海辺の町"));
        changed = base.clone();
        changed.effort = "high".into();
        assert_ne!(cache_key(&base,"title","海辺の町"),cache_key(&changed,"title","海辺の町"));
    }

    #[test]
    fn the_cache_keeps_the_newest_entries_within_its_bound() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = TranslationCache::load(dir.path().join("t.json"), 3);
        for i in 0..5 { cache.insert(format!("k{i}"), format!("v{i}")); }
        assert_eq!(cache.get("k0"), None);
        assert_eq!(cache.get("k1"), None);
        assert_eq!(cache.get("k4"), Some("v4"));
        cache.clear().unwrap();
        assert_eq!(cache.get("k4"), None);
        assert_eq!(TranslationCache::load(dir.path().join("t.json"), 3).get("k4"), None);
    }


    #[test]
    fn kana_detection() {
        assert!(has_kana("彼女のルームメイト"));
        assert!(has_kana("イキ"));
        assert!(!has_kana("带您走进港口小城的故事"));
        assert!(!has_kana("Hello"));
    }

    #[test]
    fn request_shape() {
        let cfg = AiCfg::default();
        let r = build_request(&cfg.providers[0], Some("海辺の約束"), None);
        assert_eq!(r["model"], "claude-opus-5-5");
        assert_eq!(r["fallbacks"], "default");
        assert_eq!(r["output_config"]["effort"], "low");
        assert_eq!(r["output_config"]["format"]["type"], "json_schema");
        assert_eq!(r["output_config"]["format"]["schema"]["additionalProperties"], false);
        assert!(r.get("thinking").is_none(), "Opus 5.5 的思考无法关闭,不发 thinking 参数");
        assert!(r["messages"][0]["content"].as_str().unwrap().contains("海辺の約束"));
    }

    #[test]
    fn parses_text_and_skips_other_blocks() {
        let v = json!({
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "fallback", "from": {"model": "a"}, "to": {"model": "b"}},
                {"type": "text", "text": "{\"title\": \"海边的约定\", \"plot\": \"  \"}"}
            ]
        });
        assert_eq!(parse_response(AiProtocol::Anthropic, &v).unwrap(), Translated { title: Some("海边的约定".into()), plot: None });
    }

    #[test]
    fn refusal_truncation_and_garbage_are_errors() {
        let refusal = json!({"stop_reason": "refusal", "stop_details": {"explanation": "policy"}, "content": []});
        assert!(parse_response(AiProtocol::Anthropic, &refusal).unwrap_err().contains("拒绝"));
        let cut = json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"title\": \"x"}]});
        assert!(parse_response(AiProtocol::Anthropic, &cut).unwrap_err().contains("截断"));
        let bad = json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "不是 JSON"}]});
        assert!(parse_response(AiProtocol::Anthropic, &bad).is_err());
        assert!(parse_response(AiProtocol::Anthropic, &json!({"stop_reason": "end_turn", "content": []})).is_err());
    }

    #[test]
    fn http_errors_are_readable() {
        assert_eq!(http_error(401, r#"{"error":{"message":"invalid x-api-key"}}"#), "API Key 无效(401):invalid x-api-key");
        assert_eq!(http_error(529, ""), "服务暂时不可用(529)");
    }
}
