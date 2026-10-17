//! Plex Media Server 联动:整理/撤销后只刷新变动的文件夹(局部扫描),不用等 Plex 定时扫描。
//! Plex 不读 NFO,它用文件名识别影片、用目录里的 poster.jpg / fanart.jpg 作封面背景,
//! 所以配合「Plex 友好命名」(见 organize::PlanOpts::plex_naming)效果最好。

use serde::Serialize;
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexSection {
    pub key: String,
    pub title: String,
    /// movie / show / artist …
    pub kind: String,
    pub locations: Vec<String>,
}

/// 解析 `/library/sections` 的 JSON。
pub fn parse_sections(json: &str) -> Result<Vec<PlexSection>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("Plex 返回的不是 JSON:{e}"))?;
    let dirs = v["MediaContainer"]["Directory"].as_array().cloned().unwrap_or_default();
    Ok(dirs
        .iter()
        .map(|d| PlexSection {
            key: d["key"].as_str().unwrap_or_default().to_string(),
            title: d["title"].as_str().unwrap_or_default().to_string(),
            kind: d["type"].as_str().unwrap_or_default().to_string(),
            locations: d["Location"].as_array().into_iter().flatten().filter_map(|l| l["path"].as_str().map(String::from)).collect(),
        })
        .collect())
}

/// 找包含 `path` 的分区(多个匹配时取位置最长、最具体的那个)。
pub fn match_section<'a>(sections: &'a [PlexSection], path: &Path) -> Option<&'a PlexSection> {
    sections
        .iter()
        .filter_map(|s| s.locations.iter().filter(|l| path.starts_with(l)).map(|l| (l.len(), s)).max_by_key(|(n, _)| *n))
        .max_by_key(|(n, _)| *n)
        .map(|(_, s)| s)
}

fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build()
}

fn base(url: &str) -> &str {
    url.trim_end_matches('/')
}

pub async fn sections(url: &str, token: &str) -> Result<Vec<PlexSection>, String> {
    let r = client()
        .map_err(|e| e.to_string())?
        .get(format!("{}/library/sections", base(url)))
        .header("Accept", "application/json")
        .header("X-Plex-Token", token)
        .send()
        .await
        .map_err(|e| format!("连不上 Plex({url}):{e}"))?;
    match r.status().as_u16() {
        200 => parse_sections(&r.text().await.map_err(|e| e.to_string())?),
        401 => Err("Plex Token 无效(401)".into()),
        s => Err(format!("Plex 返回状态 {s}")),
    }
}

/// 让 Plex 只扫描某个目录。
pub async fn refresh(url: &str, token: &str, section_key: &str, dir: &Path) -> Result<(), String> {
    let r = client()
        .map_err(|e| e.to_string())?
        .get(format!("{}/library/sections/{section_key}/refresh", base(url)))
        .query(&[("path", dir.to_string_lossy().as_ref())])
        .header("X-Plex-Token", token)
        .send()
        .await
        .map_err(|e| format!("连不上 Plex:{e}"))?;
    if r.status().is_success() { Ok(()) } else { Err(format!("Plex 刷新失败,状态 {}", r.status())) }
}

/// 刷新一批变动的目录:先找分区,再逐个目录局部扫描。返回刷新成功的目录数。
pub async fn refresh_dirs(url: &str, token: &str, dirs: &[std::path::PathBuf]) -> Result<usize, String> {
    let secs = sections(url, token).await?;
    let mut n = 0;
    for d in dirs {
        // 影片目录可能已被撤销删掉:刷新其上级,让 Plex 移除条目
        let target = std::iter::successors(Some(d.as_path()), |p| p.parent()).find(|p| p.exists()).unwrap_or(d);
        let Some(sec) = match_section(&secs, target) else { continue };
        refresh(url, token, &sec.key, target).await?;
        n += 1;
    }
    if n == 0 && !dirs.is_empty() {
        return Err("Plex 里没有包含媒体库目录的分区,请在 Plex 中把媒体库文件夹加入一个影片库".into());
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"MediaContainer":{"size":2,"Directory":[
        {"key":"1","title":"电影","type":"movie","Location":[{"id":1,"path":"/Users/me/Movies"}]},
        {"key":"5","title":"kiroku","type":"movie","Location":[{"id":7,"path":"/Users/me/Movies/kiroku"},{"id":8,"path":"/Volumes/NAS/kiroku"}]}
    ]}}"#;

    #[test]
    fn parses_sections_and_matches_most_specific() {
        let s = parse_sections(JSON).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].locations, vec!["/Users/me/Movies/kiroku", "/Volumes/NAS/kiroku"]);
        let hit = |p: &str| match_section(&s, Path::new(p)).map(|x| x.key.clone());
        assert_eq!(hit("/Users/me/Movies/kiroku/A/ABC-123").as_deref(), Some("5"), "取最具体的分区");
        assert_eq!(hit("/Users/me/Movies/other").as_deref(), Some("1"));
        assert_eq!(hit("/Volumes/NAS/kiroku/x").as_deref(), Some("5"));
        assert_eq!(hit("/elsewhere"), None);
        // 路径按组件比较:/Users/me/Movies2 不应匹配 /Users/me/Movies
        assert_eq!(hit("/Users/me/Movies2/x"), None);
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_sections("<html>").is_err());
        assert_eq!(parse_sections(r#"{"MediaContainer":{}}"#).unwrap(), vec![]);
    }
}
