//! 多源择优,语义对齐 amane `aggregate/engine.py`。不写 DB。
//!
//! - 路由(route)是资格真值:字段链上只会出现 route 内的站。
//! - `prefer` 与 route 求交后前置,其余 route 站保序接上;`exclude` 从该字段链剔除,与 prefer 冲突时 exclude 胜。
//! - 标量字段(含 actors/tags/directors)取链上第一个非空值;URL 类字段沿链拼接去重;
//!   剧照按来源分组;评分保留来源;external_id/source_url 按来源收集。
//! - 演员性别按姓名从其他来源回填;同名重复视为噪声。

use crate::{
    metadata::{Actor, Gender, Metadata, PartialMetadata, SourcedScore},
    source::SourceId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 可聚合/可配置优先级的字段名(设置界面按此列出)。
pub const FIELDS: &[&str] = &[
    "title", "original_title", "plot", "release_date", "runtime_min", "studio", "publisher", "series", "actors",
    "tags", "directors", "poster_urls", "thumb_urls", "trailer_urls", "extrafanart", "score",
];

/// 某个内容类型编译后的字段链。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chains {
    pub route: Vec<String>,
    pub overrides: BTreeMap<String, Vec<String>>,
}

impl Chains {
    pub fn chain(&self, field: &str) -> &[String] {
        self.overrides.get(field).unwrap_or(&self.route)
    }
}

pub fn compile_priority(
    route: &[String],
    prefer: &BTreeMap<String, Vec<String>>,
    exclude: &BTreeMap<String, Vec<String>>,
) -> Chains {
    let in_route: BTreeSet<&str> = route.iter().map(String::as_str).collect();
    let chain_for = |preferred: &[String], skip: &BTreeSet<&str>| -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for s in preferred.iter().filter(|s| in_route.contains(s.as_str())).chain(route.iter()) {
            if !skip.contains(s.as_str()) && seen.insert(s.as_str()) {
                out.push(s.clone());
            }
        }
        out
    };
    let mut overrides = BTreeMap::new();
    for f in FIELDS {
        let p = prefer.get(*f).filter(|v| !v.is_empty());
        let banned: BTreeSet<&str> = exclude.get(*f).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default();
        if p.is_some() || !banned.is_empty() {
            overrides.insert(f.to_string(), chain_for(p.map(Vec::as_slice).unwrap_or(&[]), &banned));
        }
    }
    Chains { route: route.to_vec(), overrides }
}

fn dedupe<T: Clone, K: Ord>(items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut seen = BTreeSet::new();
    items.into_iter().filter(|i| seen.insert(key(i))).collect()
}

/// 发行日期相差超过此天数,视为番号撞车取到了另一部影片。
const DATE_CONFLICT_DAYS: i64 = 365;

/// `YYYY-MM-DD` 或 `YYYY/MM/DD` → 公历日序号;无法解析时为 None,不作为证据。
fn day_number(date: &str) -> Option<i64> {
    let mut it = date.get(..10)?.split(['-', '/']).map(|n| n.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return None }
    // days_from_civil(Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    Some(era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy)
}

/// 番号撞车时某站可能返回同番号的另一部影片。各站发行日期互相比较:
/// 取日期相互一致(相差不超过一年)的最大一组,并列时取路线上靠前的站;组外有日期的结果丢弃并给出提示。
/// 没有日期或只有一个站有日期时不作判断。被丢弃的站快照由调用方保留,可人工重选。
pub fn drop_date_conflicts(parts: Vec<(SourceId, PartialMetadata)>) -> (Vec<(SourceId, PartialMetadata)>, Vec<String>) {
    drop_date_conflicts_around(parts, None)
}

