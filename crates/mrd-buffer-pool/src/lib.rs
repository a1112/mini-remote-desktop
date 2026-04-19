//! Buffer pool for reducing memory allocation overhead
//!
//! Provides reusable fixed-size buffers to minimize allocation/deallocation overhead
//! in performance-critical paths like video encoding and decoding.

use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferPoolConfig {
    /// Minimum number of buffers to keep in the pool
    pub min_buffers: usize,
    /// Maximum number of buffers to keep in the pool
    pub max_buffers: usize,
    /// Size of each buffer in bytes
    pub buffer_size: usize,
}

impl Default for BufferPoolConfig {
    fn default() -> Self {
        Self {
            min_buffers: 2,
            max_buffers: 8,
            buffer_size: 1920 * 1080 * 4, // 1080p BGRA32
        }
    }
}

#[derive(Debug)]
pub struct BufferPool {
    config: BufferPoolConfig,
    available: VecDeque<Vec<u8>>,
    acquired_count: usize,
    total_allocated: usize,
    temporary_count: usize,
}

impl BufferPool {
    /// Create a new buffer pool with the given configuration
    pub fn new(config: BufferPoolConfig) -> Self {
        let min_buffers = config.min_buffers;
        let buffer_size = config.buffer_size;

        // Pre-allocate minimum buffers
        let available: VecDeque<Vec<u8>> = (0..min_buffers)
            .map(|_| vec![0u8; buffer_size])
            .collect();

        Self {
            config,
            available,
            acquired_count: 0,
            total_allocated: min_buffers,
            temporary_count: 0,
        }
    }

    /// Create a buffer pool for 1080p BGRA32 frames (default)
    pub fn new_1080p_bgra() -> Self {
        Self::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 8,
            buffer_size: 1920 * 1080 * 4,
        })
    }

    /// Create a buffer pool for 720p BGRA32 frames
    pub fn new_720p_bgra() -> Self {
        Self::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 8,
            buffer_size: 1280 * 720 * 4,
        })
    }

    /// Create a buffer pool for 4K BGRA32 frames
    pub fn new_4k_bgra() -> Self {
        Self::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 3840 * 2160 * 4,
        })
    }

    /// Acquire a buffer from the pool
    ///
    /// Returns a buffer that can be used. If no buffer is available in the pool,
    /// a new one will be allocated (up to max_buffers limit).
    pub fn acquire(&mut self) -> Vec<u8> {
        self.acquired_count += 1;

        if let Some(mut buffer) = self.available.pop_front() {
            // Clear the buffer before returning
            buffer.clear();
            buffer.resize(self.config.buffer_size, 0);
            buffer
        } else if self.total_allocated < self.config.max_buffers {
            self.total_allocated += 1;
            vec![0u8; self.config.buffer_size]
        } else {
            // Pool is exhausted, allocate temporary buffer
            self.temporary_count += 1;
            vec![0u8; self.config.buffer_size]
        }
    }

    /// Return a buffer to the pool
    ///
    /// If the buffer size matches the pool's configured size and the pool
    /// has capacity for more pooled buffers, the buffer will be reused.
    /// Otherwise, it will be dropped.
    pub fn release(&mut self, buffer: Vec<u8>) {
        self.acquired_count = self.acquired_count.saturating_sub(1);

        // Check if this is a temporary buffer:
        // - Size matches pool size (or is smaller, indicating a resized buffer) AND
        // - We have temporary buffers out (temporary_count > 0) AND
        // - Pool is at capacity (available + total >= max_buffers)
        let is_temporary = buffer.len() <= self.config.buffer_size
            && self.temporary_count > 0
            && self.available.len() + self.total_allocated >= self.config.max_buffers;

        if is_temporary {
            // This was a temporary buffer, don't pool it
            self.temporary_count -= 1;
            return;
        }

        // Only reuse if size is <= pool size (can be resized) and we haven't exceeded total pooled buffers
        if buffer.len() <= self.config.buffer_size
            && self.available.len() < self.total_allocated
        {
            self.available.push_back(buffer);
        }
        // Otherwise, buffer is dropped
    }

    /// Acquire a buffer with a specific minimum size
    ///
    /// If the requested size is larger than the pool's buffer size,
    /// a new buffer will be allocated (not pooled).
    pub fn acquire_at_least(&mut self, min_size: usize) -> Vec<u8> {
        if min_size <= self.config.buffer_size {
            self.acquire()
        } else {
            self.acquired_count += 1;
            self.temporary_count += 1;
            vec![0u8; min_size]
        }
    }

    /// Get the number of buffers currently available in the pool
    pub fn available_count(&self) -> usize {
        self.available.len()
    }

    /// Get the number of buffers currently acquired
    pub fn acquired_count(&self) -> usize {
        self.acquired_count
    }

    /// Get the total number of buffers allocated by the pool
    pub fn total_allocated(&self) -> usize {
        self.total_allocated
    }

    /// Get the pool configuration
    pub fn config(&self) -> &BufferPoolConfig {
        &self.config
    }

    /// Shrink the pool to its minimum size
    ///
    /// Returns excess buffers to the system.
    pub fn shrink(&mut self) {
        while self.available.len() > self.config.min_buffers {
            self.available.pop_back();
            self.total_allocated = self.total_allocated.saturating_sub(1);
        }
    }

    /// Clear all buffers from the pool
    ///
    /// This returns all buffers to the system. The pool will allocate
    /// new buffers on the next acquire if needed.
    pub fn clear(&mut self) {
        self.available.clear();
        self.total_allocated = 0;
    }
}

