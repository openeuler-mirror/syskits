/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::collections::VecDeque;

/// A fixed-size ring buffer backed by a `VecDeque`.
///
/// If the ring buffer is not full, then calling the [`push_back`]
/// method appends elements, as in a [`VecDeque`]. If the ring buffer
/// is full, then calling [`push_back`] removes the element at the
/// front of the buffer (in a first-in, first-out manner) before
/// appending the new element to the back of the buffer.
///
/// Use [`from_iter`] to take the last `size` elements from an
/// iterator.
///
/// # Examples
///
/// After exceeding the size limit, the oldest elements are dropped in
/// favor of the newest element:
///
/// ```rust,ignore
/// let mut buffer: RingBuffer<u8> = RingBuffer::new(2);
/// buffer.push_back(0);
/// buffer.push_back(1);
/// buffer.push_back(2);
/// assert_eq!(vec![1, 2], buffer.data);
/// ```
///
/// Take the last `n` elements from an iterator:
///
/// ```rust,ignore
/// let iter = [0, 1, 2].iter();
/// let actual = RingBuffer::from_iter(iter, 2).data;
/// let expected = VecDeque::from_iter([1, 2].iter());
/// assert_eq!(expected, actual);
/// ```
///
/// [`push_back`]: struct.RingBuffer.html#method.push_back
/// [`from_iter`]: struct.RingBuffer.html#method.from_iter
///
pub struct CtRingBuffer<T> {
    pub data: VecDeque<T>,
    size: usize,
}

impl<T> CtRingBuffer<T> {
    pub fn new(size: usize) -> Self {
        Self {
            data: VecDeque::new(),
            size,
        }
    }

    pub fn from_iter(iter: impl Iterator<Item = T>, size: usize) -> Self {
        let mut ring_buffer = Self::new(size);
        for value in iter {
            ring_buffer.push_back(value);
        }
        ring_buffer
    }

    /// Append a value to the end of the ring buffer.
    ///
    /// If the ring buffer is not full, this method return [`None`]. If
    /// the ring buffer is full, appending a new element will cause the
    /// oldest element to be evicted. In that case this method returns
    /// that element, or `None`.
    ///
    /// In the special case where the size limit is zero, each call to
    /// this method with input `value` returns `Some(value)`, because
    /// the input is immediately evicted.
    ///
    /// # Examples
    ///
    /// Appending an element when the buffer is full returns the oldest
    /// element:
    ///
    /// ```rust,ignore
    /// let mut buf = RingBuffer::new(3);
    /// assert_eq!(None, buf.push_back(0));
    /// assert_eq!(None, buf.push_back(1));
    /// assert_eq!(None, buf.push_back(2));
    /// assert_eq!(Some(0), buf.push_back(3));
    /// ```
    ///
    /// If the size limit is zero, then this method always returns the
    /// input value:
    ///
    /// ```rust,ignore
    /// let mut buf = RingBuffer::new(0);
    /// assert_eq!(Some(0), buf.push_back(0));
    /// assert_eq!(Some(1), buf.push_back(1));
    /// assert_eq!(Some(2), buf.push_back(2));
    /// ```
    pub fn push_back(&mut self, value: T) -> Option<T> {
        if self.size == 0 {
            return Some(value);
        }
        let result = if self.size <= self.data.len() {
            self.data.pop_front()
        } else {
            None
        };
        self.data.push_back(value);
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_ringbuffer::CtRingBuffer;
    use std::collections::VecDeque;

    #[test]
    fn test_size_limit_zero() {
        let mut buf = CtRingBuffer::new(0);
        assert_eq!(Some(0), buf.push_back(0));
        assert_eq!(Some(1), buf.push_back(1));
        assert_eq!(Some(2), buf.push_back(2));
    }

    #[test]
    fn test_evict_oldest() {
        let mut buf = CtRingBuffer::new(2);
        assert_eq!(None, buf.push_back(0));
        assert_eq!(None, buf.push_back(1));
        assert_eq!(Some(0), buf.push_back(2));
    }
    #[test]
    fn test_from_iter() {
        let iter = [0, 1, 2, 3, 4].iter();
        let actual = CtRingBuffer::from_iter(iter, 2).data;
        let expected: VecDeque<&i32> = [3, 4].iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_size_limit_zero_behavior() {
        let mut buf = CtRingBuffer::new(0);
        assert_eq!(buf.push_back(0), Some(0));
        assert_eq!(buf.push_back(1), Some(1));
        assert_eq!(buf.push_back(2), Some(2));
        assert!(buf.data.is_empty());
    }

    #[test]
    fn test_push_back_into_empty_buffer() {
        let mut buf = CtRingBuffer::new(3);
        assert_eq!(buf.push_back(0), None);
        assert_eq!(buf.data, vec![0]);
    }

    #[test]
    fn test_push_back_until_full() {
        let mut buf = CtRingBuffer::new(3);
        assert_eq!(buf.push_back(0), None);
        assert_eq!(buf.push_back(1), None);
        assert_eq!(buf.push_back(2), None);
        assert_eq!(buf.data, vec![0, 1, 2]);
    }

    #[test]
    fn test_evict_oldest_element() {
        let mut buf = CtRingBuffer::new(2);
        buf.push_back(0);
        buf.push_back(1);
        assert_eq!(buf.push_back(2), Some(0));
        assert_eq!(buf.data, vec![1, 2]);
    }

    #[test]
    fn test_push_back_does_not_evict_when_not_full() {
        let mut buf = CtRingBuffer::new(3);
        buf.push_back(0);
        buf.push_back(1);
        buf.push_back(2);
        assert_eq!(buf.push_back(3), Some(0));
        assert_eq!(buf.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_from_iter_smaller_than_size_limit() {
        let iter = [0, 1].iter();
        let actual = CtRingBuffer::from_iter(iter, 3).data;
        let expected: VecDeque<&i32> = [0, 1].iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_iter_larger_than_size_limit() {
        let iter = [0, 1, 2, 3, 4, 5].iter();
        let actual = CtRingBuffer::from_iter(iter, 3).data;
        let expected: VecDeque<&i32> = [3, 4, 5].iter().collect();
        assert_eq!(actual, expected);
    }
}
