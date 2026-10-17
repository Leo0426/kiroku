//! 运行日志。核心各处用 `log` 宏打点,target 统一写成 `kiroku::<模块>`(如 `kiroku::organize`);
//! 这里的 Logger 按设置把日志送到三处,互不依赖:
//! - 终端(stderr):开发调试,`pnpm tauri dev` / CLI 里可见;
//! - App:交给客户端注册的回调(桌面端转成 `log:line` 事件,在整理页实时显示);
//! - 文件:`<设置目录>/logs/kiroku-YYYY-MM-DD.log`,按天追加,事后排查用。
//!
//! 只输出 kiroku 自己的日志(target 以 `kiroku` 开头),依赖库(reqwest、tauri…)的日志不混进来。
//! 设置可随时改([`configure`]),无需重启。

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock, RwLock,
    },
};

/// 日志设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogCfg {
    /// 输出到终端
    pub console: bool,
    /// 在 App 内实时显示
    pub app: bool,
    /// 写入日志文件
    pub file: bool,
    /// 详细:额外记录每个网络请求等调试信息
    pub verbose: bool,
}

impl Default for LogCfg {
    fn default() -> Self {
        Self { console: true, app: true, file: false, verbose: false }
    }
}

/// 一行日志(App 回调与事件载荷)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// 本地时间 `HH:MM:SS.mmm`
    pub time: String,
    /// `error` / `warn` / `info` / `debug`
    pub level: &'static str,
    /// 模块,如 `organize` `net` `scan`
    pub target: String,
    pub message: String,
}

impl LogLine {
    /// 文本形式:`20:15:13.275 INFO  organize  消息`。
    pub fn render(&self) -> String {
        format!("{} {:<5} {:<8} {}", self.time, self.level.to_uppercase(), self.target, self.message)
    }
}

pub type AppSink = Box<dyn Fn(&LogLine) + Send + Sync>;

struct Logger {
    cfg: RwLock<LogCfg>,
    dir: PathBuf,
    /// (日期, 文件);跨天时换新文件
    file: Mutex<Option<(String, File)>>,
    sink: RwLock<Option<AppSink>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// `<设置目录>/logs`
pub fn log_dir(settings_path: &Path) -> PathBuf {
    settings_path.parent().unwrap_or(Path::new(".")).join("logs")
}

/// 安装全局 Logger(进程内只生效一次;再次调用等同 [`configure`])。
pub fn init(dir: PathBuf, cfg: &LogCfg) {
    let mut fresh = false;
    let logger = LOGGER.get_or_init(|| {
        fresh = true;
        Logger { cfg: RwLock::new(cfg.clone()), dir, file: Mutex::new(None), sink: RwLock::new(None) }
    });
    if fresh {
        // 已有别的 logger(如测试框架)时忽略
        if log::set_logger(logger).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
    } else {
        configure(cfg);
    }
}

/// 更新设置(保存设置后调用)。
pub fn configure(cfg: &LogCfg) {
    if let Some(l) = LOGGER.get() {
        *l.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg.clone();
    }
}

/// 注册 App 内显示的回调(桌面端在启动时注册)。
pub fn set_app_sink(sink: AppSink) {
    if let Some(l) = LOGGER.get() {
        *l.sink.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }
}

fn level_name(l: log::Level) -> &'static str {
    match l {
        log::Level::Error => "error",
        log::Level::Warn => "warn",
        log::Level::Info => "info",
        log::Level::Debug | log::Level::Trace => "debug",
    }
}

/// `kiroku::organize` → `organize`;`kiroku_core::net` → `net`。
fn short_target(t: &str) -> String {
    t.rsplit("::").next().unwrap_or(t).to_string()
}

fn wanted(target: &str, level: log::Level, verbose: bool) -> bool {
    target.starts_with("kiroku") && (level <= log::Level::Info || verbose && level == log::Level::Debug)
}