/// A pooled buffer handle that automatically returns the buffer to the pool when dropped
#[derive(Debug)]
pub struct PooledBuffer {
    buffer: Option<Vec<u8>>,
    pool: Option<Arc<std::sync::Mutex<BufferPool>>>,
}

impl PooledBuffer {
    /// Create a new pooled buffer from a buffer pool
    pub fn from_pool(pool: Arc<std::sync::Mutex<BufferPool>>) -> Self {
        let buffer = pool.lock().unwrap().acquire();
        Self {
            buffer: Some(buffer),
            pool: Some(pool),
        }
    }

    /// Create a standalone pooled buffer (not attached to a pool)
    pub fn standalone(size: usize) -> Self {
        Self {
            buffer: Some(vec![0u8; size]),
            pool: None,
        }
    }

    /// Get a mutable reference to the buffer data
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.buffer {
            Some(v) => v.as_mut_slice(),
            None => &mut [],
        }
    }

    /// Get a reference to the buffer data
    pub fn as_slice(&self) -> &[u8] {
        match &self.buffer {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    /// Get the buffer size
    pub fn len(&self) -> usize {
        self.buffer.as_ref().map(|v| v.len()).unwrap_or(0)
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resize the buffer in place
    pub fn resize(&mut self, new_len: usize, value: u8) {
        if let Some(buffer) = &mut self.buffer {
            buffer.resize(new_len, value);
        }
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        if let Some(buffer) = &mut self.buffer {
            buffer.clear();
        }
    }

    /// Take ownership of the underlying buffer
    ///
    /// This prevents the buffer from being returned to the pool.
    pub fn take(mut self) -> Vec<u8> {
        self.buffer.take().unwrap_or_default()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let (Some(buffer), Some(pool)) = (self.buffer.take(), &self.pool) {
            pool.lock().unwrap().release(buffer);
        }
    }
}

impl AsRef<[u8]> for PooledBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsMut<[u8]> for PooledBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_pool_creates_with_min_buffers() {
        let pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 2,
            max_buffers: 4,
            buffer_size: 1024,
        });

        assert_eq!(pool.available_count(), 2);
        assert_eq!(pool.total_allocated(), 2);
    }

    #[test]
    fn buffer_pool_acquires_and_releases_buffer() {
        let mut pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        });

        let buffer = pool.acquire();
        assert_eq!(buffer.len(), 1024);
        assert_eq!(pool.acquired_count(), 1);
        assert_eq!(pool.available_count(), 0);

        pool.release(buffer);
        assert_eq!(pool.acquired_count(), 0);
        assert_eq!(pool.available_count(), 1);
    }

    #[test]
    fn buffer_pool_allocates_new_when_empty() {
        let mut pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        });

        // Acquire the initial buffer
        pool.acquire();

        // Acquire again - should allocate new
        let buffer = pool.acquire();
        assert_eq!(buffer.len(), 1024);
        assert_eq!(pool.total_allocated(), 2);
    }

    #[test]
    fn buffer_pool_limits_max_buffers() {
        let mut pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 2,
            buffer_size: 1024,
        });

        let b1 = pool.acquire();
        let b2 = pool.acquire();
        let b3 = pool.acquire(); // Exceeds max, temporary

        assert_eq!(pool.total_allocated(), 2); // Doesn't increase for temporary

        pool.release(b3);
        assert_eq!(pool.available_count(), 0); // Temporary not pooled

        pool.release(b2);
        assert_eq!(pool.available_count(), 1); // b2 is pooled

        pool.release(b1);
        assert_eq!(pool.available_count(), 2); // b1 is pooled, at max
    }

    #[test]
    fn buffer_pool_shrinks_to_min() {
        let mut pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        });

        let b1 = pool.acquire();
        let b2 = pool.acquire();
        let b3 = pool.acquire();
        let b4 = pool.acquire();

        pool.release(b1);
        pool.release(b2);
        pool.release(b3);
        pool.release(b4);

        assert_eq!(pool.available_count(), 4);
        assert_eq!(pool.total_allocated(), 4);

        pool.shrink();

        assert_eq!(pool.available_count(), 1);
        assert_eq!(pool.total_allocated(), 1);
    }

    #[test]
    fn pooled_buffer_returns_to_pool_on_drop() {
        let pool = Arc::new(std::sync::Mutex::new(BufferPool::new(BufferPoolConfig {
            min_buffers: 0,
            max_buffers: 4,
            buffer_size: 1024,
        })));

        {
            let _buffer = PooledBuffer::from_pool(pool.clone());
            assert_eq!(pool.lock().unwrap().acquired_count(), 1);
            assert_eq!(pool.lock().unwrap().available_count(), 0);
        }

        assert_eq!(pool.lock().unwrap().acquired_count(), 0);
        assert_eq!(pool.lock().unwrap().available_count(), 1);
    }

    #[test]
    fn pooled_buffer_take_prevents_return() {
        let pool = Arc::new(std::sync::Mutex::new(BufferPool::new(BufferPoolConfig {
            min_buffers: 0,
            max_buffers: 4,
            buffer_size: 1024,
        })));

        let buffer = PooledBuffer::from_pool(pool.clone());
        let data = buffer.take();

        assert_eq!(data.len(), 1024);
        assert_eq!(pool.lock().unwrap().available_count(), 0);
    }

    #[test]
    fn buffer_pool_1080p_has_correct_size() {
        let mut pool = BufferPool::new_1080p_bgra();
        let buffer = pool.acquire();

        assert_eq!(buffer.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn buffer_pool_720p_has_correct_size() {
        let mut pool = BufferPool::new_720p_bgra();
        let buffer = pool.acquire();

        assert_eq!(buffer.len(), 1280 * 720 * 4);
    }

    #[test]
    fn buffer_pool_4k_has_correct_size() {
        let mut pool = BufferPool::new_4k_bgra();
        let buffer = pool.acquire();

        assert_eq!(buffer.len(), 3840 * 2160 * 4);
    }

    #[test]
    fn buffer_pool_clear_removes_all_buffers() {
        let mut pool = BufferPool::new(BufferPoolConfig {
            min_buffers: 2,
            max_buffers: 4,
            buffer_size: 1024,
        });

        assert_eq!(pool.available_count(), 2);

        pool.clear();

        assert_eq!(pool.available_count(), 0);
        assert_eq!(pool.total_allocated(), 0);
    }

    #[test]
    fn buffer_pool_acquire_at_least_reuses_for_smaller() {
        let mut pool = BufferPool::new_1080p_bgra();

        let buffer = pool.acquire_at_least(1024);
        assert_eq!(buffer.len(), 1920 * 1080 * 4);
        assert_eq!(pool.total_allocated(), 1);
    }

    #[test]
    fn buffer_pool_acquire_at_least_allocates_for_larger() {
        let mut pool = BufferPool::new_720p_bgra();

        let buffer = pool.acquire_at_least(1920 * 1080 * 4);
        assert_eq!(buffer.len(), 1920 * 1080 * 4);
        // Larger buffers are not counted in total_allocated (only min_buffers are pre-allocated)
        assert_eq!(pool.total_allocated(), 1);
    }

    #[test]
    fn pooled_buffer_as_ref_works() {
        let pool = Arc::new(std::sync::Mutex::new(BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        })));

        let mut buffer = PooledBuffer::from_pool(pool.clone());
        buffer.as_mut_slice()[0] = 42;

        assert_eq!(buffer.as_slice()[0], 42);
    }

    #[test]
    fn pooled_buffer_resize_works() {
        let pool = Arc::new(std::sync::Mutex::new(BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        })));

        let mut buffer = PooledBuffer::from_pool(pool);
        // First shrink the buffer
        buffer.resize(512, 99);
        assert_eq!(buffer.len(), 512);

        // Then grow it - new elements should be filled with 99
        buffer.resize(1024, 99);
        assert_eq!(buffer.len(), 1024);
        // Elements 512-1023 should be 99 (the new elements)
        assert_eq!(buffer.as_slice()[512], 99);
        assert_eq!(buffer.as_slice()[1023], 99);
    }

    #[test]
    fn pooled_buffer_clear_works() {
        let pool = Arc::new(std::sync::Mutex::new(BufferPool::new(BufferPoolConfig {
            min_buffers: 1,
            max_buffers: 4,
            buffer_size: 1024,
        })));

        let mut buffer = PooledBuffer::from_pool(pool);
        buffer.as_mut_slice()[0] = 42;
        buffer.clear();

        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }
}
