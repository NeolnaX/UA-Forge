use std::net::IpAddr;
use std::sync::Arc;
use hyper::Request;
use hyper::header::{HeaderValue, USER_AGENT};

use crate::config::Config;
use crate::stats::Stats;
use crate::firewall::FirewallManager;
use crate::logger;
use crate::lru::Cache;
use parking_lot::Mutex;
use regex::Regex;

// Type-safe cache decisions (zero-cost enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum CacheDecision {
    FwWhitelist = 0,
    Modify = 1,
    Pass = 2,
}

impl CacheDecision {
    #[inline]
    const fn as_str(self) -> &'static str {
        match self {
            Self::FwWhitelist => "FW_WHITELIST",
            Self::Modify => "MODIFY",
            Self::Pass => "PASS",
        }
    }

    #[inline]
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "FW_WHITELIST" => Some(Self::FwWhitelist),
            "MODIFY" => Some(Self::Modify),
            "PASS" => Some(Self::Pass),
            _ => None,
        }
    }
}

pub struct HttpHandler {
    config: Config,
    stats: Arc<Stats>,
    fw: Arc<FirewallManager>,
    cache: Arc<Mutex<Cache>>,
    regex_cache: Option<Regex>,
    user_agent_header: HeaderValue,
}

impl HttpHandler {
    pub fn new(config: Config, stats: Arc<Stats>, fw: Arc<FirewallManager>) -> Self {
        let cache = Arc::new(Mutex::new(Cache::new(config.cache_size)));

        // Pre-compile regex if in Regex mode
        let regex_cache = if let crate::config::MatchMode::Regex { pattern, .. } = &config.match_mode {
            Regex::new(pattern).ok()
        } else {
            None
        };

        // Pre-convert user_agent to HeaderValue (validated once)
        let user_agent_header = HeaderValue::from_str(&config.user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static("UAForge"));

        Self { config, stats, fw, cache, regex_cache, user_agent_header }
    }

    /// 从缓存中获取值
    fn cache_get(&self, key: &str) -> Option<CacheDecision> {
        if self.config.cache_size == 0 {
            return None;
        }
        let mut cache = self.cache.lock();
        let value = cache.get(key)?;
        CacheDecision::from_str(&value)
    }

    /// 向缓存中写入值
    fn cache_put(&self, key: &str, value: CacheDecision) {
        if self.config.cache_size == 0 {
            return;
        }
        let mut cache = self.cache.lock();
        cache.put(key.to_string(), value.as_str().to_string());
    }

    /// 判断 UA 是否需要修改（规则匹配）
    fn should_modify_ua(&self, ua: &str) -> bool {
        use crate::config::MatchMode;

        match &self.config.match_mode {
            MatchMode::Force => {
                // 强制修改所有 UA
                true
            }
            MatchMode::Keywords(keywords) => {
                // 检查 UA 是否包含任意关键词
                keywords.iter().any(|kw| ua.contains(kw.as_str()))
            }
            MatchMode::Regex { .. } => {
                // Use pre-compiled regex
                if let Some(ref re) = self.regex_cache {
                    re.is_match(ua)
                } else {
                    false
                }
            }
        }
    }

    /// 修改 HTTP 请求的 User-Agent（流式版本）
    pub async fn modify_request(
        &self,
        mut req: Request<hyper::body::Incoming>,
        dest_ip: IpAddr,
        dest_port: u16,
    ) -> Result<Request<hyper::body::Incoming>, Box<dyn std::error::Error + Send + Sync>> {
        self.fw.report_http(dest_ip, dest_port);
        self.stats.inc_http_requests();

        // Extract UA as owned String to avoid borrow conflicts
        let original_ua: String = match req.headers().get(USER_AGENT) {
            Some(v) => match v.to_str() {
                Ok(s) => s.to_string(),
                Err(_) => return Ok(req),
            },
            None => return Ok(req),
        };

        if original_ua.is_empty() {
            return Ok(req);
        }

        // 1. 检查防火墙 UA 白名单（最高优先级）
        if self.fw.enabled() && !self.config.firewall.fw_ua_whitelist.is_empty() {
            // 先检查缓存，避免重复添加防火墙规则
            if let Some(cached) = self.cache_get(&original_ua) {
                if cached == CacheDecision::FwWhitelist {
                    return Ok(req);
                }
            }

            // 检查是否在白名单中
            for keyword in &self.config.firewall.fw_ua_whitelist {
                if original_ua.contains(keyword.as_str()) {
                    logger::log(
                        logger::Level::Info,
                        &format!("Firewall UA whitelist hit: {} (keyword: {})", original_ua, keyword)
                    );

                    self.fw.add(dest_ip, dest_port, self.config.firewall.fw_timeout);
                    self.cache_put(&original_ua, CacheDecision::FwWhitelist);

                    if self.config.firewall.fw_drop {
                        logger::log(
                            logger::Level::Info,
                            &format!("Dropping connection for {}:{} to force bypass", dest_ip, dest_port)
                        );
                        return Err("Connection dropped: UA whitelist match (will bypass on reconnect)".into());
                    }

                    return Ok(req);
                }
            }
        }

        // 检查缓存：缓存记录是否需要修改
        let should_modify = if let Some(cached_result) = self.cache_get(&original_ua) {
            // 缓存命中
            if cached_result == CacheDecision::Pass {
                self.stats.inc_cache_pass();
                false
            } else {
                self.stats.inc_cache_modify();
                true
            }
        } else {
            // 缓存未命中 - 执行规则匹配
            self.should_modify_ua(&original_ua)
        };

        // 如果需要修改
        if should_modify {
            req.headers_mut().insert(USER_AGENT, self.user_agent_header.clone());
            self.stats.inc_modified();
            self.cache_put(&original_ua, CacheDecision::Modify);

            logger::log(
                logger::Level::Debug,
                &format!("UA modified: {} -> {}", original_ua, self.config.user_agent)
            );
        } else {
            self.cache_put(&original_ua, CacheDecision::Pass);
        }

        Ok(req)
    }

    /// 报告非 HTTP 流量给防火墙
    pub fn report_non_http(&self, dest_ip: IpAddr, dest_port: u16) {
        if self.fw.enabled() && self.config.firewall.fw_bypass {
            self.fw.report_non_http(dest_ip, dest_port);
        }
    }
}
