//! HTTP 客户端 + 按 host 限速 + 失败重试。
//! PoC(docs/adr/0001)结论:JavDB 的瓶颈是按 IP 的频率限制,同 host 间隔需 ≥ 5s;
//! 底层暂用 reqwest,TLS 指纹伪装(wreq)留作 Cloudflare 升级时的替换项。
//!
//! 重试对齐 amane `net/http.py`:超时、连接中断等传输错误与 408/429/503/504 才重试,
//! 总共最多发 [`Retry::attempts`] 次;第 n 次重试前等 (3n + 2) 秒(2s、5s…),服务器给了更长的
//! `Retry-After` 则按它(最多 30s)。正文读到一半断开也算传输错误,整次请求重来。
//! 每次尝试都重新经过限速,不会因为重试而加快对同一站点的请求。

use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

/// JavDB PoC 验证过的安全间隔。
pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// 值得重试的 HTTP 状态:请求超时、频率限制、服务暂不可用、网关超时。
const RETRYABLE_STATUS: [u16; 4] = [408, 429, 503, 504];

/// 服务器要求的 `Retry-After` 最多等这么久,再长就当次失败。
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// 一次文本响应。
pub struct Page {
    pub status: u16,
    pub url: String,
    pub body: String,
}

/// 重试策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// 总尝试次数(含第一次);1 为不重试
    pub attempts: u32,
    /// 等待时间单位:第 n 次重试前等 (3n + 2) × unit;测试用 0
    pub unit: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self { attempts: 3, unit: Duration::from_secs(1) }
    }
}

/// 一次尝试的结果,供 [`retry_wait`] 判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 收到响应(状态码,服务器给的 Retry-After)
    Status(u16, Option<Duration>),
    /// 传输错误;`retryable` 为超时、连接或正文中断这类偶发错误(URL 写错等不重试)
    Transport { retryable: bool },
}

/// 纯判断:`attempt`(从 0 计)这次的结果是否要重试,要则返回等待时间。
pub fn retry_wait(outcome: Outcome, attempt: u32, policy: Retry) -> Option<Duration> {
    if attempt + 1 >= policy.attempts {
        return None;
    }
    let backoff = policy.unit * (3 * attempt + 2);
    match outcome {
        Outcome::Status(s, after) if RETRYABLE_STATUS.contains(&s) => match after {
            Some(a) if a > MAX_RETRY_AFTER => None,
            Some(a) => Some(backoff.max(a)),
            None => Some(backoff),
        },
        Outcome::Transport { retryable: true } => Some(backoff),
        _ => None,
    }
}

fn retryable(e: &reqwest::Error) -> bool {
    !(e.is_builder() || e.is_redirect())
}

/// `Retry-After` 的秒数形式(HTTP 日期形式少见,忽略)。
fn retry_after(r: &reqwest::Response) -> Option<Duration> {
    r.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok().map(Duration::from_secs)
}

/// 读完整个响应:(状态码, 最终 URL, 正文, Retry-After)。
struct Raw {
    status: u16,
    url: String,
    body: Vec<u8>,
    retry_after: Option<Duration>,
}

pub(crate) const IMAGE_RESPONSE_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    #[error("图片响应超过 {0} 字节上限")]
    TooLarge(usize),
}

fn client(jar: &Arc<reqwest::cookie::Jar>, proxy: Option<&str>) -> reqwest::Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0 Safari/537.36")
        .cookie_provider(jar.clone())
        .timeout(Duration::from_secs(20));
    if let Some(p) = proxy {
        b = b.proxy(reqwest::Proxy::all(p)?);
    }
    b.build()
}

/// 用户自己运行的 FlareSolverr v1(ADR-0007)。kiroku 只调用,不打包、不启动。
#[derive(Clone)]
pub struct Solver {
    endpoint: String,
    timeout: Duration,
    /// 来源的代理,交给求解器出站
    proxy: Option<String>,
    client: reqwest::Client,
}

impl Solver {
    /// `endpoint` 是完整地址,例如 `http://127.0.0.1:8191/v1`。
    pub fn new(endpoint: &str, timeout: Duration, proxy: Option<&str>) -> reqwest::Result<Self> {
        // 求解器在用户本机或局域网,直连;它自己要等挑战,留出比 maxTimeout 多一点的余量
        let client = reqwest::Client::builder().no_proxy().timeout(timeout + Duration::from_secs(15)).build()?;
        Ok(Self { endpoint: endpoint.trim().into(), timeout, proxy: proxy.map(String::from), client })
    }

