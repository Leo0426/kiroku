//! 读取 Kodi/Jellyfin 风格的 `movie.nfo`(含外部工具生成的),用于重建媒体库索引。
//! 只取库浏览需要的字段;外部 NFO 的信息天然不完整,缺失处保持为空。

use crate::metadata::{Actor, Gender, Metadata, SourcedScore};
use quick_xml::{events::Event, Reader};

/// 解析失败返回 None。`fallback_code` 在 NFO 没有 `<num>` 时作番号(如目录名)。
pub fn parse_nfo(xml: &str, fallback_code: &str) -> Option<Metadata> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut path: Vec<String> = vec![];
    let mut contents: Vec<String> = vec![];
    let mut aspects: Vec<Option<String>> = vec![];
    let mut m = Metadata::default();
    let (mut num, mut rating) = (None::<String>, None::<f32>);
    let f = &mut m.fields;
    let mut saw_movie = false;
    loop {
        match r.read_event().ok()? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                saw_movie |= name == "movie";
                path.push(name);
                contents.push(String::new());
                aspects.push(e.attributes().filter_map(Result::ok).find(|a| a.key.as_ref()==b"aspect").and_then(|a|a.unescape_value().ok().map(|v|v.into_owned())));
            }
            Event::Text(t) => {
                if let Some(content)=contents.last_mut() {content.push_str(&t.unescape().ok()?);}
            }
            Event::CData(t) => {
                if let Some(content)=contents.last_mut() {content.push_str(std::str::from_utf8(t.as_ref()).ok()?);}
            }
            Event::End(_) => {
                let text = contents.pop()?.trim().to_string();
                let aspect = aspects.pop()?;
                if text.is_empty() {path.pop(); continue;}
                let here: Vec<&str> = path.iter().map(String::as_str).collect();
                match here.as_slice() {
                    ["movie", "num"] => num = Some(text),
                    ["movie", "title"] => f.title = Some(text),
                    ["movie", "originaltitle"] => f.original_title = Some(text),
                    ["movie", "plot"] => f.plot = Some(text),
                    ["movie", "premiered"] | ["movie", "releasedate"] => {
                        f.release_date.get_or_insert(text);
                    }
                    ["movie", "year"] => {
                        // 没有 premiered 时用 year 兜底(只取年)
                        if f.release_date.is_none() {
                            f.release_date = Some(text);
                        }
                    }
                    ["movie", "runtime"] => f.runtime_min = text.parse().ok(),
                    ["movie", "studio"] | ["movie", "maker"] => {
                        f.studio.get_or_insert(text);
                    }
                    ["movie", "publisher"] | ["movie", "label"] => {
                        f.publisher.get_or_insert(text);
                    }
                    ["movie", "series"] => f.series = Some(text),
                    ["movie", "set"] | ["movie", "set", "name"] => {
                        f.series.get_or_insert(text);
                    }
                    ["movie", "genre"] | ["movie", "tag"] => {
                        if !f.tags.contains(&text) {
                            f.tags.push(text);
                        }
                    }
                    ["movie", "director"] => f.directors.push(text),
                    ["movie", "actor", "name"] => f.actors.push(Actor::new(text, Gender::Unknown)),
                    ["movie", "actor", "thumb"] => if let Some(a) = f.actors.last_mut() { a.thumb = Some(text) },
                    ["movie", "rating"] => rating = text.parse().ok(),
                    ["movie", "poster"] => f.poster_urls.push(text),
                    ["movie", "cover"] => f.thumb_urls.push(text),
                    ["movie", "trailer"] => f.trailer_urls.push(text),
                    ["movie", "thumb"] => if aspect.as_deref()==Some("poster") { f.poster_urls.push(text) } else { f.thumb_urls.push(text) },
                    _ => {}
                }
                path.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_movie {
        return None;
    }
    // amane/kiroku 的 <title> 带「番号 」前缀,索引里去掉,避免卡片上重复
    let code = num.unwrap_or_else(|| fallback_code.to_string());
    let strip = |s: Option<String>| s.map(|t| t.strip_prefix(&format!("{code} ")).map(str::to_string).unwrap_or(t));
    f.title = strip(f.title.take());
    f.original_title = strip(f.original_title.take());
    // Kodi rating 为 0–10,内部为 0–100
    if let Some(r) = rating {
        m.scores.push(SourcedScore { site: "nfo".into(), score: r * 10.0 });
    }
    m.code = code;
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{metadata::PartialMetadata, nfo::to_nfo};

    #[test]
    fn roundtrips_our_own_nfo() {
        let m = Metadata {
            code: "ABC-123".into(),
            fields: PartialMetadata {
                title: Some("A & <B>".into()),
                original_title: Some("原題".into()),
                plot: Some("line1\nline2".into()),
                release_date: Some("2024-05-01".into()),
                runtime_min: Some(120),
                studio: Some("S".into()),
                publisher: Some("P".into()),
                series: Some("Ser".into()),
                tags: vec!["t1".into(), "t2".into()],
                directors: vec!["D".into()],
                actors: vec![Actor::new("Foo", Gender::Female)],
                ..Default::default()
            },
            scores: vec![SourcedScore { site: "a".into(), score: 85.0 }],
            ..Default::default()
        };
        let back = parse_nfo(&to_nfo(&m), "fallback").unwrap();
        assert_eq!(back.code, "ABC-123");
        assert_eq!(back.fields.title.as_deref(), Some("A & <B>"), "番号前缀与转义都应还原");
        assert_eq!(back.fields.original_title.as_deref(), Some("原題"));
        assert_eq!(back.fields.plot.as_deref(), Some("line1\nline2"));
        assert_eq!(back.fields.release_date.as_deref(), Some("2024-05-01"));
        assert_eq!((back.fields.runtime_min, back.fields.studio.as_deref(), back.fields.publisher.as_deref()), (Some(120), Some("S"), Some("P")));
        assert_eq!(back.fields.series.as_deref(), Some("Ser"));
        assert_eq!(back.fields.tags, ["t1", "t2"], "tag 与 genre 双写不应重复");
        assert_eq!(back.fields.actors[0].name, "Foo");
        assert!((back.mean_score().unwrap() - 85.0).abs() < 0.6);
    }

    #[test]
    fn foreign_nfo_and_garbage() {
        let foreign = r#"<?xml version="1.0"?><movie><title>Hello</title><year>2020</year><actor><name>X</name><role>r</role></actor></movie>"#;
        let m = parse_nfo(foreign, "DIR-001").unwrap();
        assert_eq!((m.code.as_str(), m.fields.title.as_deref(), m.fields.release_date.as_deref()), ("DIR-001", Some("Hello"), Some("2020")));
        assert_eq!(m.fields.actors.len(), 1);
        assert!(parse_nfo("<tvshow><title>x</title></tvshow>", "c").is_none());
        assert!(parse_nfo("not xml <<<", "c").is_none());
    }
}