/// 同 [`drop_date_conflicts`];`pinned` 为用户指定的来源时,以它的日期为准。
pub fn drop_date_conflicts_around(parts: Vec<(SourceId, PartialMetadata)>, pinned: Option<SourceId>) -> (Vec<(SourceId, PartialMetadata)>, Vec<String>) {
    let days: Vec<Option<i64>> = parts.iter().map(|(_, p)| p.release_date.as_deref().and_then(day_number)).collect();
    let near = |a: i64, b: i64| (a - b).abs() <= DATE_CONFLICT_DAYS;
    let pinned_day = pinned.and_then(|id| parts.iter().position(|(p, _)| *p == id)).and_then(|i| days[i].map(|d| (i, d)));
    // 以每个有日期的结果为锚统计支持数;max_by_key 取最后一个最大值,所以倒序遍历让并列时路线靠前者胜出
    let anchor = pinned_day.or_else(|| days.iter().enumerate().rev().filter_map(|(i, d)| d.map(|d| (i, d)))
        .max_by_key(|&(_, a)| days.iter().flatten().filter(|&&b| near(a, b)).count()));
    let Some((_, anchor)) = anchor.filter(|_| days.iter().flatten().count() > 1) else { return (parts, vec![]) };
    let (mut kept, mut notes) = (vec![], vec![]);
    for ((id, p), d) in parts.into_iter().zip(days) {
        match d {
            Some(d) if !near(anchor, d) => notes.push(format!(
                "来源结果不一致:{id} 的发行日期 {} 与其他来源相差超过一年,可能是同番号的另一部影片,已不采用(可在详情页重选来源)",
                p.release_date.as_deref().unwrap_or_default())),
            _ => kept.push((id, p)),
        }
    }
    (kept, notes)
}