    fn command(&self, url: &str, post: Option<&str>, cookies: &[(String, String)]) -> String {
        let mut cmd = serde_json::json!({
            "cmd": if post.is_some() { "request.post" } else { "request.get" },
            "url": url,
            "maxTimeout": self.timeout.as_millis() as u64,
        });
        if let Some(data) = post { cmd["postData"] = data.into(); }
        if !cookies.is_empty() {
            cmd["cookies"] = cookies.iter().map(|(n, v)| serde_json::json!({ "name": n, "value": v })).collect();
        }
        if let Some(p) = &self.proxy { cmd["proxy"] = serde_json::json!({ "url": p }); }
        cmd.to_string()
    }
}

fn split_cookies(header: &str) -> Vec<(String, String)> {
    header.split(';').filter_map(|c| c.trim().split_once('=')).map(|(n, v)| (n.trim().into(), v.trim().into())).collect()
}

/// 求解器返回的一个 Cookie(域名为空时用请求的 host)。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct SolvedCookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub path: String,
}

/// 求解结果:目标站的状态码、最终 URL、正文和 Cookie。
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub status: u16,
    pub url: String,
    pub body: String,
    pub cookies: Vec<SolvedCookie>,
}

/// 纯函数:解析 FlareSolverr v1 的响应;求解器报错或不是 FlareSolverr 响应时给出原因。
pub fn parse_solution(solver_status: u16, body: &[u8]) -> Result<Solved, String> {
    #[derive(serde::Deserialize)]
    struct Reply { status: String, #[serde(default)] message: String, solution: Option<Solution> }
    #[derive(serde::Deserialize)]
    struct Solution { url: String, status: u16, #[serde(default)] response: String, #[serde(default)] cookies: Vec<SolvedCookie> }
    match serde_json::from_slice::<Reply>(body) {
        Ok(Reply { status, solution: Some(s), .. }) if status == "ok" => Ok(Solved { status: s.status, url: s.url, body: s.response, cookies: s.cookies }),
        Ok(r) => Err(if r.message.is_empty() { format!("状态 {}", r.status) } else { r.message }),
        Err(_) => Err(format!("HTTP {solver_status},不是 FlareSolverr 响应")),
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    jar: Arc<reqwest::cookie::Jar>,
    min_interval: Duration,
    retry: Retry,
    /// 设置后文本请求改走求解器(图片仍直连)
    solver: Option<Arc<Solver>>,
    /// 每个 host 一把锁:等待某站间隔时不阻塞其他站
    last: Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<Option<Instant>>>>>>,
}

impl Http {
    pub fn new(min_interval: Duration, proxy: Option<&str>) -> reqwest::Result<Self> {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        Ok(Self { client: client(&jar, proxy)?, jar, min_interval, retry: Retry::default(), solver: None, last: Default::default() })
    }

    /// 换一个代理(None 为直连),其余共享:Cookie、重试策略和同 host 请求间隔都不因换 client 而重置。
    pub fn with_proxy(&self, proxy: Option<&str>) -> reqwest::Result<Self> {
        Ok(Self { client: client(&self.jar, proxy)?, ..self.clone() })
    }

    /// 文本请求改走求解器(None 为直连);Cookie、重试和同 host 间隔共享。
    pub fn with_solver(&self, solver: Option<Solver>) -> Self {
        Self { solver: solver.map(Arc::new), ..self.clone() }
    }

    /// 换一个重试策略(测试或一次性探测用)。
    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// Artwork keeps the same transport, cookies and retry policy, but never
    /// consumes the scraping host budget. Callers bound image request concurrency.
    pub fn for_images(&self) -> Self {
        Self { client: self.client.clone(), jar: self.jar.clone(), min_interval: Duration::ZERO,
            retry: self.retry, solver: None, last: Default::default() }
    }

    /// 同一 host 的请求至少间隔 `min_interval`(持该 host 的锁等待,并发下同站也串行,不同站互不阻塞)。
    async fn throttle(&self, url: &str) {
        let Some(host) = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)) else { return };
        let slot = self.last.lock().unwrap_or_else(|e| e.into_inner()).entry(host).or_default().clone();
        let mut last = slot.lock().await;
        if let Some(prev) = *last {
            let wait = self.min_interval.saturating_sub(prev.elapsed());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// 限速 + 按策略重试,直到拿到完整响应或放弃。`build` 每次尝试都重新构造请求。
    async fn fetch(&self, url: &str, build: impl Fn() -> reqwest::RequestBuilder) -> reqwest::Result<Raw> {
        match self.fetch_limited(url, build, None).await {
            Ok(raw) => Ok(raw),
            Err(FetchError::Transport(error)) => Err(error),
            Err(FetchError::TooLarge(_)) => unreachable!("unlimited response cannot exceed a byte limit"),
        }
    }

    async fn fetch_limited(&self, url: &str, build: impl Fn() -> reqwest::RequestBuilder, limit: Option<usize>) -> Result<Raw, FetchError> {
        let mut attempt = 0;
        loop {
            self.throttle(url).await;
            let started = Instant::now();
            let res = async {
                let mut r = build().send().await?;
                let (status, final_url, after) = (r.status().as_u16(), r.url().to_string(), retry_after(&r));
                let body = if let Some(limit) = limit {
                    if r.content_length().is_some_and(|size| size > limit as u64) {
                        return Err(FetchError::TooLarge(limit));
                    }
                    let mut body = Vec::new();
                    while let Some(chunk) = r.chunk().await? {
                        if chunk.len() > limit - body.len() { return Err(FetchError::TooLarge(limit)); }
                        let needed = body.len() + chunk.len();
                        if needed > body.capacity() {
                            let capacity = body.capacity().max(8192).saturating_mul(2).max(needed).min(limit);
                            body.reserve_exact(capacity - body.len());
                        }
                        body.extend_from_slice(&chunk);
                    }
                    body
                } else { r.bytes().await?.to_vec() };
                Ok::<_, FetchError>(Raw { status, url: final_url, body, retry_after: after })
            }
            .await;
            let outcome = match &res {
                Ok(raw) => Outcome::Status(raw.status, raw.retry_after),
                Err(FetchError::Transport(e)) => Outcome::Transport { retryable: retryable(e) },
                Err(FetchError::TooLarge(_)) => Outcome::Transport { retryable: false },
            };
            let ms = started.elapsed().as_millis();
            match &res {
                Ok(raw) => log::debug!(target: "kiroku::net", "{url} → {} ({} KB, {ms} ms)", raw.status, raw.body.len() / 1024),
                Err(e) => log::debug!(target: "kiroku::net", "{url} → 出错 {e} ({ms} ms)"),
            }
            match retry_wait(outcome, attempt, self.retry) {
                Some(wait) => {
                    let why = match &res { Ok(raw) => format!("HTTP {}", raw.status), Err(e) => e.to_string() };
                    log::warn!(target: "kiroku::net", "{url} {why},{:.1}s 后重试(第 {}/{} 次)", wait.as_secs_f32(), attempt + 2, self.retry.attempts);
                    tokio::time::sleep(wait).await
                }
                None => return res,
            }
            attempt += 1;
        }
    }

    /// 带额外请求头(如站点要求的年龄验证 Cookie)的 GET。未显式给出 Referer 时补同源 Referer——
    /// 图片站(如 JavBus)校验它,缺失会返回 403。
    async fn get_raw(&self, url: &str, headers: &[(&str, &str)]) -> reqwest::Result<Raw> {
        if let Some(solver) = &self.solver {
            // Cookie 头转成求解器的 cookies;其他请求头 FlareSolverr 不支持
            let (cookie, other): (Vec<_>, Vec<_>) = headers.iter().partition(|(k, _)| k.eq_ignore_ascii_case("cookie"));
            if !other.is_empty() {
                log::debug!(target: "kiroku::net", "{url} 经求解器请求,额外请求头不生效");
            }
            let cookies: Vec<(String, String)> = cookie.iter().flat_map(|(_, v)| split_cookies(v)).collect();
            return self.solve(solver, url, None, cookies).await;
        }
        self.fetch(url, || self.get_request(url, headers)).await
    }

    /// 经求解器请求一次:同 host 间隔和重试按目标站计算,返回的 Cookie 并入会话。
    async fn solve(&self, solver: &Solver, url: &str, post: Option<&str>, mut cookies: Vec<(String, String)>) -> reqwest::Result<Raw> {
        let target = reqwest::Url::parse(url).ok();
        let session = target.as_ref()
            .and_then(|u| reqwest::cookie::CookieStore::cookies(&*self.jar, u))
            .and_then(|h| h.to_str().ok().map(split_cookies))
            .unwrap_or_default();
        // 显式给的 Cookie 优先于会话里的同名 Cookie
        cookies.extend(session.into_iter().filter(|(n, _)| !cookies.iter().any(|(m, _)| m == n)).collect::<Vec<_>>());
        let command = solver.command(url, post, &cookies);
        let raw = self.fetch(url, || solver.client.post(&solver.endpoint).header("Content-Type", "application/json").body(command.clone())).await?;
        // 求解器失败映射为 502 交给来源,按网络失败处理,不回退直连(ADR-0007)
        let solved = match parse_solution(raw.status, &raw.body) {
            Ok(s) => s,
            Err(why) => {
                log::warn!(target: "kiroku::net", "{url} 求解器失败:{why}");
                return Ok(Raw { status: 502, url: url.into(), body: format!("求解器失败:{why}").into_bytes(), retry_after: None });
            }
        };
        if let Some(target) = &target {
            for c in &solved.cookies {
                let domain = if c.domain.is_empty() { String::new() } else { format!("; Domain={}", c.domain) };
                let path = if c.path.is_empty() { "/" } else { &c.path };
                self.jar.add_cookie_str(&format!("{}={}{domain}; Path={path}", c.name, c.value), target);
            }
        }
        Ok(Raw { status: solved.status, url: solved.url, body: solved.body.into_bytes(), retry_after: None })
    }

    fn get_request(&self, url: &str, headers: &[(&str, &str)]) -> reqwest::RequestBuilder {
        let referer = (!headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("referer")))
            .then(|| reqwest::Url::parse(url).ok().and_then(|u| Some(format!("{}://{}/", u.scheme(), u.host_str()?))))
            .flatten();
        let mut req = self.client.get(url);
        if let Some(referer) = referer { req = req.header("Referer", referer); }
        for (key, value) in headers { req = req.header(*key, *value); }
        req
    }

    /// 返回状态码、最终 URL(跟随重定向后)与正文。
    pub async fn get_text(&self, url: &str) -> reqwest::Result<Page> {
        self.get_text_with(url, &[]).await
    }

    pub async fn get_text_with(&self, url: &str, headers: &[(&str, &str)]) -> reqwest::Result<Page> {
        let r = self.get_raw(url, headers).await?;
        Ok(Page { status: r.status, url: r.url, body: String::from_utf8_lossy(&r.body).into_owned() })
    }

    pub async fn get_bytes(&self, url: &str) -> reqwest::Result<(u16, Vec<u8>)> {
        let r = self.get_raw(url, &[]).await?;
        Ok((r.status, r.body))
    }

    /// Download an image with the shared 32 MiB response budget. Cookies,
    /// same-origin Referer, proxy and retry policy use the existing session.
    pub async fn get_image(&self, url: &str) -> std::io::Result<(u16, Vec<u8>)> {
        self.get_image_bytes(url, IMAGE_RESPONSE_LIMIT).await
    }

    pub(crate) async fn get_image_bytes(&self, url: &str, limit: usize) -> std::io::Result<(u16, Vec<u8>)> {
        let raw = self.fetch_limited(url, || self.get_request(url, &[]), Some(limit)).await
            .map_err(std::io::Error::other)?;
        Ok((raw.status, raw.body))
    }

    /// Add configured cookies to the shared jar before session bootstrap. Server Set-Cookie
    /// then replaces stale values; no explicit Cookie header may discard anonymous CSRF cookies.
    pub fn seed_cookies(&self, url: &str, cookies: &str) {
        if let Ok(url) = reqwest::Url::parse(url) {
            for cookie in cookies.split(';').map(str::trim).filter(|c| c.contains('=')) {
                self.jar.add_cookie_str(&format!("{cookie}; Path=/"), &url);
            }
        }
    }

    /// JSON search POST uses the same rate limiter, retry policy and cookie session as GET.
    pub async fn post_json_text(&self, url: &str, body: &serde_json::Value, headers: &[(&str, &str)]) -> reqwest::Result<Page> {
        if self.solver.is_some() {
            // FlareSolverr 只会 GET 和表单 POST;不悄悄直连(ADR-0007)
            return Ok(Page { status: 502, url: url.into(), body: "求解器失败:不支持 JSON 请求,这个来源不能走求解器".into() });
        }
        let r = self.fetch(url, || {
            let mut req = self.client.post(url).header("Content-Type", "application/json").body(body.to_string());
            for (key, value) in headers { req = req.header(*key, *value); }
            req
        }).await?;
        Ok(Page { status: r.status, url: r.url, body: String::from_utf8_lossy(&r.body).into_owned() })
    }

    /// 表单 POST(跟随重定向),返回最终 (状态码, 正文)。站点搜索用,重复提交无副作用,可以重试。
    pub async fn post_form_text(&self, url: &str, form: &[(&str, &str)]) -> reqwest::Result<Page> {
        let r = if let Some(solver) = &self.solver {
            let mut encoded = reqwest::Url::parse("form:x").expect("static url");
            encoded.query_pairs_mut().extend_pairs(form);
            self.solve(solver, url, Some(encoded.query().unwrap_or("")), vec![]).await?
        } else {
            self.fetch(url, || self.client.post(url).form(form)).await?
        };
        Ok(Page { status: r.status, url: r.url, body: String::from_utf8_lossy(&r.body).into_owned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn public_image_download_rejects_oversized_declared_and_streamed_responses() {
        use std::io::{Read, Write};
        for framing in ["declared", "chunked", "eof"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).unwrap();
                if framing == "declared" {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 33554433\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let headers = if framing == "chunked" { "Transfer-Encoding: chunked\r\n" } else { "" };
                    stream.write_all(format!("HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n").as_bytes()).unwrap();
                    let chunk = vec![1; 1024 * 1024];
                    for _ in 0..33 {
                        if framing == "chunked" && stream.write_all(b"100000\r\n").is_err() { break; }
                        if stream.write_all(&chunk).is_err() { break; }
                        if framing == "chunked" && stream.write_all(b"\r\n").is_err() { break; }
                    }
                    if framing == "chunked" { let _ = stream.write_all(b"0\r\n\r\n"); }
                }
            });
            let http = Http::new(Duration::ZERO, None).unwrap().with_retry(Retry { attempts: 3, unit: Duration::ZERO });
            let error = http.get_image(&format!("http://{address}/image.png")).await.unwrap_err();
            server.join().unwrap();
            assert!(error.to_string().contains("图片响应超过"), "{framing}: {error}");
        }
    }

    /// 本地 HTTP 桩:对每个连接回同一个响应,返回 (地址, 已接受连接数)。
    fn counting_server(status: u16) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let (address, hits) = (listener.local_addr().unwrap(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let counter = hits.clone();
        std::thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.read(&mut [0; 4096]);
            let _ = stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").as_bytes());
        });
        (address, hits)
    }

    #[tokio::test]
    async fn waiting_for_one_host_does_not_delay_another() {
        let (server, _) = counting_server(200);
        let port = server.port();
        let (a, b) = (format!("http://127.0.0.1:{port}/a"), format!("http://localhost:{port}/b"));
        let http = Http::new(Duration::from_millis(400), None).unwrap().with_retry(Retry { attempts: 1, unit: Duration::ZERO });
        // b's spacing ends ~300 ms before a's: b must not queue behind a's wait.
        http.get_text(&b).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        http.get_text(&a).await.unwrap();
        let start = Instant::now();
        let timed = |url: String| { let http = http.clone(); async move { http.get_text(&url).await.unwrap(); start.elapsed() } };
        let (_, b_done) = tokio::join!(timed(a.clone()), timed(b.clone()));
        assert!(b_done < Duration::from_millis(300), "a different host is not delayed by another host's wait: {b_done:?}");
        let start = Instant::now();
        let (x, y) = tokio::join!(http.get_text(&a), http.get_text(&a));
        x.unwrap(); y.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(750), "the same host stays spaced: {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn per_source_clients_change_the_route_but_share_host_spacing() {
        let (target, target_hits) = counting_server(200);
        let (proxy, proxy_hits) = counting_server(502);
        let url = format!("http://{target}/page");
        let global = Http::new(Duration::from_millis(400), Some(&format!("http://{proxy}"))).unwrap().with_retry(Retry { attempts: 1, unit: Duration::ZERO });
        let direct = global.with_proxy(None).unwrap();
        assert_eq!(direct.get_text(&url).await.unwrap().status, 200);
        assert_eq!((target_hits.load(std::sync::atomic::Ordering::SeqCst), proxy_hits.load(std::sync::atomic::Ordering::SeqCst)), (1, 0));
        let start = Instant::now();
        assert_eq!(global.get_text(&url).await.unwrap().status, 502);
        assert!(start.elapsed() >= Duration::from_millis(350), "host spacing must hold across per-source clients");
        assert_eq!((target_hits.load(std::sync::atomic::Ordering::SeqCst), proxy_hits.load(std::sync::atomic::Ordering::SeqCst)), (1, 1));
    }

    #[tokio::test]
    async fn public_image_download_retains_cookies_referer_and_retries() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for status in [503, 200] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096];
                let len = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
                assert!(request.contains("cookie: auth=secret"));
                assert!(request.contains("referer: http://127.0.0.1/"));
                stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: 3\r\nConnection: close\r\n\r\nimg").as_bytes()).unwrap();
            }
        });
        let url = format!("http://{address}/image.png");
        let http = Http::new(Duration::ZERO, None).unwrap().with_retry(Retry { attempts: 2, unit: Duration::ZERO });
        http.seed_cookies(&url, "auth=secret");
        assert_eq!(http.get_image(&url).await.unwrap(), (200, b"img".to_vec()));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn image_response_limit_checks_declared_and_streamed_bodies_without_retry() {
        use std::io::{Read, Write};
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n123456789",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n5\r\n56789\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n123456789",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096]; let length = stream.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..length]).to_ascii_lowercase().contains("referer:"));
                let _ = stream.write_all(response.as_bytes());
            });
            let http = Http::new(Duration::ZERO, None).unwrap().with_retry(Retry { attempts: 3, unit: Duration::ZERO }).for_images();
            let result = http.get_image_bytes(&format!("http://{address}/image.png"), 8).await;
            server.join().unwrap();
            assert!(result.is_err(), "declared, chunked and EOF bodies must stop at the byte budget");
            assert!(result.unwrap_err().to_string().contains("图片响应超过"));
        }
    }

    const P: Retry = Retry { attempts: 3, unit: Duration::from_secs(1) };
    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn retries_transient_failures_with_backoff() {
        assert_eq!(retry_wait(Outcome::Status(429, None), 0, P), Some(S(2)));
        assert_eq!(retry_wait(Outcome::Status(503, None), 1, P), Some(S(5)));
        assert_eq!(retry_wait(Outcome::Transport { retryable: true }, 0, P), Some(S(2)));
        assert_eq!(retry_wait(Outcome::Status(504, None), 2, P), None, "第三次之后放弃");
    }

    #[test]
    fn does_not_retry_definite_answers() {
        for s in [200, 302, 403, 404, 500] {
            assert_eq!(retry_wait(Outcome::Status(s, None), 0, P), None, "{s}");
        }
        assert_eq!(retry_wait(Outcome::Transport { retryable: false }, 0, P), None);
        assert_eq!(retry_wait(Outcome::Status(429, None), 0, Retry { attempts: 1, ..P }), None, "1 次即不重试");
    }

    #[test]
    fn honours_retry_after_within_limit() {
        assert_eq!(retry_wait(Outcome::Status(429, Some(S(10))), 0, P), Some(S(10)));
        assert_eq!(retry_wait(Outcome::Status(429, Some(S(1))), 1, P), Some(S(5)), "取退避与 Retry-After 的较大者");
        assert_eq!(retry_wait(Outcome::Status(429, Some(S(120))), 0, P), None, "要等太久就放弃");
    }

    #[tokio::test]
    async fn image_requests_have_an_independent_unthrottled_session() {
        let scrape = Http::new(Duration::from_secs(60), None).unwrap();
        let images = scrape.for_images();
        assert!(Arc::ptr_eq(&scrape.jar, &images.jar), "authentication is retained");
        assert!(!Arc::ptr_eq(&scrape.last, &images.last));
        scrape.throttle("https://example.com/page").await;
        tokio::time::timeout(Duration::from_millis(100), async {
            images.throttle("https://example.com/image1.jpg").await;
            images.throttle("https://example.com/image2.jpg").await;
        }).await.expect("images must not wait for scraping or for each other");
        assert_eq!(scrape.min_interval, Duration::from_secs(60));
        assert_eq!(scrape.last.lock().unwrap().len(), 1);
    }

    /// 端到端:连不上的地址按策略尝试 3 次后返回错误(不需要真实网络,也不绑定本地端口)。
    #[tokio::test]
    async fn unreachable_host_is_tried_three_times() {
        let http = Http::new(Duration::ZERO, None).unwrap().with_retry(Retry { attempts: 3, unit: Duration::ZERO });
        let tries = std::sync::atomic::AtomicU32::new(0);
        let res = http
            .fetch("http://127.0.0.1:9/", || {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                http.client.get("http://127.0.0.1:9/")
            })
            .await;
        assert!(res.is_err());
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
