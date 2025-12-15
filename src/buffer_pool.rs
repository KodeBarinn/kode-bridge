use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing::debug;

/// Thread-safe buffer pool for reducing memory allocations
#[derive(Debug, Clone)]
pub struct BufferPool {
    buffers: Arc<Mutex<VecDeque<Vec<u8>>>>,
    buffer_size: usize,
    max_pool_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new(buffer_size: usize, max_pool_size: usize) -> Self {
        Self {
            buffers: Arc::new(Mutex::new(VecDeque::with_capacity(max_pool_size))),
            buffer_size,
            max_pool_size,
        }
    }

    /// Get a buffer from the pool or create a new one
    pub fn get(&self) -> PooledBuffer {
        let mut buffer = {
            let mut buffers = self.buffers.lock();
            buffers.pop_front().unwrap_or_else(|| {
                debug!("Creating new buffer of size {}", self.buffer_size);
                Vec::with_capacity(self.buffer_size)
            })
        };

        // Ensure buffer has the right capacity and is cleared
        buffer.clear();
        if buffer.capacity() < self.buffer_size {
            buffer.reserve(self.buffer_size - buffer.capacity());
        }

        PooledBuffer {
            buffer,
            pool: Arc::downgrade(&self.buffers),
            max_pool_size: self.max_pool_size,
        }
    }

    /// Get current pool size for monitoring
    pub fn size(&self) -> usize {
        self.buffers.lock().len()
    }

    /// Pre-warm the pool with buffers
    pub fn warm_up(&self, count: usize) {
        let to_create = {
            let mut buffers = self.buffers.lock();
            let current_size = buffers.len();
            let to_create = (count.saturating_sub(current_size)).min(self.max_pool_size - current_size);

            for _ in 0..to_create {
                buffers.push_back(Vec::with_capacity(self.buffer_size));
            }

            to_create
        };

        debug!("Buffer pool warmed up with {} buffers", to_create);
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(16384, 64) // 16KB buffers, max 64 in pool (increased for better performance)
    }
}

/// RAII wrapper for pooled buffers that returns buffer to pool on drop
pub struct PooledBuffer {
    buffer: Vec<u8>,
    pool: std::sync::Weak<Mutex<VecDeque<Vec<u8>>>>,
    max_pool_size: usize,
}

impl PooledBuffer {
    /// Get mutable reference to the underlying buffer
    pub const fn as_mut_vec(&mut self) -> &mut Vec<u8> {
        &mut self.buffer
    }

    /// Get reference to the underlying buffer
    pub const fn as_vec(&self) -> &Vec<u8> {
        &self.buffer
    }

    /// Get the buffer as a byte slice
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    /// Get the buffer capacity
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Get the buffer length
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Extend buffer with slice
    pub fn extend_from_slice(&mut self, other: &[u8]) {
        self.buffer.extend_from_slice(other);
    }

    /// Push a single byte
    pub fn push(&mut self, byte: u8) {
        self.buffer.push(byte);
    }

    /// Reserve additional capacity
    pub fn reserve(&mut self, additional: usize) {
        self.buffer.reserve(additional);
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            let returned = {
                let mut buffers = pool.lock();
                if buffers.len() < self.max_pool_size && self.buffer.capacity() >= 1024 {
                    // Only return buffer to pool if it's reasonably sized and pool has space
                    let mut returned_buffer = std::mem::take(&mut self.buffer);
                    returned_buffer.clear();
                    buffers.push_back(returned_buffer);
                    true
                } else {
                    false
                }
            };

            if returned {
                debug!("Buffer returned to pool");
            }
        }
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}

/// Global buffer pools for different use cases
pub struct GlobalBufferPools {
    /// Small buffers for headers and small data
    small: BufferPool,
    /// Medium buffers for typical HTTP requests/responses
    medium: BufferPool,
    /// Large buffers for big payloads
    large: BufferPool,
    /// Extra large buffers for very large PUT/POST requests
    extra_large: BufferPool,
}

impl GlobalBufferPools {
    pub fn new() -> Self {
        Self {
            small: BufferPool::new(2048, 32),         // 2KB buffers, more instances
            medium: BufferPool::new(16384, 64),       // 16KB buffers, doubled size and count
            large: BufferPool::new(131072, 16),       // 128KB buffers, doubled size, more instances
            extra_large: BufferPool::new(1048576, 8), // 1MB buffers for very large requests
        }
    }

