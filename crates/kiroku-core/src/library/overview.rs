use super::{query, Library, Query, Result};
use rusqlite::params_from_iter;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub movies: usize,
    pub files: usize,
    pub bytes: u64,
    pub favorites: usize,
    pub average_score: Option<f64>,
    pub missing_poster: usize,
    pub missing_year: usize,
    pub missing_score: usize,
}
impl Library {
    /// Aggregation of the complete current query, independent of pagination.
    /// File sizes are logical indexed bytes; links may share physical storage.
    pub fn overview(&self, q: &Query) -> Result<Overview> {
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let (conditions, args) = query::filters(q);
        let clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        let sql = format!("WITH matched AS (SELECT * FROM movie {clause}) SELECT
          COUNT(*), AVG(score), COALESCE(SUM(poster IS NULL OR poster=''),0),
          COALESCE(SUM(year IS NULL),0), COALESCE(SUM(score IS NULL),0),
          (SELECT COUNT(*) FROM file WHERE code IN (SELECT code FROM matched)),
          (SELECT COALESCE(SUM(size),0) FROM file WHERE code IN (SELECT code FROM matched)),
          (SELECT COUNT(DISTINCT code) FROM movie_facet WHERE kind='favorite' AND code IN (SELECT code FROM matched))
          FROM matched");
        let result = self
            .conn
            .query_row(&sql, params_from_iter(args.iter()), |r| {
                Ok(Overview {
                    movies: r.get::<_, i64>(0)? as usize,
                    average_score: r.get(1)?,
                    missing_poster: r.get::<_, i64>(2)? as usize,
                    missing_year: r.get::<_, i64>(3)? as usize,
                    missing_score: r.get::<_, i64>(4)? as usize,
                    files: r.get::<_, i64>(5)? as usize,
                    bytes: r.get::<_, i64>(6)?.max(0) as u64,
                    favorites: r.get::<_, i64>(7)? as usize,
                })
            })?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(result)
    }
}
