//! kiroku 领域核心：番号解析 → 多源聚合 → 整理落盘。
//! 不依赖任何 UI / 服务化外壳，CLI 与 Tauri 共用。

pub mod aggregate;
pub mod assets;
pub mod badge;
pub mod code;
pub mod journal;
pub mod library;
pub mod logging;
pub mod media_server;
pub mod nfo_read;
pub mod metadata;
pub mod net;
pub mod nfo;
pub mod organize;
pub mod pipeline;
pub mod plex;
pub mod settings;
pub mod source;
pub mod subtitle;
pub mod template;
pub mod translate;
pub mod video_filter;
pub mod sources;

pub use code::{parse_code, Code, CodeKind};
pub use metadata::{Metadata, PartialMetadata};

pub mod revision;

pub mod refresh;

pub mod artwork;

pub mod append;

pub mod connectivity;

pub mod health;

pub mod maintenance;

pub mod names;

pub mod catalog;
pub mod placement;
pub mod localization;

pub mod recovery;
