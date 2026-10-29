//! A read-only, bounded-memory CSV export of one committed index snapshot.
use super::{Library, Query, Result};
use std::io::{self, Write};
use tokio_util::sync::CancellationToken;

fn cell(value: &str) -> String {
    // Guard spreadsheet formula prefixes, including leading whitespace/control
    // characters. Quoting alone does not prevent spreadsheet evaluation.
    let trimmed = value.trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}');
    let guard = trimmed.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
        || value.starts_with(['\t', '\r', '\n']);
    format!("\"{}{}\"", if guard { "'" } else { "" }, value.replace('"', "\"\""))
}

impl Library {
    pub fn export_csv(&self, query: &Query, out: &mut impl Write, cancel: &CancellationToken, progress: &mut impl FnMut(usize, usize)) -> Result<usize> {
        self.verify_binding()?;
        let transaction = if self.conn.is_autocommit() { Some(self.conn.unchecked_transaction()?) } else { None };
        out.write_all("\u{feff}编号,标题,年份,演员,评分,分片数,收藏\r\n".as_bytes())?;
        let mut query = query.clone();
        query.offset = 0;
        query.limit = 500;
        loop {
            if cancel.is_cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into()); }
            let page = self.list(&query)?;
            progress(query.offset, page.total);
            for card in &page.items {
                if cancel.is_cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into()); }
                let values = [card.code.clone(), card.title.clone().unwrap_or_default(), card.year.map(|v| v.to_string()).unwrap_or_default(), card.actors.join(" / "), card.score.map(|s| format!("{:.1}", s / 10.0)).unwrap_or_default(), card.parts.to_string(), if card.favorite { "是" } else { "否" }.into()];
                writeln!(out, "{}\r", values.iter().map(|v| cell(v)).collect::<Vec<_>>().join(","))?;
            }
            query.offset += page.items.len();
            progress(query.offset, page.total);
            if query.offset >= page.total || page.items.is_empty() { break; }
        }
        self.verify_binding()?;
        if cancel.is_cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "导出已停止").into()); }
        if let Some(transaction) = transaction { transaction.commit()?; }
        Ok(query.offset)
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
