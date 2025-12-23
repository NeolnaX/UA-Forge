use std::net::IpAddr;
use std::sync::Arc;
use hyper::Request;
use hyper::header::{HeaderValue, USER_AGENT};

use crate::config::Config;
use crate::stats::Stats;
use crate::firewall::FirewallManager;
use crate::logger;
use crate::lru::Cache;
use std::sync::Mutex;

pub struct HttpHandler {
    config: Config,
    stats: Arc<Stats>,
    fw: Arc<FirewallManager>,
    cache: Arc<Mutex<Cache>>,
}

impl HttpHandler {
    pub fn new(config: Config, stats: Arc<Stats>, fw: Arc<FirewallManager>) -> Self {
        let cache = Arc::new(Mutex::new(Cache::new(config.cache_size)));
        Self { config, stats, fw, cache }
    }

    /// 修改 HTTP 请求的 User-Agent（流式版本）
    pub async fn modify_request(
        &self,
        mut req: Request<hyper::body::Incoming>,
        dest_ip: IpAddr,
        dest_port: u16,
    ) -> Result<Request<hyper::body::Incoming>, Box<dyn std::error::Error + Send + Sync>> {
        // 报告 HTTP 流量到防火墙
        self.fw.report_http(dest_ip, dest_port);
        self.stats.inc_http_requests();

        // 获取原始 User-Agent
        let original_ua = req.headers()
            .get(USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if original_ua.is_empty() {
            return Ok(req);
        }

        // 1. 检查防火墙 UA 白名单（最高优先级）
        if self.fw.enabled() && !self.config.firewall.fw_ua_whitelist.is_empty() {
            // 先检查缓存，避免重复添加防火墙规则
            let is_fw_whitelisted = if self.config.cache_size > 0 {
                if let Ok(mut cache) = self.cache.lock() {
                    if let Some(cached) = cache.get(&original_ua) {
                        cached == "FW_WHITELIST"
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if is_fw_whitelisted {
                // 缓存命中，直接返回
                return Ok(req);
            }

            // 检查是否在白名单中
            for keyword in &self.config.firewall.fw_ua_whitelist {
                if original_ua.contains(keyword) {
                    logger::log(
                        logger::Level::Info,
                        &format!("Firewall UA whitelist hit: {} (keyword: {})", original_ua, keyword)
                    );

                    // 添加到防火墙规则（只在第一次）
                    self.fw.add(dest_ip, dest_port, self.config.firewall.fw_timeout);

                    // 缓存防火墙白名单状态
                    if self.config.cache_size > 0 {
                        if let Ok(mut cache) = self.cache.lock() {
                            cache.put(original_ua.clone(), "FW_WHITELIST".to_string());
                        }
                    }

                    // 如果启用了 fw_drop，断开连接强制重连
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
        let should_modify = if self.config.cache_size > 0 {
            if let Ok(mut cache) = self.cache.lock() {
                if let Some(cached_result) = cache.get(&original_ua) {
                    // 缓存命中
                    if cached_result == "PASS" {
                        self.stats.inc_cache_pass();
                        false
                    } else {
                        self.stats.inc_cache_modify();
                        true
                    }
                } else {
                    // 缓存未命中
                    true
                }
            } else {
                true
            }
        } else {
            true
        };

        // 如果需要修改
        if should_modify {
            let new_ua = self.config.user_agent.clone();
            
            req.headers_mut().insert(
                USER_AGENT,
                HeaderValue::from_str(&new_ua)?
            );
            self.stats.inc_modified();

            // 更新缓存：记录需要修改
            if self.config.cache_size > 0 {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.put(original_ua.clone(), "MODIFY".to_string());
                }
            }

            logger::log(
                logger::Level::Debug,
                &format!("UA modified: {} -> {}", original_ua, new_ua)
            );
        } else {
            // 不需要修改，更新缓存记录
            if self.config.cache_size > 0 {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.put(original_ua.clone(), "PASS".to_string());
                }
            }
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
