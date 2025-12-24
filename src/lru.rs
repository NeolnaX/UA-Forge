use lru::LruCache;
use std::num::NonZeroUsize;

// Type-safe cache decisions (zero-cost enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CacheDecision {
    FwWhitelist = 0,
    Modify = 1,
    Pass = 2,
}

pub struct Cache {
    inner: LruCache<String, CacheDecision>,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        // 允许 cap=0 表示禁用缓存，使用最小值 1 避免 panic
        let capacity = NonZeroUsize::new(cap.max(1)).unwrap();
        Self {
            inner: LruCache::new(capacity),
        }
    }

    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.inner.cap().get() == 1
    }

    pub fn get(&mut self, key: &str) -> Option<CacheDecision> {
        self.inner.get(key).copied()
    }

    pub fn put(&mut self, key: String, value: CacheDecision) {
        self.inner.put(key, value);
    }
}
