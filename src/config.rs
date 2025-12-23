use clap::Parser;
use std::time::Duration;

const DEFAULT_POOL_SIZE: usize = 64;
const MAX_POOL_SIZE: usize = 256;

#[derive(Clone, Debug)]
pub struct FirewallConfig {
    pub enable_firewall_set: bool,
    pub fw_type: Option<String>,
    pub fw_set_name: Option<String>,
    pub fw_drop: bool,
    pub fw_ua_whitelist: Vec<String>,
    pub fw_bypass: bool,
    pub fw_nonhttp_threshold: u32,
    pub fw_timeout: u32,
    pub fw_decision_delay: Duration,
    pub fw_http_cooldown: Duration,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            enable_firewall_set: false,
            fw_type: None,
            fw_set_name: None,
            fw_drop: false,
            fw_ua_whitelist: Vec::new(),
            fw_bypass: false,
            fw_nonhttp_threshold: 5,
            fw_timeout: 8 * 3600,
            fw_decision_delay: Duration::from_secs(60),
            fw_http_cooldown: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone, Debug)]
pub enum MatchMode {
    Keywords(Vec<String>),
    Force,
    Regex { pattern: String, partial: bool },
}

#[derive(Parser, Clone, Debug)]
#[command(name = "uaforge", version = "0.1.1", about = "User-Agent modification proxy")]
pub struct CliArgs {
    #[arg(short = 'u', long, default_value = "FFF", help = "User-Agent string to use")]
    pub user_agent: String,

    #[arg(long, default_value = "8080", help = "Port to listen on")]
    pub port: u16,

    #[arg(long, default_value = "info", help = "Log level (debug/info/warn/error)")]
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

    #[arg(long, default_value = "8192", help = "Buffer size (1024-65536)")]
    pub buffer_size: usize,

    #[arg(short = 'p', long, default_value = "0", help = "Connection pool size (0=auto)")]
    pub pool_size: usize,

    #[arg(long, help = "Force replace all User-Agents")]
    pub force: bool,

    #[arg(long, help = "Enable regex mode")]
    pub enable_regex: bool,

    #[arg(short = 's', long, help = "Enable partial regex replacement")]
    pub partial_replace: bool,

    // Firewall options
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

#[derive(Clone, Debug)]
pub struct Config {
    pub user_agent: String,
    pub port: u16,
    pub log_level: String,
    pub show_version: bool,
    pub log_file: Option<String>,
    pub whitelist: Vec<String>,
    pub cache_size: usize,
    pub buffer_size: usize,
    pub pool_size: usize,
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

        // Validate buffer size
        if cli.buffer_size < 1024 || cli.buffer_size > 65536 {
            return Err("invalid buffer-size (expected 1024..65536)".to_string());
        }

        // Validate and adjust pool size
        let pool_size = if cli.pool_size == 0 {
            DEFAULT_POOL_SIZE
        } else if cli.pool_size > MAX_POOL_SIZE {
            MAX_POOL_SIZE
        } else {
            cli.pool_size
        };

        // Determine match mode
        let match_mode = if cli.force {
            MatchMode::Force
        } else if cli.enable_regex {
            let pattern = cli.regex_pattern.unwrap_or_else(|| {
                "(iPhone|iPad|Android|Macintosh|Windows|Linux|Apple|Mac OS X|Mobile)".to_string()
            });
            MatchMode::Regex {
                pattern,
                partial: cli.partial_replace,
            }
        } else {
            let keywords = cli
                .keywords
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            MatchMode::Keywords(keywords)
        };

        // Build firewall config
        let mut firewall = FirewallConfig::default();
        if cli.fw_type.is_some() || cli.fw_set_name.is_some() {
            firewall.enable_firewall_set = true;
            firewall.fw_type = cli.fw_type;
            firewall.fw_set_name = cli.fw_set_name;
        }
        firewall.fw_drop = cli.fw_drop;
        firewall.fw_ua_whitelist = cli.fw_ua_w;
        firewall.fw_bypass = cli.fw_bypass;
        firewall.fw_nonhttp_threshold = cli.fw_nonhttp_threshold;
        firewall.fw_timeout = cli.fw_timeout;
        if let Some(delay) = cli.fw_decision_delay {
            firewall.fw_decision_delay = delay;
        }
        if let Some(cooldown) = cli.fw_http_cooldown {
            firewall.fw_http_cooldown = cooldown;
        }

        Ok(Self {
            user_agent: cli.user_agent,
            port: cli.port,
            log_level: cli.loglevel,
            show_version: cli.version,
            log_file: cli.log,
            whitelist: cli.whitelist,
            cache_size: cli.cache_size,
            buffer_size: cli.buffer_size,
            pool_size,
            match_mode,
            firewall,
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