impl Logger {
    fn write_file(&self, line: &LogLine) {
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut g = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().is_none_or(|(d, _)| *d != date) {
            let path = self.dir.join(format!("kiroku-{date}.log"));
            let opened = fs::create_dir_all(&self.dir).and_then(|_| OpenOptions::new().create(true).append(true).open(&path));
            match opened {
                Ok(f) => *g = Some((date, f)),
                Err(e) => {
                    // 只提示一次:写不了日志不能影响整理本身
                    if !FILE_ERROR_SHOWN.swap(true, Ordering::Relaxed) {
                        eprintln!("无法写入日志文件 {}: {e}", path.display());
                    }
                    *g = None;
                    return;
                }
            }
        }
        if let Some((_, f)) = g.as_mut() {
            let _ = writeln!(f, "{}", line.render());
        }
    }
}

static FILE_ERROR_SHOWN: AtomicBool = AtomicBool::new(false);

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        let verbose = self.cfg.read().map(|c| c.verbose).unwrap_or(false);
        wanted(m.target(), m.level(), verbose)
    }

    fn log(&self, r: &log::Record) {
        let cfg = self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone();
        if !wanted(r.target(), r.level(), cfg.verbose) || !(cfg.console || cfg.app || cfg.file) {
            return;
        }
        let line = LogLine {
            time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            level: level_name(r.level()),
            target: short_target(r.target()),
            message: r.args().to_string(),
        };
        if cfg.console {
            eprintln!("{}", line.render());
        }
        if cfg.file {
            self.write_file(&line);
        }
        if cfg.app {
            if let Some(sink) = self.sink.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
                sink(&line);
            }
        }
    }

    fn flush(&self) {
        if let Some((_, f)) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn filters_by_target_and_level() {
        assert!(wanted("kiroku::organize", log::Level::Info, false));
        assert!(wanted("kiroku::net", log::Level::Warn, false));
        assert!(!wanted("kiroku::net", log::Level::Debug, false));
        assert!(wanted("kiroku::net", log::Level::Debug, true));
        assert!(!wanted("reqwest::connect", log::Level::Info, true), "依赖库的日志不输出");
        assert_eq!(short_target("kiroku::organize"), "organize");
    }

    /// 全局 Logger 只能装一次,三种输出放在同一个测试里验证。
    /// 其他并行的测试也会打 kiroku 日志,这里只看自己 target(`logtest`)的行。
    #[test]
    fn routes_lines_to_app_and_file_per_settings() {
        let d = tempfile::tempdir().unwrap();
        init(d.path().join("logs"), &LogCfg { console: false, app: true, file: true, verbose: false });
        let got = Arc::new(StdMutex::new(Vec::<LogLine>::new()));
        let g2 = got.clone();
        set_app_sink(Box::new(move |l| if l.target == "logtest" { g2.lock().unwrap().push(l.clone()) }));

        log::info!(target: "kiroku::logtest", "移动 a → b");
        log::debug!(target: "kiroku::logtest", "GET x");
        log::info!(target: "other::logtest", "不该出现");
        {
            let lines = got.lock().unwrap();
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert_eq!((lines[0].level, lines[0].message.as_str()), ("info", "移动 a → b"));
        }
        let file = fs::read_dir(d.path().join("logs")).unwrap().next().unwrap().unwrap().path();
        assert!(file.file_name().unwrap().to_string_lossy().starts_with("kiroku-"));
        let text = || fs::read_to_string(&file).unwrap();
        assert!(text().contains("INFO  logtest  移动 a → b"));
        assert!(!text().contains("GET x") && !text().contains("不该出现"));

        // 改设置立即生效:关掉 App、打开详细
        configure(&LogCfg { console: false, app: false, file: true, verbose: true });
        log::debug!(target: "kiroku::logtest", "GET y");
        assert_eq!(got.lock().unwrap().len(), 1);
        assert!(text().contains("DEBUG logtest  GET y"));
    }
}
