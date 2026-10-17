use crate::{code::Code, metadata::PartialMetadata, net::Http};
use async_trait::async_trait;

pub type SourceId = &'static str;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("not found")]
    NotFound,
    #[error("blocked by anti-bot ({0})")]
    Blocked(String),
    #[error("network: {0}")]
    Network(String),
    #[error("parse: {0}")]
    Parse(String),
    /// 站点要求登录(被重定向到登录页);提示用户在设置里填写 Cookie
    #[error("{0}")]
    NeedLogin(String),
}

/// 每个站点实现一次；无状态，番号 → 部分元数据。
#[async_trait]
pub trait Source: Send + Sync {
    async fn check_connectivity(&self, _http: &Http) -> crate::connectivity::ConnectivityOutcome { crate::connectivity::ConnectivityOutcome::skipped("undeclared") }
    fn id(&self) -> SourceId;
    fn supports(&self, code: &Code) -> bool;
    async fn fetch(&self, code: &Code, http: &Http) -> Result<PartialMetadata, SourceError>;
    /// 用户指定的本站详情页 → (字段, 页面文本);None 表示本站不支持按地址刮削。
    /// 地址归属已由调用方按设置的 base 校验。页面文本只用于核对番号,不保存。
    async fn fetch_detail(&self, _url: &str, _code: &Code, _http: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> { None }
}
