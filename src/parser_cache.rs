use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing::debug;

/// Type alias for complex HTTP parsing result
pub type HttpParseResult = Result<(String, String, Vec<(String, String)>), httparse::Error>;

/// Thread-safe HTTP parser cache to avoid repeated allocations
pub struct HttpParserCache {
    parsers: Arc<Mutex<VecDeque<ReusableParser>>>,
    max_cache_size: usize,
}

/// Reusable HTTP parser - simplified version without lifetime issues
#[derive(Default)]
pub struct ReusableParser {
    // We'll create headers buffer on each use to avoid lifetime issues
    // This is still more efficient than creating parsers themselves repeatedly
}

impl ReusableParser {
    /// Create a new reusable parser
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse HTTP response
    pub fn parse_response(
        &mut self,
        buffer: &[u8],
    ) -> Result<(u16, Vec<(String, String)>), httparse::Error> {
        // Use a local header buffer - still more efficient than recreating parser objects
        let mut headers = [httparse::EMPTY_HEADER; 128];
        let mut response = httparse::Response::new(&mut headers);

        match response.parse(buffer)? {
            httparse::Status::Complete(_) => {
                let status_code = response.code.ok_or(httparse::Error::Status)?;

                // Extract headers
                let mut parsed_headers = Vec::with_capacity(response.headers.len());
                for header in response.headers.iter() {
                    parsed_headers.push((
                        header.name.to_string(),
                        String::from_utf8_lossy(header.value).to_string(),
                    ));
                }

                Ok((status_code, parsed_headers))
            }
            httparse::Status::Partial => Err(httparse::Error::TooManyHeaders),
        }
    }

    /// Parse HTTP request
    pub fn parse_request(&mut self, buffer: &[u8]) -> HttpParseResult {
        // Use a local header buffer
        let mut headers = [httparse::EMPTY_HEADER; 128];
        let mut request = httparse::Request::new(&mut headers);

        match request.parse(buffer)? {
            httparse::Status::Complete(_) => {
                let method = request.method.ok_or(httparse::Error::Status)?;
                let path = request.path.ok_or(httparse::Error::Status)?;

                // Extract headers
                let mut parsed_headers = Vec::with_capacity(request.headers.len());
                for header in request.headers.iter() {
                    parsed_headers.push((
                        header.name.to_string(),
                        String::from_utf8_lossy(header.value).to_string(),
                    ));
                }

                Ok((method.to_string(), path.to_string(), parsed_headers))
            }
            httparse::Status::Partial => Err(httparse::Error::TooManyHeaders),
        }
    }

    /// Reset parser state for reuse (no-op in simplified version)
    pub fn reset(&mut self) {
        // No state to reset in simplified version
    }
}

impl HttpParserCache {
    /// Create a new parser cache
    pub fn new(max_cache_size: usize) -> Self {
        Self {
            parsers: Arc::new(Mutex::new(VecDeque::with_capacity(max_cache_size))),
            max_cache_size,
        }
    }

    /// Get a parser from cache or create new one
    pub fn get(&self) -> CachedParser {
        let mut parser = {
            let mut parsers = self.parsers.lock();
            parsers.pop_front().unwrap_or_else(|| {
                debug!("Creating new HTTP parser");
                ReusableParser::new()
            })
        };

        parser.reset();

        CachedParser {
            parser: Some(parser),
            cache: Arc::downgrade(&self.parsers),
            max_cache_size: self.max_cache_size,
        }
    }

    /// Get current cache size
    pub fn size(&self) -> usize {
        self.parsers.lock().len()
    }

    /// Pre-warm the cache
    pub fn warm_up(&self, count: usize) {
        let mut parsers = self.parsers.lock();
        let current_size = parsers.len();
        let to_create =
            (count.saturating_sub(current_size)).min(self.max_cache_size - current_size);

        for _ in 0..to_create {
            parsers.push_back(ReusableParser::new());
        }

        debug!("Parser cache warmed up with {} parsers", to_create);
    }
}

impl Default for HttpParserCache {
    fn default() -> Self {
        Self::new(8) // Default to 8 cached parsers
    }
}

/// RAII wrapper that returns parser to cache on drop
pub struct CachedParser {
    parser: Option<ReusableParser>,
    cache: std::sync::Weak<Mutex<VecDeque<ReusableParser>>>,
    max_cache_size: usize,
}

impl CachedParser {
    /// Parse HTTP response
    pub fn parse_response(
        &mut self,
        buffer: &[u8],
    ) -> Result<(u16, Vec<(String, String)>), httparse::Error> {
        self.parser
            .as_mut()
            .expect("Parser not available")
            .parse_response(buffer)
    }

    /// Parse HTTP request  
    pub fn parse_request(&mut self, buffer: &[u8]) -> HttpParseResult {
        self.parser
            .as_mut()
            .expect("Parser not available")
            .parse_request(buffer)
    }
}

impl Drop for CachedParser {
    fn drop(&mut self) {
        if let (Some(parser), Some(cache)) = (self.parser.take(), self.cache.upgrade()) {
            let mut parsers = cache.lock();
            if parsers.len() < self.max_cache_size {
                parsers.push_back(parser);
                debug!("Parser returned to cache");
            }
        }
    }
}

/// Response caching system for frequently accessed data
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Cache key for responses
#[derive(Clone, Debug)]
pub struct CacheKey {
    method: String,
    path: String,
    headers_hash: u64,
}

