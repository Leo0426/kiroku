//! Query accelerators remain rebuildable caches of the movie index.
pub(super) const MIGRATION: &str = r#"
CREATE TABLE library_state(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL);
INSERT INTO library_state VALUES(1,0);
CREATE INDEX movie_added_code ON movie(-added_at,code);
CREATE INDEX movie_title_code ON movie(COALESCE(title,code) COLLATE NOCASE,code);
CREATE INDEX movie_year_code ON movie((-year IS NULL),-year,code);
CREATE INDEX movie_score_code ON movie((-score IS NULL),-score,code);
CREATE VIRTUAL TABLE movie_search USING fts5(search,content='movie',content_rowid='rowid',tokenize='trigram');
CREATE TRIGGER movie_search_insert AFTER INSERT ON movie BEGIN
 INSERT INTO movie_search(rowid,search) VALUES(new.rowid,new.search);
 UPDATE library_state SET revision=revision+1 WHERE id=1;
END;
CREATE TRIGGER movie_search_delete AFTER DELETE ON movie BEGIN
 INSERT INTO movie_search(movie_search,rowid,search) VALUES('delete',old.rowid,old.search);
 UPDATE library_state SET revision=revision+1 WHERE id=1;
END;
CREATE TRIGGER movie_search_update AFTER UPDATE OF search ON movie WHEN old.search IS NOT new.search BEGIN
 INSERT INTO movie_search(movie_search,rowid,search) VALUES('delete',old.rowid,old.search);
 INSERT INTO movie_search(rowid,search) VALUES(new.rowid,new.search);
END;
CREATE TRIGGER movie_revision_update AFTER UPDATE ON movie BEGIN
 UPDATE library_state SET revision=revision+1 WHERE id=1;
END;
INSERT INTO movie_search(movie_search) VALUES('rebuild');
"#;

use super::Sort;
// Reverse only the value/code keys: unknown values still sort last.
pub(super) const REVERSE_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS movie_year_reverse_code ON movie((-year IS NULL),-year DESC,code DESC);
CREATE INDEX IF NOT EXISTS movie_score_reverse_code ON movie((-score IS NULL),-score DESC,code DESC);
";
use rusqlite::types::Value;
use std::{collections::VecDeque, sync::Mutex};

const COUNT_LIMIT: usize = 32;
const ANCHOR_LIMIT: usize = 64;
const KEY_LIMIT: usize = 4096;

#[derive(Clone)]
pub(super) struct Anchor {
    pub offset: usize,
    pub value: Value,
    pub code: String,
}
#[derive(Default)]
struct Cached {
    revision: Option<i64>,
    counts: VecDeque<(String, usize)>,
    anchors: VecDeque<(String, Anchor)>,
    #[cfg(test)]
    hits: usize,
    #[cfg(test)]
    seeks: usize,
}
#[derive(Default)]
pub(super) struct ReadCache(Mutex<Cached>);
impl ReadCache {
    fn align(cache: &mut Cached, revision: i64) -> bool {
        if cache.revision == Some(revision) {
            return true;
        }
        // An old WAL reader may finish after a newer committed reader.
        if cache.revision.is_some_and(|current| current > revision) {
            return false;
        }
        cache.revision = Some(revision);
        cache.counts.clear();
        cache.anchors.clear();
        true
    }
    pub fn count(&self, revision: i64, key: &str) -> Option<usize> {
        if key.len() > KEY_LIMIT {
            return None;
        }
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !Self::align(&mut cache, revision) {
            return None;
        }
        let index = cache.counts.iter().position(|(saved, _)| saved == key)?;
        let entry = cache.counts.remove(index).unwrap();
        let count = entry.1;
        cache.counts.push_back(entry);
        #[cfg(test)]
        {
            cache.hits += 1;
        }
        Some(count)
    }
    pub fn remember_count(&self, revision: i64, key: String, count: usize) {
        if key.len() > KEY_LIMIT {
            return;
        }
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !Self::align(&mut cache, revision) {
            return;
        }
        cache.counts.retain(|(saved, _)| saved != &key);
        cache.counts.push_back((key, count));
        if cache.counts.len() > COUNT_LIMIT {
            cache.counts.pop_front();
        }
    }
    pub fn anchor(&self, revision: i64, key: &str, offset: usize) -> Option<Anchor> {
        if key.len() > KEY_LIMIT {
            return None;
        }
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !Self::align(&mut cache, revision) {
            return None;
        }
        let anchor = cache
            .anchors
            .iter()
            .filter(|(saved, anchor)| saved == key && anchor.offset <= offset && offset - anchor.offset <= 500)
            .max_by_key(|(_, anchor)| anchor.offset)
            .map(|(_, anchor)| anchor.clone());
        #[cfg(test)]
        {
            if anchor.is_some() {
                cache.seeks += 1;
            }
        }
        anchor
    }
    pub fn remember_anchor(&self, revision: i64, key: String, anchor: Anchor) {
        if key.len() > KEY_LIMIT
            || anchor.code.len() > 512
            || matches!(&anchor.value, Value::Text(text) if text.len()>KEY_LIMIT)
        {
            return;
        }
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !Self::align(&mut cache, revision) {
            return;
        }
        cache
            .anchors
            .retain(|(saved, entry)| saved != &key || entry.offset != anchor.offset);
        cache.anchors.push_back((key, anchor));
        if cache.anchors.len() > ANCHOR_LIMIT {
            cache.anchors.pop_front();
        }
    }
    #[cfg(test)]
    pub fn stats(&self) -> (usize, usize, usize, usize) {
        let cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        (
            cache.counts.len(),
            cache.anchors.len(),
            cache.hits,
            cache.seeks,
        )
    }
}

