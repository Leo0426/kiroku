//! Jellyfin / Emby 联动(ADR-0010):整理/撤销后按路径通知服务器,只发出站请求。
//! Jellyfin 接口按官方 OpenAPI 12.2.0 核对:`POST /Library/Media/Updated`(204)、
//! `GET /Library/VirtualFolders`、`GET /System/Info`,鉴权头 `Authorization: MediaBrowser Token="…"`。
//! Emby 使用同名接口与 `X-Emby-Token` 头。

use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, time::Duration};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    #[default]
    Jellyfin,
    Emby,
}

impl ServerKind {
    fn label(self) -> &'static str {
        match self {
            Self::Jellyfin => "Jellyfin",
            Self::Emby => "Emby",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaServerCfg {
    pub enabled: bool,
    pub kind: ServerKind,
    pub url: String,
    pub api_key: String,
    /// 本机路径前缀;服务器在 Docker/NAS 上看到的路径不同时,与 `server_prefix` 一起替换。
    pub local_prefix: String,
    pub server_prefix: String,
}

impl MediaServerCfg {
    pub fn active(&self) -> bool {
        self.enabled && !self.url.trim().is_empty() && !self.api_key.trim().is_empty()
    }

    /// 本机路径 → 服务器路径。按路径组件匹配前缀,不匹配时原样返回。
    pub fn server_path(&self, path: &Path) -> String {
        let (local, server) = (self.local_prefix.trim(), self.server_prefix.trim());
        if !local.is_empty() {
            if let Ok(rest) = path.strip_prefix(local) {
                let server = server.trim_end_matches(['/', '\\']);
                let sep = if server.contains('\\') && !server.contains('/') { "\\" } else { "/" };
                let rest: Vec<_> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                return if rest.is_empty() { server.to_string() } else { format!("{server}{sep}{}", rest.join(sep)) };
            }
        }
        path.to_string_lossy().into_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UpdateType {
    Created,
    Modified,
    Deleted,
}

/// 整理后的目录存在 → Created;撤销后已删除 → Deleted。
pub fn updates(cfg: &MediaServerCfg, dirs: &[PathBuf]) -> serde_json::Value {
    let list: Vec<_> = dirs
        .iter()
        .map(|d| serde_json::json!({"Path": cfg.server_path(d), "UpdateType": if d.exists() { UpdateType::Created } else { UpdateType::Deleted }}))
        .collect();
    serde_json::json!({ "Updates": list })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerLibrary {
    pub name: String,
    pub locations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerTest {
    pub server_name: String,
    pub version: String,
    pub libraries: Vec<ServerLibrary>,
    /// 包含(映射后)媒体库目录的服务器媒体库名
    pub matched: Option<String>,
}

fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build()
}

fn request(cfg: &MediaServerCfg, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let url = format!("{}{path}", cfg.url.trim().trim_end_matches('/'));
    let key = cfg.api_key.trim();
    let builder = client().map_err(|e| e.to_string())?.request(method, url).header("Accept", "application/json");
    Ok(match cfg.kind {
        ServerKind::Jellyfin => builder.header("Authorization", format!("MediaBrowser Token=\"{key}\"")),
        ServerKind::Emby => builder.header("X-Emby-Token", key),
    })
}

/// 发送请求并把状态码映射成可读错误;错误里不含 API Key。
async fn send(cfg: &MediaServerCfg, builder: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
    let name = cfg.kind.label();
    let r = builder.send().await.map_err(|e| format!("连不上 {name}({}):{}", cfg.url.trim(), e.without_url()))?;
    match r.status().as_u16() {
        200..=299 => Ok(r),
        401 => Err(format!("{name} API Key 无效(401)")),
        403 => Err(format!("{name} 拒绝访问(403),请使用管理员 API Key")),
        s => Err(format!("{name} 返回状态 {s}")),
    }
}

pub fn parse_libraries(json: &str) -> Result<Vec<ServerLibrary>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("服务器返回的不是 JSON:{e}"))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|l| ServerLibrary {
            name: l["Name"].as_str().unwrap_or_default().to_string(),
            locations: l["Locations"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect(),
        })
        .collect())
}

/// 找包含 `server_path` 的媒体库(按路径组件比较,取最具体的位置)。
pub fn match_library<'a>(libraries: &'a [ServerLibrary], server_path: &str) -> Option<&'a ServerLibrary> {
    let path = Path::new(server_path);
    libraries
        .iter()
        .filter_map(|l| l.locations.iter().filter(|loc| path.starts_with(loc)).map(|loc| (loc.len(), l)).max_by_key(|(n, _)| *n))
        .max_by_key(|(n, _)| *n)
        .map(|(_, l)| l)
}