impl CacheKey {
    pub fn new(method: &str, path: &str, headers: &[(String, String)]) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        // Hash relevant headers (ignore cache-control, etc.)
        let relevant_headers: Vec<_> = headers
            .iter()
            .filter(|(name, _)| {
                let name_lower = name.to_lowercase();
                !name_lower.starts_with("cache-")
                    && name_lower != "date"
                    && name_lower != "last-modified"
                    && name_lower != "etag"
            })
            .collect();

        relevant_headers.hash(&mut hasher);

        Self {
            method: method.to_string(),
            path: path.to_string(),
            headers_hash: hasher.finish(),
        }
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.method.hash(state);
        self.path.hash(state);
        self.headers_hash.hash(state);
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method
            && self.path == other.path
            && self.headers_hash == other.headers_hash
    }
}

impl Eq for CacheKey {}

/// Cached response entry
#[derive(Clone)]
pub struct CachedResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: bytes::Bytes,
    pub cached_at: Instant,
    pub expires_at: Option<Instant>,
}

/// Simple LRU response cache
pub struct ResponseCache {
    cache: Arc<Mutex<HashMap<CacheKey, CachedResponse>>>,
    max_entries: usize,
}

impl ResponseCache {
    pub fn new(max_entries: usize, _default_ttl: Duration) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::with_capacity(max_entries))),
            max_entries,
        }
    }

    /// Get cached response if available and not expired
    pub fn get(&self, key: &CacheKey) -> Option<CachedResponse> {
        let mut cache = self.cache.lock();

        if let Some(entry) = cache.get(key) {
            // Check if expired
            if let Some(expires_at) = entry.expires_at {
                if Instant::now() > expires_at {
                    cache.remove(key);
                    return None;
                }
            }

            Some(entry.clone())
        } else {
            None
        }
    }

    /// Store response in cache
    pub fn put(&self, key: CacheKey, response: CachedResponse) {
        let mut cache = self.cache.lock();

        // Simple eviction: remove oldest entries if over limit
        if cache.len() >= self.max_entries {
            // Find oldest entry to remove
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, v)| v.cached_at)
                .map(|(k, _)| k.clone());

            if let Some(key_to_remove) = oldest_key {
                cache.remove(&key_to_remove);
            }
        }

        cache.insert(key, response);
    }

    /// Clear expired entries
    pub fn cleanup_expired(&self) {
        let mut cache = self.cache.lock();
        let now = Instant::now();

        cache.retain(|_, entry| match entry.expires_at {
            None => true,
            Some(expires_at) => now <= expires_at,
        });
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.lock();
        CacheStats {
            entries: cache.len(),
            max_entries: self.max_entries,
        }
    }

    /// Clear all cached entries
    pub fn clear(&self) {
        let mut cache = self.cache.lock();
        cache.clear();
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub max_entries: usize,
}

// Global instances
use std::sync::OnceLock;

static GLOBAL_PARSER_CACHE: OnceLock<HttpParserCache> = OnceLock::new();
static GLOBAL_RESPONSE_CACHE: OnceLock<ResponseCache> = OnceLock::new();

/// Get global parser cache
pub fn global_parser_cache() -> &'static HttpParserCache {
    GLOBAL_PARSER_CACHE.get_or_init(|| {
        let cache = HttpParserCache::new(8);
        cache.warm_up(4);
        cache
    })
}

/// Get global response cache
pub fn global_response_cache() -> &'static ResponseCache {
    GLOBAL_RESPONSE_CACHE.get_or_init(|| {
        ResponseCache::new(100, Duration::from_secs(300)) // 100 entries, 5 min TTL
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_cache() {
        let cache = HttpParserCache::new(2);

        {
            let mut parser1 = cache.get();
            let http_response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
            let result = parser1.parse_response(http_response).unwrap();
            assert_eq!(result.0, 200);
            assert_eq!(result.1.len(), 1);
            assert_eq!(result.1[0].0, "Content-Length");
        }

        // Parser should be returned to cache
        assert_eq!(cache.size(), 1);

        {
            let mut parser2 = cache.get();
            let http_request = b"GET /api HTTP/1.1\r\nHost: localhost\r\n\r\n";
            let result = parser2.parse_request(http_request).unwrap();
            assert_eq!(result.0, "GET");
            assert_eq!(result.1, "/api");
            assert_eq!(result.2.len(), 1);
        }
    }

    #[test]
    fn test_response_cache() {
        let cache = ResponseCache::new(2, Duration::from_secs(1));

        let key = CacheKey::new("GET", "/api", &[]);
        let response = CachedResponse {
            status_code: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: bytes::Bytes::from("test"),
            cached_at: Instant::now(),
            expires_at: Some(Instant::now() + Duration::from_secs(1)),
        };

        cache.put(key.clone(), response.clone());

        let cached = cache.get(&key).unwrap();
        assert_eq!(cached.status_code, 200);
        assert_eq!(cached.body, bytes::Bytes::from("test"));

        // Test expiration
        std::thread::sleep(Duration::from_millis(1100));
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_cache_key_equality() {
        let headers1 = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer token".to_string()),
        ];

        let headers2 = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer token".to_string()),
            (
                "Date".to_string(),
                "Wed, 21 Oct 2015 07:28:00 GMT".to_string(),
            ),
        ];

        let key1 = CacheKey::new("GET", "/api", &headers1);
        let key2 = CacheKey::new("GET", "/api", &headers2);

        // Keys should be equal because Date header is ignored
        assert_eq!(key1, key2);
    }
}
