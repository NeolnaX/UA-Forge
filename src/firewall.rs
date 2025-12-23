use std::collections::HashMap;
use std::net::IpAddr;
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::FirewallConfig;
use crate::logger;

#[derive(Clone)]
pub struct FirewallManager {
    inner: Arc<Inner>,
}

// 注意：不能为 Inner 派生 Debug，因为 JoinHandle 不实现 Debug
struct Inner {
    config: FirewallConfig,
    tx: mpsc::Sender<Event>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

#[derive(Debug)]
enum Event {
    Http { ip: IpAddr, port: u16 },
    NonHttp { ip: IpAddr, port: u16 },
    Add { ip: IpAddr, port: u16, timeout: u32 },
    Stop,
}

#[derive(Debug)]
struct PortProfile {
    non_http_score: u32,
    http_lock_expires: Option<Instant>,
    last_event: Instant,
    decision_deadline: Option<Instant>,
}

impl PortProfile {
    fn new(now: Instant) -> Self {
        Self {
            non_http_score: 0,
            http_lock_expires: None,
            last_event: now,
            decision_deadline: None,
        }
    }
}

impl FirewallManager {
    pub fn new(cfg: FirewallConfig) -> Self {
        let (tx, rx) = mpsc::channel::<Event>();

        let worker_config = cfg.clone();
        let handle = thread::spawn(move || worker(worker_config, rx));

        let inner = Arc::new(Inner {
            config: cfg,
            tx,
            handle: Mutex::new(Some(handle)),
        });

        Self { inner }
    }

    pub fn enabled(&self) -> bool {
        self.inner.config.enable_firewall_set
            && self.inner.config.fw_set_name.as_deref().unwrap_or("").len() > 0
            && self.inner.config.fw_type.as_deref().unwrap_or("").len() > 0
    }

    pub fn report_http(&self, ip: IpAddr, port: u16) {
        if !self.enabled() {
            return;
        }
        let _ = self.inner.tx.send(Event::Http { ip, port });
    }

    pub fn report_non_http(&self, ip: IpAddr, port: u16) {
        if !self.enabled() || !self.inner.config.fw_bypass {
            return;
        }
        let _ = self.inner.tx.send(Event::NonHttp { ip, port });
    }

