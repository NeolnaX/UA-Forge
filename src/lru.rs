use lru::LruCache;
use std::num::NonZeroUsize;

pub struct Cache {
    inner: LruCache<String, String>,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        let capacity = NonZeroUsize::new(cap.max(1)).unwrap();
        Self {
            inner: LruCache::new(capacity),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<String> {
        self.inner.get(key).cloned()
    }

    pub fn put(&mut self, key: String, value: String) {
        self.inner.put(key, value);
    }
}
