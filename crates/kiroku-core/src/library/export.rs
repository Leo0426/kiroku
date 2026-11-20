//! A read-only, bounded-memory CSV export of one committed index snapshot.
use super::{Library, Query, Result};
use std::io::{self, Write};
use tokio_util::sync::CancellationToken;

fn cell(value: &str) -> String {
    // Guard spreadsheet formula prefixes, including leading whitespace/control
    // characters. Quoting alone does not prevent spreadsheet evaluation.
    let trimmed =
        value.trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}');
    let guard = trimmed.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
        || value.starts_with(['\t', '\r', '\n']);
    format!(
        "\"{}{}\"",
        if guard { "'" } else { "" },
        value.replace('"', "\"\"")
    )
}

#[derive(Clone, Copy, serde::Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CsvColumn {
    Code,
    Title,
    Year,
    Actors,
    Score,
    Parts,
    Favorite,
    Tags,
    Paths,
}
impl CsvColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Code => "编号",
            Self::Title => "标题",
            Self::Year => "年份",
            Self::Actors => "演员",
            Self::Score => "评分",
            Self::Parts => "分片数",
            Self::Favorite => "收藏",
            Self::Tags => "标签",
            Self::Paths => "文件路径",
        }
    }
}
#[derive(serde::Deserialize)]
pub struct CsvOptions {
    pub columns: Vec<CsvColumn>,
    pub codes: Option<Vec<String>>,
}
impl Default for CsvOptions {
    fn default() -> Self {
        use CsvColumn::*;
        Self {
            columns: vec![Code, Title, Year, Actors, Score, Parts, Favorite],
            codes: None,
        }
    }
}
impl Library {
    pub fn export_csv(
        &self,
        query: &Query,
        out: &mut impl Write,
        cancel: &CancellationToken,
        progress: &mut impl FnMut(usize, usize),
    ) -> Result<usize> {
        self.export_csv_with(query, &CsvOptions::default(), out, cancel, progress)
    }
    pub fn export_csv_with(
        &self,
        query: &Query,
        options: &CsvOptions,
        out: &mut impl Write,
        cancel: &CancellationToken,
        progress: &mut impl FnMut(usize, usize),
    ) -> Result<usize> {
        use std::collections::HashSet;
        let columns = &options.columns;
        if columns.is_empty() || columns.iter().collect::<HashSet<_>>().len() != columns.len() {
            return Err(io::Error::other("请选择不重复的 CSV 列").into());
        }
        let selected = options
            .codes
            .as_ref()
            .map(|codes| codes.iter().collect::<HashSet<_>>());
        self.verify_binding()?;
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        write!(
            out,
            "\u{feff}{}\r\n",
            columns
                .iter()
                .map(|c| c.label())
                .collect::<Vec<_>>()
                .join(",")
        )?;
        let mut query = query.clone();
        query.offset = 0;
        query.limit = 500;
        let mut written = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into());
            }
            let page = self.list(&query)?;
            progress(query.offset, page.total);
            for card in &page.items {
                if cancel.is_cancelled() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into());
                }
                if selected
                    .as_ref()
                    .is_some_and(|codes| !codes.contains(&card.code))
                {
                    continue;
                }
                let tags = if columns.contains(&CsvColumn::Tags) {
                    let mut stmt = self.conn.prepare_cached(
                        "SELECT value FROM movie_facet WHERE code=?1 AND kind='tag' ORDER BY value",
                    )?;
                    let rows = stmt
                        .query_map([&card.code], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows.join(" / ")
                } else {
                    String::new()
                };
                let paths = if columns.contains(&CsvColumn::Paths) {
                    let mut stmt=self.conn.prepare_cached("SELECT rel_path FROM file WHERE code=?1 ORDER BY COALESCE(part,0),rel_path")?;
                    let rows = stmt
                        .query_map([&card.code], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows.iter()
                        .map(|p| self.abs(p).display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                };
                let values = columns
                    .iter()
                    .map(|column| {
                        use CsvColumn::*;
                        cell(&match column {
                            Code => card.code.clone(),
                            Title => card.title.clone().unwrap_or_default(),
                            Year => card.year.map(|y| y.to_string()).unwrap_or_default(),
                            Actors => card.actors.join(" / "),
                            Score => card
                                .score
                                .map(|s| format!("{:.1}", s / 10.))
                                .unwrap_or_default(),
                            Parts => card.parts.to_string(),
                            Favorite => if card.favorite { "是" } else { "否" }.into(),
                            Tags => tags.clone(),
                            Paths => paths.clone(),
                        })
                    })
                    .collect::<Vec<_>>();
                write!(out, "{}\r\n", values.join(","))?;
                written += 1;
            }
            query.offset += page.items.len();
            progress(query.offset, page.total);
            if query.offset >= page.total || page.items.is_empty() {
                break;
            }
        }
        self.verify_binding()?;
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into());
        }
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        if selected.is_some_and(|s| written != s.len()) {
            return Err(io::Error::other("选中的影片已变化，请重新选择").into());
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_and_neutralizes_formula_prefixes() {
        assert_eq!(cell("a,\"b\"\nc"), "\"a,\"\"b\"\"\nc\"");
        for value in ["=1+1", "  +1", "\t@x", "\n-1", "＝1", "\u{feff}=1"] {
            assert!(cell(value).starts_with("\"'"));
        }
        assert_eq!(cell("普通标题"), "\"普通标题\"");
    }
}
