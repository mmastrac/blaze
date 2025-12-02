use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use crate::server::WakerHandle;

pub struct SyncRingBuffer<const SIZE: usize, T: Copy> {
    buffer: [T; SIZE],
    write_index: usize,
    read_index: usize,
}

impl<const SIZE: usize, T: Copy + Default> Default for SyncRingBuffer<SIZE, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize, T: Copy> SyncRingBuffer<SIZE, T> {
    pub fn new() -> Self
    where
        T: Default,
    {
        Self {
            buffer: [T::default(); SIZE],
            write_index: 0,
            read_index: 0,
        }
    }

    pub fn push(&mut self, b: T) {
        assert!(!self.is_full());
        self.buffer[self.write_index] = b;
        self.write_index = (self.write_index + 1) % SIZE;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.read_index == self.write_index {
            return None;
        }
        let b = self.buffer[self.read_index];
        self.read_index = (self.read_index + 1) % SIZE;
        Some(b)
    }

    pub fn clear(&mut self) {
        self.read_index = self.write_index;
    }

    pub fn is_empty(&self) -> bool {
        self.read_index == self.write_index
    }

    pub fn is_full(&self) -> bool {
        (self.write_index + 1) % SIZE == self.read_index
    }

    pub fn len(&self) -> usize {
        if self.write_index >= self.read_index {
            self.write_index - self.read_index
        } else {
            SIZE - self.read_index + self.write_index
        }
    }

    pub fn copy_into_slice<'buf>(&self, buf: &'buf mut [T; SIZE]) -> &'buf [T] {
        let len = self.len();
        if len == 0 {
            return &buf[..0];
        }

        // Check if data wraps around the buffer
        if self.read_index > self.write_index {
            // Data wraps: copy from read_index to end, then from start to write_index
            let first_part_len = SIZE - self.read_index;
            buf[..first_part_len].copy_from_slice(&self.buffer[self.read_index..SIZE]);
            buf[first_part_len..len].copy_from_slice(&self.buffer[0..self.write_index]);
        } else {
            // Data is contiguous: copy directly
            buf[..len].copy_from_slice(&self.buffer[self.read_index..self.read_index + len]);
        }
        &buf[..len]
    }

    pub fn replace_with_slice(&mut self, op_buf: &[T]) {
        assert!(op_buf.len() <= SIZE);
        self.read_index = 0;
        self.write_index = op_buf.len();
        self.buffer[0..op_buf.len()].copy_from_slice(op_buf);
    }
}

#[derive(Clone, Default)]
pub struct SyncRingBufferHandle<const SIZE: usize, T: Copy + Default>(
    pub Arc<Mutex<SyncRingBuffer<SIZE, T>>>,
);

impl<const SIZE: usize, T: Copy + Default> SyncRingBufferHandle<SIZE, T> {
    pub fn push(&self, b: T) {
        self.0.lock().unwrap().push(b);
    }

    pub fn pop(&self) -> Option<T> {
        self.0.lock().unwrap().pop()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.0.lock().unwrap().is_full()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn copy_into_slice<'buf>(&self, buf: &'buf mut [T; SIZE]) -> &'buf [T] {
        self.0.lock().unwrap().copy_into_slice(buf)
    }
}

struct RingBuffer<const SIZE: usize, T: Copy + Default> {
    buffer: [T; SIZE],
    write_waker: Arc<WakerHandle>,
    read_waker: Arc<WakerHandle>,
    write_index: usize,
    read_index: usize,
}

impl<const SIZE: usize, T: Copy + Default> Default for RingBuffer<SIZE, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize, T: Copy + Default> RingBuffer<SIZE, T> {
    pub fn new() -> Self {
        Self {
            buffer: [T::default(); SIZE],
            write_waker: Arc::new(WakerHandle::new()),
            read_waker: Arc::new(WakerHandle::new()),
            write_index: 0,
            read_index: 0,
        }
    }

    /// Check if the buffer is full
    fn is_full(&self) -> bool {
        (self.write_index + 1) % SIZE == self.read_index
    }

    /// Check if the buffer is empty
    fn is_empty(&self) -> bool {
        self.read_index == self.write_index
    }

    pub fn clear(&mut self) {
        self.read_index = self.write_index;
        self.write_waker.maybe_wake();
        self.read_waker.maybe_wake();
    }

    pub fn len(&self) -> usize {
        if self.write_index >= self.read_index {
            self.write_index - self.read_index
        } else {
            SIZE - self.read_index + self.write_index
        }
    }