    /// Get appropriate buffer based on expected size with better size selection
    pub fn get_buffer(&self, expected_size: usize) -> PooledBuffer {
        if expected_size <= 2048 {
            self.small.get()
        } else if expected_size <= 16384 {
            self.medium.get()
        } else if expected_size <= 131072 {
            self.large.get()
        } else {
            self.extra_large.get()
        }
    }

    /// Get small buffer (for headers, small data)
    pub fn get_small(&self) -> PooledBuffer {
        self.small.get()
    }

    /// Get medium buffer (for typical HTTP data)
    pub fn get_medium(&self) -> PooledBuffer {
        self.medium.get()
    }

    /// Get large buffer (for big payloads)
    pub fn get_large(&self) -> PooledBuffer {
        self.large.get()
    }

    /// Get extra large buffer (for very large PUT/POST requests)
    pub fn get_extra_large(&self) -> PooledBuffer {
        self.extra_large.get()
    }

    /// Warm up all pools
    pub fn warm_up(&self) {
        self.small.warm_up(16); // More pre-warmed buffers
        self.medium.warm_up(32);
        self.large.warm_up(8);
        self.extra_large.warm_up(4);
    }

    /// Get pool statistics
    pub fn stats(&self) -> BufferPoolStats {
        BufferPoolStats {
            small_pool_size: self.small.size(),
            medium_pool_size: self.medium.size(),
            large_pool_size: self.large.size(),
            extra_large_pool_size: self.extra_large.size(),
        }
    }
}

impl Default for GlobalBufferPools {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct BufferPoolStats {
    pub small_pool_size: usize,
    pub medium_pool_size: usize,
    pub large_pool_size: usize,
    pub extra_large_pool_size: usize,
}

// Global instance - use lazy initialization
use std::sync::OnceLock;

static GLOBAL_POOLS: OnceLock<GlobalBufferPools> = OnceLock::new();

/// Get global buffer pools instance
pub fn global_pools() -> &'static GlobalBufferPools {
    GLOBAL_POOLS.get_or_init(|| {
        let pools = GlobalBufferPools::new();
        pools.warm_up();
        pools
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_basic() {
        let pool = BufferPool::new(1024, 4);

        {
            let mut buf1 = pool.get();
            buf1.extend_from_slice(b"hello");
            assert_eq!(buf1.len(), 5);
            assert!(buf1.capacity() >= 1024);
        }

        // Buffer should be returned to pool
        assert_eq!(pool.size(), 1);

        let mut buf2 = pool.get();
        assert_eq!(buf2.len(), 0); // Should be cleared
        buf2.extend_from_slice(b"world");
        assert_eq!(buf2.as_slice(), b"world");
    }

    #[test]
    fn test_buffer_pool_max_size() {
        let pool = BufferPool::new(1024, 2);

        let _buf1 = pool.get();
        let _buf2 = pool.get();
        let _buf3 = pool.get();

        // When all buffers drop, only 2 should be retained
        drop(_buf1);
        drop(_buf2);
        drop(_buf3);

        assert_eq!(pool.size(), 2);
    }

    #[test]
    fn test_global_pools() {
        let pools = global_pools();

        let mut small_buf = pools.get_small();
        small_buf.extend_from_slice(b"small");

        let mut medium_buf = pools.get_medium();
        medium_buf.extend_from_slice(b"medium data");

        let mut large_buf = pools.get_large();
        large_buf.extend_from_slice(b"large data payload");

        assert_eq!(small_buf.as_slice(), b"small");
        assert_eq!(medium_buf.as_slice(), b"medium data");
        assert_eq!(large_buf.as_slice(), b"large data payload");

        // Check capacities
        assert!(small_buf.capacity() >= 1024);
        assert!(medium_buf.capacity() >= 8192);
        assert!(large_buf.capacity() >= 65536);
    }

    #[test]
    fn test_buffer_selection() {
        let pools = GlobalBufferPools::new();

        let buf1 = pools.get_buffer(512); // Should get small
        let buf2 = pools.get_buffer(4096); // Should get medium
        let buf3 = pools.get_buffer(32768); // Should get large

        assert!(buf1.capacity() >= 1024);
        assert!(buf2.capacity() >= 8192);
        assert!(buf3.capacity() >= 65536);
    }
}