pub fn aggregate(code: &str, parts: &[(SourceId, PartialMetadata)], chains: &Chains) -> Metadata {
    let mut out = Metadata { code: code.to_string(), ..Default::default() };
    let get = |site: &str| parts.iter().find(|(id, _)| *id == site).map(|(_, p)| p);
    // 链上已抓到数据的来源,按链顺序
    let ordered = |field: &str| -> Vec<(&str, &PartialMetadata)> {
        chains.chain(field).iter().filter_map(|s| get(s).map(|p| (s.as_str(), p))).collect()
    };

    macro_rules! scalar {
        ($name:literal, $field:ident, $empty:expr) => {
            if let Some((site, p)) = ordered($name).into_iter().find(|(_, p)| !$empty(&p.$field)) {
                out.fields.$field = p.$field.clone();
                out.provenance.insert($name.to_string(), site.to_string());
            }
        };
    }
    let opt = |v: &Option<String>| v.as_deref().map_or(true, |s| s.trim().is_empty());
    let num = |v: &Option<u32>| v.is_none();
    let list = |v: &Vec<String>| v.is_empty();
    let actors_empty = |v: &Vec<Actor>| v.is_empty();

    scalar!("title", title, opt);
    scalar!("original_title", original_title, opt);
    scalar!("plot", plot, opt);
    scalar!("release_date", release_date, opt);
    scalar!("runtime_min", runtime_min, num);
    scalar!("studio", studio, opt);
    scalar!("publisher", publisher, opt);
    scalar!("series", series, opt);
    scalar!("actors", actors, actors_empty);
    scalar!("tags", tags, list);
    scalar!("directors", directors, list);

    // 名单锁定后,按姓名从各来源回填未知性别,再去重
    for a in out.fields.actors.iter_mut().filter(|a| a.gender == Gender::Unknown) {
        a.gender = parts
            .iter()
            .flat_map(|(_, p)| p.actors.iter())
            .find(|s| s.name == a.name && s.gender != Gender::Unknown)
            .map(|s| s.gender)
            .unwrap_or(Gender::Unknown);
    }
    // 头像同样按姓名、按路线顺序从任意来源回填(ADR-0009)
    for a in out.fields.actors.iter_mut().filter(|a| a.thumb.is_none()) {
        a.thumb = ordered("actors").into_iter().flat_map(|(_, p)| p.actors.iter()).find(|s| s.name == a.name && s.thumb.is_some()).and_then(|s| s.thumb.clone());
    }
    out.fields.actors = dedupe(std::mem::take(&mut out.fields.actors).into_iter().filter(|a| !a.name.is_empty()), |a| a.name.clone());
    out.fields.tags = dedupe(std::mem::take(&mut out.fields.tags).into_iter().filter(|t| !t.is_empty()), Clone::clone);
    out.fields.directors = dedupe(std::mem::take(&mut out.fields.directors).into_iter().filter(|t| !t.is_empty()), Clone::clone);

    // URL 类字段:沿链拼接去重
    macro_rules! urls {
        ($name:literal, $field:ident) => {
            out.fields.$field = dedupe(
                ordered($name).into_iter().flat_map(|(_, p)| p.$field.clone()).filter(|u| !u.is_empty()),
                Clone::clone,
            );
        };
    }
    urls!("poster_urls", poster_urls);
    urls!("thumb_urls", thumb_urls);
    urls!("trailer_urls", trailer_urls);

    for (site, p) in ordered("extrafanart") {
        if !p.extrafanart.is_empty() {
            out.extrafanart_by_source.insert(site.to_string(), p.extrafanart.clone());
            // 落盘用链上第一个有剧照的来源,不跨站混合
            if out.fields.extrafanart.is_empty() {
                out.fields.extrafanart = p.extrafanart.clone();
            }
        }
    }
    for (site, p) in ordered("score") {
        if let Some(score) = p.score {
            out.scores.push(SourcedScore { site: site.to_string(), score });
        }
    }
    for (site, p) in parts {
        if let Some(id) = p.external_id.as_deref().filter(|s| !s.is_empty()) {
            out.external_ids.entry(site.to_string()).or_insert_with(|| id.to_string());
        }
        if let Some(u) = p.source_url.as_deref().filter(|s| !s.is_empty()) {
            out.source_urls.entry(site.to_string()).or_insert_with(|| u.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn dated(id: SourceId, date: Option<&str>) -> (SourceId, PartialMetadata) {
        (id, PartialMetadata { title: Some(format!("{id} title")), release_date: date.map(String::from), ..Default::default() })
    }
    fn ids(parts: &[(SourceId, PartialMetadata)]) -> Vec<SourceId> { parts.iter().map(|(id, _)| *id).collect() }

    #[test]
    fn colliding_code_page_with_a_conflicting_release_date_is_dropped_with_a_note() {
        let search = include_str!("../tests/fixtures/javdb/search_XYZ-018.html");
        assert_eq!(crate::sources::javdb::parse_search(search, "XYZ-018").as_deref(), Some("/v/BBB01"), "javdb picks the 2022 title");
        let javdb = crate::sources::javdb::parse_detail(include_str!("../tests/fixtures/javdb/detail_XYZ-018.html")).unwrap();
        let jav321 = crate::sources::jav321::parse_detail(include_str!("../tests/fixtures/jav321/detail_XYZ-018_48xyz.html")).unwrap();
        assert_eq!(jav321.release_date.as_deref(), Some("2010-01-06"));
        let (kept, notes) = drop_date_conflicts(vec![("javdb", javdb), ("jav321", jav321)]);
        assert_eq!(ids(&kept), ["javdb"]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("jav321") && notes[0].contains("2010-01-06"), "{}", notes[0]);
        let chains = crate::settings::Settings::default().chains_for(crate::code::ContentType::Censored);
        let m = aggregate("XYZ-018", &kept, &chains);
        assert_ne!(m.fields.title.as_deref(), Some("Sample Title 2010"));
        assert_eq!(m.provenance.get("title").map(String::as_str), Some("javdb"));
    }

    #[test]
    fn date_check_keeps_agreeing_single_and_undated_results() {
        let (kept, notes) = drop_date_conflicts(vec![dated("a", Some("2022-01-01")), dated("b", Some("2022/06/30")), dated("c", None)]);
        assert_eq!((ids(&kept), notes.len()), (vec!["a", "b", "c"], 0));
        let (kept, notes) = drop_date_conflicts(vec![dated("a", Some("2010-01-06")), dated("b", None)]);
        assert_eq!((ids(&kept), notes.len()), (vec!["a", "b"], 0), "nothing to compare against");
        let (kept, _) = drop_date_conflicts(vec![dated("a", Some("2022-01-01")), dated("b", Some("unknown"))]);
        assert_eq!(ids(&kept), ["a", "b"], "unparseable dates are not evidence");
    }

    #[test]
    fn the_larger_agreeing_group_wins_and_ties_follow_route_order() {
        let (kept, notes) = drop_date_conflicts(vec![dated("a", Some("2010-01-06")), dated("b", Some("2022-01-01")), dated("c", Some("2022-02-01"))]);
        assert_eq!((ids(&kept), notes.len()), (vec!["b", "c"], 1), "majority beats route order");
        let (kept, _) = drop_date_conflicts(vec![dated("a", Some("2022-01-01")), dated("b", Some("2010-01-06"))]);
        assert_eq!(ids(&kept), ["a"], "tie keeps the earlier route site");
    }
    fn map(k: &str, v: &[&str]) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(k.to_string(), s(v))])
    }

    #[test]
    fn compile_prefer_intersects_route_and_exclude_wins() {
        let route = s(&["a", "b", "c"]);
        // prefer 中不在 route 的站无效;其余保序接上
        let c = compile_priority(&route, &map("title", &["c", "zzz"]), &BTreeMap::new());
        assert_eq!(c.chain("title"), ["c", "a", "b"]);
        // 未覆盖字段直接用 route
        assert_eq!(c.chain("plot"), ["a", "b", "c"]);
        // 同时出现在 prefer 与 exclude 时 exclude 胜
        let c = compile_priority(&route, &map("title", &["c"]), &map("title", &["c", "a"]));
        assert_eq!(c.chain("title"), ["b"]);
        // 只有 exclude 也生效
        let c = compile_priority(&route, &BTreeMap::new(), &map("plot", &["b"]));
        assert_eq!(c.chain("plot"), ["a", "c"]);
    }

    #[test]
    fn scalar_first_non_empty_and_provenance() {
        let a = PartialMetadata { title: Some("A".into()), ..Default::default() };
        let b = PartialMetadata { title: Some("  ".into()), plot: Some("P".into()), ..Default::default() };
        let chains = compile_priority(&s(&["b", "a"]), &BTreeMap::new(), &BTreeMap::new());
        let m = aggregate("X-001", &[("a", a), ("b", b)], &chains);
        assert_eq!(m.fields.title.as_deref(), Some("A")); // b 的空白标题被跳过
        assert_eq!(m.provenance["title"], "a");
        assert_eq!(m.provenance["plot"], "b");
    }

    #[test]
    fn urls_concatenate_dedupe_and_follow_chain_order() {
        let a = PartialMetadata { thumb_urls: s(&["u1", "u2"]), ..Default::default() };
        let b = PartialMetadata { thumb_urls: s(&["u2", "u3"]), ..Default::default() };
        let chains = compile_priority(&s(&["b", "a"]), &BTreeMap::new(), &BTreeMap::new());
        let m = aggregate("X-001", &[("a", a), ("b", b)], &chains);
        assert_eq!(m.fields.thumb_urls, ["u2", "u3", "u1"]);
    }

    #[test]
    fn actors_first_non_empty_then_gender_backfill_and_dedupe() {
        let a = PartialMetadata {
            actors: vec![Actor::new("X", Gender::Unknown), Actor::new("X", Gender::Female), Actor::new("Y", Gender::Unknown)],
            ..Default::default()
        };
        let b = PartialMetadata { actors: vec![Actor::new("Y", Gender::Male)], ..Default::default() };
        let chains = compile_priority(&s(&["a", "b"]), &BTreeMap::new(), &BTreeMap::new());
        let m = aggregate("X-001", &[("a", a), ("b", b)], &chains);
        // 名单取 a(第一个非空);同名 X 去重并取到已知性别,Y 的性别从 b 回填
        assert_eq!(m.fields.actors, vec![Actor::new("X", Gender::Female), Actor::new("Y", Gender::Male)]);
    }

    #[test]
    fn extrafanart_scores_and_ids_keep_source() {
        let a = PartialMetadata { extrafanart: s(&["e1"]), score: Some(80.0), external_id: Some("1".into()), ..Default::default() };
        let b = PartialMetadata { extrafanart: s(&["e2"]), score: Some(90.0), source_url: Some("http://b".into()), ..Default::default() };
        let chains = compile_priority(&s(&["a", "b"]), &BTreeMap::new(), &BTreeMap::new());
        let m = aggregate("X-001", &[("a", a), ("b", b)], &chains);
        assert_eq!(m.fields.extrafanart, ["e1"], "链上第一个有剧照的来源");
        assert_eq!(m.extrafanart_by_source["a"], ["e1"]);
        assert_eq!(m.extrafanart_by_source["b"], ["e2"]);
        assert_eq!(m.mean_score(), Some(85.0));
        assert_eq!(m.external_ids["a"], "1");
        assert_eq!(m.source_urls["b"], "http://b");
    }
}
