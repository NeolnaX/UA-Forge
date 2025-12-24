use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn parse(s: &str) -> Level {
        match s.to_ascii_lowercase().as_str() {
            "debug" => Level::Debug,
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => Level::Info,
        }
    }
}

struct Logger {
    level: Level,
    out: Mutex<Box<dyn Write + Send>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(level: Level, path: Option<&str>) -> io::Result<()> {
    let writer: Box<dyn Write + Send> = if let Some(p) = path {
        let f = OpenOptions::new().create(true).append(true).open(p)?;
        Box::new(f)
    } else {
        Box::new(io::stderr())
    };
    let _ = LOGGER.set(Logger {
        level,
        out: Mutex::new(writer),
    });
    Ok(())
}

pub fn log(level: Level, args: std::fmt::Arguments) {
    let Some(logger) = LOGGER.get() else {
        let _ = writeln!(io::stderr(), "{}", args);
        return;
    };
    if level < logger.level {
        return;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let level_str = match level {
        Level::Debug => "DEBUG",
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERROR",
    };
    let mut out = logger.out.lock();
    let _ = writeln!(out, "[{ts}] [{level_str}] {}", args);
    // 移除 flush() 以减少 I/O 阻塞，依赖操作系统缓冲
}