    pub fn add(&self, ip: IpAddr, port: u16, timeout: u32) {
        if !self.enabled() {
            return;
        }
        let _ = self.inner.tx.send(Event::Add { ip, port, timeout });
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        let _ = self.inner.tx.send(Event::Stop);
    }
}

impl Drop for FirewallManager {
    fn drop(&mut self) {
        // 发送停止信号
        let _ = self.inner.tx.send(Event::Stop);

        // 等待后台线程结束
        if let Ok(mut guard) = self.inner.handle.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

fn worker(fw_config: FirewallConfig, rx: mpsc::Receiver<Event>) {
    let profiles: HashMap<(IpAddr, u16), PortProfile> = HashMap::new();
    let profiles = Arc::new(Mutex::new(profiles));

    // Batch state: dedup by ip:port; single set/type pair in current OpenWrt usage.
    let mut batch: HashMap<(IpAddr, u16), u32> = HashMap::new();
    let mut batch_deadline: Option<Instant> = None;

    let cleanup_interval = Duration::from_secs(10 * 60);
    let mut cleanup_deadline = Instant::now() + cleanup_interval;

    loop {
        let now = Instant::now();
        let next = min_instant(batch_deadline, cleanup_deadline, decision_deadline(&profiles));
        let timeout = next.map(|t| t.saturating_duration_since(now));

        let evt = match timeout {
            Some(t) => rx.recv_timeout(t).ok(),
            None => rx.recv().ok(),
        };

        let now = Instant::now();
        if let Some(e) = evt {
            match e {
                Event::Stop => break,
                Event::Add { ip, port, timeout } => {
                    batch.insert((ip, port), timeout);
                    if batch_deadline.is_none() {
                        batch_deadline = Some(now + Duration::from_millis(100));
                    }
                    if batch.len() >= 200 {
                        flush_batch(&fw_config, &mut batch);
                        batch_deadline = None;
                    }
                }
                Event::Http { ip, port } => {
                    let mut guard = match profiles.lock() {
                        Ok(g) => g,
                        Err(e) => {
                            logger::log(
                                logger::Level::Warn,
                                "firewall profiles mutex poisoned (http), recovering",
                            );
                            e.into_inner()
                        }
                    };
                    let p = guard
                        .entry((ip, port))
                        .or_insert_with(|| PortProfile::new(Instant::now()));

                    // Within cooldown, ignore.
                    if p.http_lock_expires.is_some_and(|t| now < t) {
                        continue;
                    }

                    p.non_http_score = 0;
                    p.http_lock_expires = Some(now + fw_config.fw_http_cooldown);
                    p.decision_deadline = None;
                    p.last_event = now;
                }
                Event::NonHttp { ip, port } => {
                    let mut guard = match profiles.lock() {
                        Ok(g) => g,
                        Err(e) => {
                            logger::log(
                                logger::Level::Warn,
                                "firewall profiles mutex poisoned (non-http), recovering",
                            );
                            e.into_inner()
                        }
                    };
                    let p = guard
                        .entry((ip, port))
                        .or_insert_with(|| PortProfile::new(Instant::now()));

                    // Ignore during HTTP cooldown.
                    if p.http_lock_expires.is_some_and(|t| now < t) {
                        p.last_event = now;
                        continue;
                    }

                    p.non_http_score = p.non_http_score.saturating_add(1);
                    p.last_event = now;

                    if p.non_http_score >= fw_config.fw_nonhttp_threshold {
                        if p.decision_deadline.is_none() {
                            p.decision_deadline = Some(now + fw_config.fw_decision_delay);
                        }
                    }
                }
            }
        }

        // Timers: batch flush
        if batch_deadline.is_some_and(|t| Instant::now() >= t) && !batch.is_empty() {
            flush_batch(&fw_config, &mut batch);
            batch_deadline = None;
        }

        // Timers: finalize decisions
        finalize_decisions(&fw_config, &profiles, &mut batch, &mut batch_deadline);

        // Timers: cleanup
        if Instant::now() >= cleanup_deadline {
            cleanup_profiles(&profiles, cleanup_interval);
            cleanup_deadline = Instant::now() + cleanup_interval;
        }
    }

    if !batch.is_empty() {
        flush_batch(&fw_config, &mut batch);
    }
}

fn decision_deadline(profiles: &Arc<Mutex<HashMap<(IpAddr, u16), PortProfile>>>) -> Option<Instant> {
    let guard = profiles.lock().ok()?;
    guard
        .values()
        .filter_map(|p| p.decision_deadline)
        .min()
}

fn finalize_decisions(
    fw_config: &FirewallConfig,
    profiles: &Arc<Mutex<HashMap<(IpAddr, u16), PortProfile>>>,
    batch: &mut HashMap<(IpAddr, u16), u32>,
    batch_deadline: &mut Option<Instant>,
) {
    let now = Instant::now();
    let mut add_list: Vec<(IpAddr, u16, u32)> = Vec::new();

    {
        let mut guard = match profiles.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let keys: Vec<(IpAddr, u16)> = guard
            .iter()
            .filter_map(|(k, p)| {
                if let Some(deadline) = p.decision_deadline {
                    if now >= deadline
                        && p.non_http_score >= fw_config.fw_nonhttp_threshold
                        && !p.http_lock_expires.is_some_and(|t| now < t)
                    {
                        return Some(*k);
                    }
                }
                None
            })
            .collect();

        for k in keys {
            guard.remove(&k);
            add_list.push((k.0, k.1, fw_config.fw_timeout));
        }
    }

    for (ip, port, timeout) in add_list {
        batch.insert((ip, port), timeout);
        if batch_deadline.is_none() {
            *batch_deadline = Some(Instant::now() + Duration::from_millis(100));
        }
    }
}

fn cleanup_profiles(
    profiles: &Arc<Mutex<HashMap<(IpAddr, u16), PortProfile>>>,
    interval: Duration,
) {
    let now = Instant::now();
    let mut guard = match profiles.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    guard.retain(|_, p| {
        // Keep if in decision window or cooldown.
        if p.decision_deadline.is_some() {
            return true;
        }
        if p.http_lock_expires.is_some_and(|t| now < t) {
            return true;
        }
        now.duration_since(p.last_event) <= interval
    });
}

fn flush_batch(fw_config: &FirewallConfig, batch: &mut HashMap<(IpAddr, u16), u32>) {
    let fw_type = fw_config.fw_type.as_deref().unwrap_or("");
    let set_name = fw_config.fw_set_name.as_deref().unwrap_or("");
    if fw_type.is_empty() || set_name.is_empty() {
        batch.clear();
        return;
    }

    let items: Vec<(IpAddr, u16, u32)> = batch
        .drain()
        .map(|((ip, port), timeout)| (ip, port, timeout))
        .collect();

    if items.is_empty() {
        return;
    }

    let result = match fw_type {
        "nft" => flush_nft(set_name, &items),
        _ => flush_ipset(set_name, &items),
    };
    if let Err(e) = result {
        logger::log(
            logger::Level::Warn,
            &format!("firewall batch failed ({fw_type}/{set_name}): {e}"),
        );
    }
}

fn flush_ipset(set_name: &str, items: &[(IpAddr, u16, u32)]) -> std::io::Result<()> {
    let mut stdin = String::new();
    for (ip, port, timeout) in items {
        if timeout > &0 {
            stdin.push_str(&format!(
                "add {set_name} {ip},{port} timeout {timeout} -exist\n"
            ));
        } else {
            stdin.push_str(&format!("add {set_name} {ip},{port} -exist\n"));
        }
    }
    let mut cmd = Command::new("ipset");
    cmd.arg("restore");
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut s) = child.stdin.take() {
        use std::io::Write;
        let _ = s.write_all(stdin.as_bytes());
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("ipset restore failed: {}", String::from_utf8_lossy(&out.stderr)),
        ));
    }
    Ok(())
}

fn flush_nft(set_name: &str, items: &[(IpAddr, u16, u32)]) -> std::io::Result<()> {
    // nft add element inet fw4 <setName> { <ip> . <port> timeout <t>, ... }
    let mut elements = String::new();
    for (idx, (ip, port, timeout)) in items.iter().enumerate() {
        if idx > 0 {
            elements.push_str(", ");
        }
        elements.push_str(&format!("{ip} . {port}"));
        if *timeout > 0 {
            elements.push_str(&format!(" timeout {timeout}s"));
        }
    }
    let mut cmd = Command::new("nft");
    cmd.args([
        "add",
        "element",
        "inet",
        "fw4",
        set_name,
        "{",
        &elements,
        "}",
    ]);
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("nft failed: {}", String::from_utf8_lossy(&out.stderr)),
        ));
    }
    Ok(())
}

fn min_instant(
    a: Option<Instant>,
    b: Instant,
    c: Option<Instant>,
) -> Option<Instant> {
    let mut out = Some(b);
    if let Some(x) = a {
        out = Some(out.map(|o| o.min(x)).unwrap_or(x));
    }
    if let Some(x) = c {
        out = Some(out.map(|o| o.min(x)).unwrap_or(x));
    }
    out
}
