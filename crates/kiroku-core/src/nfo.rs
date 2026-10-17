//! Kodi `<movie>` NFO,供 Emby / Jellyfin / Kodi 读取。字段与写法对齐 amane `media/nfo.py`。

use crate::metadata::Metadata;

#[path = "nfo_edit.rs"]
mod edit;
pub use edit::{merge_revision,merge_language_export};

/// 先剥非法 XML 字符(存量脏数据兜底),再转义;换行原样保留。
fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| matches!(c, '\t' | '\n' | '\r') || (*c >= ' ' && !matches!(*c, '\u{FFFE}' | '\u{FFFF}')))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

pub fn to_nfo(m: &Metadata) -> String {
    let projected=crate::localization::project(m,&m.nfo_language);
    let m=&projected.metadata;
    let f = &m.fields;
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<movie>\n");
    let mut line = |s: String| {
        x.push_str("  ");
        x.push_str(&s);
        x.push('\n');
    };
    let nonempty = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    // plot 按纯文本写出,不用 CDATA:转义后 "]]>" 不可能出现
    if let Some(p) = nonempty(&f.plot) {
        line(format!("<plot>{}</plot>", esc(&p)));
        line(format!("<outline>{}</outline>", esc(&p)));
    }
    if let Some(d) = nonempty(&f.release_date) {
        line(format!("<premiered>{}</premiered>", esc(&d)));
        line(format!("<releasedate>{}</releasedate>", esc(&d)));
        if let Some(y) = m.year() {
            line(format!("<year>{}</year>", esc(y)));
        }
    }
    let num = esc(&m.code);
    line(format!("<num>{num}</num>"));
    // 标题带番号前缀,便于媒体库按番号排序
    let explicit_target=match m.nfo_language {crate::localization::NfoLanguage::Current=>false,crate::localization::NfoLanguage::Original=>m.text_versions.original.title.is_some(),crate::localization::NfoLanguage::ZhHans=>m.text_versions.localized.iter().find(|(tag,_)|tag.eq_ignore_ascii_case("zh-Hans")).is_some_and(|(_,v)|v.title.is_some())};
    let title = nonempty(&f.title).or_else(|| if m.locked_fields.contains("title") || explicit_target { None } else { nonempty(&f.original_title) });
    if let Some(t) = &title {
        line(format!("<title>{num} {}</title>", esc(t)));
        line(format!("<sorttitle>{num} {}</sorttitle>", esc(t)));
    }
    else {line(format!("<title>{num}</title>"));line(format!("<sorttitle>{num}</sorttitle>"));}
    if let Some(orig) = nonempty(&f.original_title) {
        line(format!("<originaltitle>{num} {}</originaltitle>", esc(&orig)));
    }
    line("<mpaa>JP-18+</mpaa>".into());
    for (order, a) in f.actors.iter().enumerate() {
        let thumb = a.thumb.as_deref().map(|t| format!("\n    <thumb>{}</thumb>", esc(t))).unwrap_or_default();
        line(format!("<actor>\n    <name>{}</name>\n    <type>Actor</type>\n    <order>{order}</order>{thumb}\n  </actor>", esc(&a.name)));
    }
    if let Some(score) = m.mean_score() {
        // 内部为 0–100:Kodi rating 为 0–10,criticrating 为 0–100
        line(format!("<rating>{:.1}</rating>", score / 10.0));
        line(format!("<criticrating>{}</criticrating>", score.round() as i32));
    }
    if let Some(r) = f.runtime_min {
        line(format!("<runtime>{r}</runtime>"));
    }
    if let Some(s) = nonempty(&f.series) {
        line(format!("<series>{}</series>", esc(&s)));
        line(format!("<set>\n    <name>{}</name>\n  </set>", esc(&s)));
    }
    if let Some(s) = nonempty(&f.studio) {
        line(format!("<studio>{}</studio>", esc(&s)));
        line(format!("<maker>{}</maker>", esc(&s)));
    }
    if let Some(p) = nonempty(&f.publisher) {
        line(format!("<publisher>{}</publisher>", esc(&p)));
        line(format!("<label>{}</label>", esc(&p)));
    }
    for t in f.tags.iter().filter(|t| !t.is_empty()) {
        line(format!("<tag>{}</tag>", esc(t)));
        line(format!("<genre>{}</genre>", esc(t)));
    }
    if let Some(u) = f.poster_urls.first() {
        line(format!("<poster>{}</poster>", esc(u)));
    }
    if let Some(u) = f.thumb_urls.first() {
        line(format!("<cover>{}</cover>", esc(u)));
    }
    if let Some(u) = f.trailer_urls.first() {
        line(format!("<trailer>{}</trailer>", esc(u)));
    }
    for d in &f.directors {
        line(format!("<director>{}</director>", esc(d)));
    }
    for (site, id) in &m.external_ids {
        line(format!("<{site}id>{}</{site}id>", esc(id)));
    }
    x.push_str("</movie>\n");
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Actor, Gender, PartialMetadata, SourcedScore};

    fn sample() -> Metadata {
        Metadata {
            code: "ABC-123".into(),
            fields: PartialMetadata {
                title: Some("A & <B>".into()),
                original_title: Some("原題".into()),
                release_date: Some("2024-05-01".into()),
                publisher: Some("Label".into()),
                studio: Some("Studio".into()),
                tags: vec!["t1".into()],
                directors: vec!["D".into()],
                actors: vec![Actor::new("Foo", Gender::Female)],
                series: Some("S".into()),
                ..Default::default()
            },
            scores: vec![SourcedScore { site: "a".into(), score: 80.0 }, SourcedScore { site: "b".into(), score: 90.0 }],
            external_ids: [("javdb".to_string(), "ZY5eq".to_string())].into(),
            ..Default::default()
        }
    }

    #[test]
    fn escapes_prefixes_title_and_skips_empty() {
        let x = to_nfo(&sample());
        assert!(x.contains("<title>ABC-123 A &amp; &lt;B&gt;</title>"));
        assert!(x.contains("<originaltitle>ABC-123 原題</originaltitle>"));
        assert!(x.contains("<sorttitle>ABC-123 A &amp; &lt;B&gt;</sorttitle>"));
        assert!(x.contains("<year>2024</year>") && x.contains("<releasedate>2024-05-01</releasedate>"));
        assert!(!x.contains("<plot>") && !x.contains("<trailer>"));
    }

    #[test]
    fn amane_dual_writes_and_extras() {
        let x = to_nfo(&sample());
        for t in ["<tag>t1</tag>", "<genre>t1</genre>", "<maker>Studio</maker>", "<label>Label</label>", "<publisher>Label</publisher>",
                  "<series>S</series>", "<name>S</name>", "<director>D</director>", "<javdbid>ZY5eq</javdbid>", "<mpaa>JP-18+</mpaa>",
                  "<type>Actor</type>"] {
            assert!(x.contains(t), "缺少 {t}");
        }
        // 平均分 85 → rating 8.5 / criticrating 85
        assert!(x.contains("<rating>8.5</rating>") && x.contains("<criticrating>85</criticrating>"));
    }

    #[test]
    fn strips_illegal_xml_chars() {
        let mut m = sample();
        m.fields.plot = Some("a\u{0008}b".into());
        assert!(to_nfo(&m).contains("<plot>ab</plot>"));
    }
}