/// 测试连接:读服务器信息与媒体库列表,并找出包含媒体库根目录的那个。
pub async fn test(cfg: &MediaServerCfg, library_root: Option<&Path>) -> Result<ServerTest, String> {
    let info = send(cfg, request(cfg, reqwest::Method::GET, "/System/Info")?).await?.text().await.map_err(|e| e.without_url().to_string())?;
    let info: serde_json::Value = serde_json::from_str(&info).map_err(|e| format!("服务器信息不是 JSON:{e}"))?;
    let text = send(cfg, request(cfg, reqwest::Method::GET, "/Library/VirtualFolders")?).await?.text().await.map_err(|e| e.without_url().to_string())?;
    let libraries = parse_libraries(&text)?;
    let matched = library_root.and_then(|root| match_library(&libraries, &cfg.server_path(root)).map(|l| l.name.clone()));
    Ok(ServerTest {
        server_name: info["ServerName"].as_str().unwrap_or_default().to_string(),
        version: info["Version"].as_str().unwrap_or_default().to_string(),
        libraries,
        matched,
    })
}

/// 通知服务器这些目录有变化。返回通知的目录数;失败只报告,不影响整理结果。
pub async fn notify(cfg: &MediaServerCfg, dirs: &[PathBuf]) -> Result<usize, String> {
    if dirs.is_empty() {
        return Ok(0);
    }
    send(cfg, request(cfg, reqwest::Method::POST, "/Library/Media/Updated")?.header("Content-Type", "application/json").body(updates(cfg, dirs).to_string())).await?;
    Ok(dirs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn cfg(kind: ServerKind, url: String) -> MediaServerCfg {
        MediaServerCfg { enabled: true, kind, url, api_key: "secret-key".into(), ..Default::default() }
    }

    /// 一次只应答一个请求,把收到的原始请求交回测试。
    fn serve(responses: Vec<(u16, &'static str)>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut buf = vec![0; 16384];
                let mut got = String::new();
                // 读到头部结束,再按 Content-Length 读完 body
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 { break }
                    got.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if let Some(end) = got.find("\r\n\r\n") {
                        let len = got[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if got.len() >= end + 4 + len { break }
                    }
                }
                let _ = tx.send(got);
                let _ = stream.write_all(format!("HTTP/1.1 {status} T\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        (format!("http://{address}"), rx)
    }

    #[test]
    fn maps_local_prefix_by_components() {
        let c = MediaServerCfg { local_prefix: "/Volumes/NAS/av".into(), server_prefix: "/media/movies/".into(), ..Default::default() };
        assert_eq!(c.server_path(Path::new("/Volumes/NAS/av/A/ABC-123")), "/media/movies/A/ABC-123");
        assert_eq!(c.server_path(Path::new("/Volumes/NAS/av")), "/media/movies");
        assert_eq!(c.server_path(Path::new("/Volumes/NAS/avx/A")), "/Volumes/NAS/avx/A", "前缀按组件比较");
        let win = MediaServerCfg { local_prefix: "/Volumes/NAS".into(), server_prefix: "D:\\NAS".into(), ..Default::default() };
        assert_eq!(win.server_path(Path::new("/Volumes/NAS/A/B")), "D:\\NAS\\A\\B");
    }

    #[test]
    fn existing_dirs_are_created_and_removed_dirs_are_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("gone");
        let body = updates(&MediaServerCfg::default(), &[tmp.path().to_path_buf(), gone.clone()]);
        assert_eq!(body["Updates"][0]["UpdateType"], "Created");
        assert_eq!(body["Updates"][1], serde_json::json!({"Path": gone.to_string_lossy(), "UpdateType": "Deleted"}));
    }

    #[test]
    fn libraries_match_the_most_specific_location() {
        let libs = parse_libraries(r#"[{"Name":"影片","Locations":["/media"]},{"Name":"Movies","Locations":["/media/movies","/mnt/b"]}]"#).unwrap();
        assert_eq!(match_library(&libs, "/media/movies/A").map(|l| l.name.as_str()), Some("Movies"));
        assert_eq!(match_library(&libs, "/media/other").map(|l| l.name.as_str()), Some("影片"));
        assert_eq!(match_library(&libs, "/media2/x"), None);
        assert!(parse_libraries("<html>").is_err());
    }

    #[tokio::test]
    async fn jellyfin_notify_posts_updates_with_the_token_header() {
        let (url, rx) = serve(vec![(204, "")]);
        let mut c = cfg(ServerKind::Jellyfin, url);
        c.local_prefix = "/lib".into();
        c.server_prefix = "/media".into();
        assert_eq!(notify(&c, &[PathBuf::from("/lib/gone/ABC-123")]).await, Ok(1));
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /Library/Media/Updated HTTP/1.1"), "{req}");
        assert!(req.to_ascii_lowercase().contains("authorization: mediabrowser token=\"secret-key\""), "{req}");
        assert!(req.contains(r#"{"Updates":[{"Path":"/media/gone/ABC-123","UpdateType":"Deleted"}]}"#), "{req}");
        assert!(!req.contains("api_key="), "Key 不进 URL");
    }

    #[tokio::test]
    async fn emby_test_reads_info_and_matches_the_library_root() {
        let (url, rx) = serve(vec![
            (200, r#"{"ServerName":"家里","Version":"4.9.1"}"#),
            (200, r#"[{"Name":"Movies","Locations":["/media/movies"]}]"#),
        ]);
        let mut c = cfg(ServerKind::Emby, url);
        c.local_prefix = "/Volumes/NAS/av".into();
        c.server_prefix = "/media/movies".into();
        let t = test(&c, Some(Path::new("/Volumes/NAS/av"))).await.unwrap();
        assert_eq!((t.server_name.as_str(), t.version.as_str(), t.matched.as_deref()), ("家里", "4.9.1", Some("Movies")));
        let first = rx.recv().unwrap();
        assert!(first.starts_with("GET /System/Info"), "{first}");
        assert!(first.to_ascii_lowercase().contains("x-emby-token: secret-key"), "{first}");
        assert!(rx.recv().unwrap().starts_with("GET /Library/VirtualFolders"));
    }

    #[tokio::test]
    async fn auth_errors_are_explained_without_leaking_the_key() {
        let (url, _rx) = serve(vec![(401, ""), (403, ""), (500, "")]);
        let c = cfg(ServerKind::Jellyfin, url);
        let dirs = [PathBuf::from("/x")];
        assert_eq!(notify(&c, &dirs).await, Err("Jellyfin API Key 无效(401)".into()));
        assert!(notify(&c, &dirs).await.unwrap_err().contains("403"));
        assert_eq!(notify(&c, &dirs).await, Err("Jellyfin 返回状态 500".into()));
        let dead = cfg(ServerKind::Emby, "http://127.0.0.1:9".into());
        let e = notify(&dead, &dirs).await.unwrap_err();
        assert!(e.starts_with("连不上 Emby") && !e.contains("secret-key"), "{e}");
    }
}
