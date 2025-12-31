use clap::{Parser, Args};
use std::time::Duration;

// 默认值常量
const DEFAULT_DECISION_DELAY_SECS: u64 = 60;
const DEFAULT_HTTP_COOLDOWN_SECS: u64 = 3600;
const DEFAULT_REGEX_PATTERN: &str = "(iPhone|iPad|Android|Macintosh|Windows|Linux|Apple|Mac OS X|Mobile)";

#[derive(Clone, Debug, Args)]
pub struct FirewallConfig {
    #[arg(long, help = "Firewall type (ipset/nft)")]
    pub fw_type: Option<String>,

    #[arg(long, help = "Firewall set name")]
    pub fw_set_name: Option<String>,

    #[arg(long, help = "Drop connections on firewall whitelist hit")]
    pub fw_drop: bool,

    #[arg(long, value_delimiter = ',', help = "Firewall UA whitelist (comma-separated)")]
    pub fw_ua_w: Vec<String>,

    #[arg(long, help = "Enable firewall bypass for non-HTTP traffic")]
    pub fw_bypass: bool,

    #[arg(long, default_value = "5", help = "Non-HTTP threshold for firewall")]
    pub fw_nonhttp_threshold: u32,

    #[arg(long, default_value = "28800", help = "Firewall timeout in seconds")]
    pub fw_timeout: u32,

    #[arg(long, value_parser = parse_duration, help = "Firewall decision delay (e.g., 60s, 1m)")]
    pub fw_decision_delay: Option<Duration>,

    #[arg(long, value_parser = parse_duration, help = "Firewall HTTP cooldown (e.g., 1h, 60m)")]
    pub fw_http_cooldown: Option<Duration>,
}

impl FirewallConfig {
    pub fn enable_firewall_set(&self) -> bool {
        self.fw_type.as_ref().is_some_and(|s| !s.is_empty())
            && self.fw_set_name.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn get_decision_delay(&self) -> Duration {
        self.fw_decision_delay.unwrap_or_else(|| Duration::from_secs(DEFAULT_DECISION_DELAY_SECS))
    }

    pub fn get_http_cooldown(&self) -> Duration {
        self.fw_http_cooldown.unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_COOLDOWN_SECS))
    }
}

#[derive(Clone, Debug)]
pub enum MatchMode {
    Keywords(Vec<String>),
    Force,
    Regex { pattern: String },
}

#[derive(Parser, Clone, Debug)]
#[command(name = "uaforge", version = "0.1.1", about = "User-Agent modification proxy")]
pub struct CliArgs {
    #[arg(short = 'u', long, default_value = "FFF", help = "User-Agent string to use")]
    pub user_agent: String,

    #[arg(long, default_value = "8080", help = "Port to listen on")]
    pub port: u16,

    #[arg(long = "log-level", default_value = "info", help = "Log level (debug/info/warn/error)")]
    pub loglevel: String,

    #[arg(short = 'v', long, help = "Show version")]
    pub version: bool,

    #[arg(long, help = "Log file path")]
    pub log: Option<String>,

    #[arg(short = 'w', long, value_delimiter = ',', help = "Whitelist User-Agents (comma-separated)")]
    pub whitelist: Vec<String>,

    #[arg(long, default_value = "iPhone,iPad,Android,Macintosh,Windows", help = "Keywords to match (comma-separated)")]
    pub keywords: String,

    #[arg(short = 'r', long, help = "Regex pattern for matching")]
    pub regex_pattern: Option<String>,

    #[arg(long, default_value = "1000", help = "Cache size")]
    pub cache_size: usize,

    #[arg(long, help = "Force replace all User-Agents")]
    pub force: bool,

    #[arg(long, help = "Enable regex mode")]
    pub enable_regex: bool,

    #[command(flatten)]
    pub firewall: FirewallConfig,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub user_agent: String,
    pub port: u16,
    pub log_level: String,
    pub show_version: bool,
    pub log_file: Option<String>,
    pub whitelist: Vec<String>,
    pub cache_size: usize,
    pub match_mode: MatchMode,
    pub firewall: FirewallConfig,
}

impl Config {
    pub fn from_args<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString> + Clone,
    {
        // OpenWrt init script historically passes "long flags" with a single dash
        // (e.g. `-port 12032`, `-loglevel info`, `-fw-type nft`).
        // `clap` expects `--port` style for long flags, so normalize here for compatibility.
        let normalized_args: Vec<std::ffi::OsString> = args
            .into_iter()
            .map(|s| s.into())
            .enumerate()
            .map(|(idx, os)| {
                if idx == 0 {
                    return os;
                }
                let s = os.to_string_lossy();
                if s.starts_with('-') && !s.starts_with("--") && s.len() > 2 {
                    return format!("--{}", &s[1..]).into();
                }
                os
            })
            .collect();

        if std::env::var_os("UAFORGE_DEBUG_ARGS")
            .and_then(|v| v.to_string_lossy().parse::<u8>().ok())
            == Some(1)
        {
            eprintln!("[uaforge] normalized args:");
            for a in &normalized_args {
                eprintln!("  {:?}", a);
            }
        }

        let cli = CliArgs::parse_from(normalized_args);

        // Determine match mode
        let match_mode = if cli.force {
            MatchMode::Force
        } else if cli.enable_regex {
            let pattern = cli.regex_pattern.unwrap_or_else(|| DEFAULT_REGEX_PATTERN.to_string());
            MatchMode::Regex { pattern }
        } else {
            let keywords: Vec<String> = cli
                .keywords
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            MatchMode::Keywords(keywords)
        };

        Ok(Self {
            user_agent: cli.user_agent,
            port: cli.port,
            log_level: cli.loglevel,
            show_version: cli.version,
            log_file: cli.log,
            whitelist: cli.whitelist,
            cache_size: cli.cache_size,
            match_mode,
            firewall: cli.firewall,
        })
    }
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".to_string());
    }

    // 处理纯数字（默认为秒）
    if let Ok(n) = s.parse::<u64>() {
        return Ok(Duration::from_secs(n));
    }

    // 处理带单位的格式
    if s.len() < 2 {
        return Err("duration too short (expected format: 60s, 1m, 1h)".to_string());
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let n = num_str
        .parse::<u64>()
        .map_err(|_| format!("invalid duration number: {}", num_str))?;
    match unit {
        "s" => Ok(Duration::from_secs(n)),
        "m" => Ok(Duration::from_secs(n * 60)),
        "h" => Ok(Duration::from_secs(n * 3600)),
        _ => Err(format!("invalid duration unit: {} (expected s/m/h)", unit)),
    }
}
