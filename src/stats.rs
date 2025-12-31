use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Condvar};
use std::thread;
use std::time::{Duration, Instant};
use std::sync::atomic::AtomicBool;

pub struct Stats {
    // NOTE: mipsel_24kc does not guarantee 64-bit atomics, so use AtomicUsize for portability.
    // These counters may wrap on 32-bit targets; this is acceptable for runtime stats display.
    active_connections: AtomicUsize,
    http_requests: AtomicUsize,
    modified_requests: AtomicUsize,
    cache_hit_modify: AtomicUsize,
    cache_hit_pass: AtomicUsize,
    stop: AtomicBool,
    writer_handle: Mutex<Option<thread::JoinHandle<()>>>,
    stop_cond: Condvar,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            http_requests: AtomicUsize::new(0),
            modified_requests: AtomicUsize::new(0),
            cache_hit_modify: AtomicUsize::new(0),
            cache_hit_pass: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            writer_handle: Mutex::new(None),
            stop_cond: Condvar::new(),
        }
    }

    pub fn inc_active(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_active(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn inc_http_requests(&self) {
        self.http_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_modified(&self) {
        self.modified_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_modify(&self) {
        self.cache_hit_modify.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_pass(&self) {
        self.cache_hit_pass.fetch_add(1, Ordering::Relaxed);
    }

    pub fn start_writer(self: &Arc<Self>, path: &str, interval: Duration) {
        let stats = Arc::clone(self);
        let path = path.to_string();
        let handle = thread::spawn(move || {
            let mut last_http = 0u64;
            let mut last = Instant::now();
            let dummy_mutex = Mutex::new(());
            loop {
                // 使用 Condvar 等待，提升退出响应性
                let guard = dummy_mutex.lock().unwrap();
                let (guard, _timeout) = stats.stop_cond.wait_timeout(guard, interval).unwrap();
                drop(guard);

                if stats.stop.load(Ordering::Relaxed) {
                    return;
                }
                let active = stats.active_connections.load(Ordering::Relaxed) as u64;
                let http = stats.http_requests.load(Ordering::Relaxed) as u64;
                let modified = stats.modified_requests.load(Ordering::Relaxed) as u64;
                let cache_mod = stats.cache_hit_modify.load(Ordering::Relaxed) as u64;
                let cache_pass = stats.cache_hit_pass.load(Ordering::Relaxed) as u64;

                let now = Instant::now();
                let secs = now.duration_since(last).as_secs_f64();
                let rps = if secs > 0.0 {
                    (http.saturating_sub(last_http)) as f64 / secs
                } else {
                    0.0
                };
                last_http = http;
                last = now;

                let total_cache = cache_mod + cache_pass;
                let rule_processing = http.saturating_sub(total_cache);
                let direct_pass = http.saturating_sub(modified);
                let cache_ratio = if http > 0 {
                    (total_cache as f64) * 100.0 / (http as f64)
                } else {
                    0.0
                };

                let content = format!(
                    "current_connections:{active}\n\
total_requests:{http}\n\
rps:{rps:.2}\n\
successful_modifications:{modified}\n\
direct_passthrough:{direct_pass}\n\
rule_processing:{rule_processing}\n\
cache_hit_modify:{cache_mod}\n\
cache_hit_pass:{cache_pass}\n\
total_cache_ratio:{cache_ratio:.2}\n"
                );

                // 原子写入：先写临时文件，再 rename（避免 LuCI 读到半截）
                let tmp_path = format!("{}.tmp", path);
                if fs::write(&tmp_path, &content).is_ok() {
                    let _ = fs::rename(&tmp_path, &path);
                }
            }
        });

        // 存储线程句柄
        if let Ok(mut guard) = self.writer_handle.lock() {
            *guard = Some(handle);
        }
    }
}

impl Drop for Stats {
    fn drop(&mut self) {
        // 设置停止标志
        self.stop.store(true, Ordering::Relaxed);

        // 通知 Condvar 唤醒等待线程
        self.stop_cond.notify_all();

        // 等待后台线程结束
        if let Ok(mut guard) = self.writer_handle.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}