    pub async fn push(this: &Arc<Mutex<Self>>, b: T) {
        // Wait if buffer is full
        loop {
            let waker = {
                let mut lock = this.lock().unwrap();
                if !lock.is_full() {
                    let write_index = lock.write_index;
                    lock.buffer[write_index] = b;
                    lock.write_index = (lock.write_index + 1) % lock.buffer.len();

                    // Wake any waiting readers
                    lock.read_waker.maybe_wake();
                    return;
                }
                lock.write_waker.clone()
            };
            waker.wait().await;
        }
    }

    pub async fn pop(this: &Arc<Mutex<Self>>) -> T {
        // Wait if buffer is empty
        loop {
            let waker = {
                let mut lock = this.lock().unwrap();
                if !lock.is_empty() {
                    // Pop the byte
                    let b = lock.buffer[lock.read_index];
                    lock.read_index = (lock.read_index + 1) % SIZE;

                    // Wake any waiting writers
                    lock.write_waker.maybe_wake();

                    return b;
                }
                lock.read_waker.clone()
            };
            waker.wait().await;
        }
    }

    pub fn pop_sync(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        let b = self.buffer[self.read_index];
        self.read_index = (self.read_index + 1) % SIZE;
        self.write_waker.maybe_wake();
        Some(b)
    }

    pub fn push_sync(&mut self, b: T) {
        if self.is_full() {
            return;
        }
        self.buffer[self.write_index] = b;
        self.write_index = (self.write_index + 1) % SIZE;
        self.read_waker.maybe_wake();
    }

    pub fn free(&self) -> usize {
        SIZE - self.len()
    }

    pub fn register(&self, cx: &mut Context<'_>) {
        self.read_waker.register(cx)
    }
}

#[derive(Clone, Default)]
pub struct RingBufferHandle<const SIZE: usize, T: Copy + Default> {
    buffer: Arc<Mutex<RingBuffer<SIZE, T>>>,
}

impl<const SIZE: usize, T: Copy + Default> RingBufferHandle<SIZE, T> {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(RingBuffer::new())),
        }
    }

    pub async fn push(&self, b: T) {
        RingBuffer::push(&self.buffer, b).await;
    }

    pub async fn pop(&self) -> T {
        RingBuffer::pop(&self.buffer).await
    }

    pub fn len(&self) -> usize {
        let lock = self.buffer.lock().unwrap();
        lock.len()
    }

    pub fn is_empty(&self) -> bool {
        let lock = self.buffer.lock().unwrap();
        lock.is_empty()
    }

    pub fn pop_sync(&self) -> Option<T> {
        let mut lock = self.buffer.lock().unwrap();
        lock.pop_sync()
    }

    pub fn push_sync(&self, b: T) {
        let mut lock = self.buffer.lock().unwrap();
        lock.push_sync(b);
    }

    pub fn free(&self) -> usize {
        let lock = self.buffer.lock().unwrap();
        lock.free()
    }

    pub fn register(&self, cx: &mut Context<'_>) {
        let lock = self.buffer.lock().unwrap();
        lock.register(cx)
    }

    pub fn clear(&self) {
        let mut lock = self.buffer.lock().unwrap();
        lock.clear()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_ring_buffer_one() {
        let buffer = RingBufferHandle::<16, u8>::new();

        let a = {
            let buffer = buffer.clone();
            tokio::spawn(async move {
                _ = buffer.pop().await;
            })
        };

        let b = tokio::spawn(async move {
            buffer.push(1).await;
        });

        a.await.unwrap();
        b.await.unwrap();
    }

    #[tokio::test]
    async fn test_ring_buffer_faster_reader() {
        let buffer = RingBufferHandle::<16, u8>::new();

        let a = {
            let buffer = buffer.clone();
            tokio::spawn(async move {
                for _ in 0..32 {
                    _ = buffer.pop().await;
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
        };

        let b = tokio::spawn(async move {
            for _ in 0..4 {
                buffer.push(1).await;
            }

            for _ in 0..28 {
                buffer.push(1).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        a.await.unwrap();
        b.await.unwrap();
    }

    #[tokio::test]
    async fn test_ring_buffer_faster_writer() {
        let buffer = RingBufferHandle::<16, u8>::new();

        let a = {
            let buffer = buffer.clone();
            tokio::spawn(async move {
                for _ in 0..128 {
                    _ = buffer.pop().await;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
        };

        let b = tokio::spawn(async move {
            for _ in 0..128 {
                buffer.push(1).await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        a.await.unwrap();
        b.await.unwrap();
    }
}