pub(super) fn key(sort: Sort) -> (&'static str, bool) {
    match sort {
        Sort::Added => ("-added_at", false),
        Sort::Title => ("COALESCE(title, code) COLLATE NOCASE", false),
        Sort::Year => ("-year", true),
        Sort::Score => ("-score", true),
    }
}

/// Split the mixed-key seek into indexed equal-key and greater-key ranges.
/// A row-value comparison on an expression index can silently become a full scan.
pub(super) fn page_sql(
    filters: &[String],
    filter_args: Vec<Value>,
    sort: Sort,
    reverse: bool,
    offset: usize,
    limit: usize,
    anchor: Option<Anchor>,
) -> (String, Vec<Value>) {
    let (key, nullable) = key(sort);
    let compare = if reverse { "<" } else { ">" };
    let select=format!("SELECT code,title,year,actors,poster,content_type,score,added_at,(SELECT COUNT(*) FROM file WHERE file.code=movie.code),poster_version,{key} AS sort_key,({key} IS NULL) AS sort_null FROM movie");
    let mut sql_parts = Vec::new();
    let mut args = Vec::new();
    let mut remaining = offset;
    let mut branch = |condition: Option<String>, extra: Vec<Value>| {
        let mut clauses = filters.to_vec();
        if let Some(condition) = condition {
            clauses.push(condition);
        }
        let clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        sql_parts.push(format!("{select}{clause}"));
        args.extend(filter_args.iter().cloned());
        args.extend(extra);
    };
    if let Some(anchor) = anchor {
        remaining -= anchor.offset;
        if anchor.value == Value::Null {
            branch(
                Some(format!("({key} IS NULL)=1 AND {key} IS NULL AND code{compare}?")),
                vec![Value::Text(anchor.code)],
            );
        } else {
            let prefix = if nullable {
                format!("({key} IS NULL)=0 AND ")
            } else {
                String::new()
            };
            branch(
                Some(format!("{prefix}{key}=? AND code{compare}?")),
                vec![anchor.value.clone(), Value::Text(anchor.code)],
            );
            branch(Some(format!("{prefix}{key}{compare}?")), vec![anchor.value]);
            if nullable {
                branch(Some(format!("({key} IS NULL)=1 AND {key} IS NULL")), vec![]);
            }
        }
    } else {
        branch(None, vec![]);
    }
    let direction = if reverse { "DESC" } else { "ASC" };
    let order = format!("{}sort_key {direction},code {direction}", if nullable { "sort_null," } else { "" });
    args.push(Value::Integer(limit as i64));
    args.push(Value::Integer(i64::try_from(remaining).unwrap_or(i64::MAX)));
    (
        format!(
            "{} ORDER BY {order} LIMIT ? OFFSET ?",
            sql_parts.join(" UNION ALL ")
        ),
        args,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_distant_anchor_does_not_turn_a_cold_jump_into_an_expensive_union_scan() {
        let cache=ReadCache::default();
        cache.remember_anchor(1,"query".into(),Anchor{offset:120,value:Value::Integer(-100),code:"AAA-111".into()});
        assert!(cache.anchor(1,"query",120).is_some());
        assert!(cache.anchor(1,"query",620).is_some());
        assert!(cache.anchor(1,"query",621).is_none());
        assert!(cache.anchor(1,"query",100000).is_none());
    }
}

/// Shared authority for list, CSV and overview membership.
pub(super) fn filters(q: &super::Query) -> (Vec<String>, Vec<Value>) {
    use super::{like_pattern, MissingField};
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
        if let Some(score) = q.min_score {
            wh.push("score >= ?".into());
            args.push(Value::Integer(score as i64));
        }
        if let Some(missing) = q.missing {
            wh.push(match missing {
                MissingField::Poster => "(poster IS NULL OR poster = '')",
                MissingField::Score => "score IS NULL",
                MissingField::Year => "year IS NULL",
            }.into());
        }
        for filter in &q.filters {
            wh.push("code IN (SELECT code FROM movie_facet WHERE kind = ? AND value = ?)".into());
            args.push(Value::Text(filter.kind.as_str().into()));
            args.push(Value::Text(filter.value.clone()));
        }
    (wh,args)
}
