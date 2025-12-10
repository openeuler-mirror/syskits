/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
//! Implementations of digest functions, like md5 and sha1.
//!
//! The [`Digest`] trait represents the interface for providing inputs
//! to these digest functions and accessing the resulting hash. The
//! [`DigestWriter`] struct provides a wrapper around [`Digest`] that
//! implements the [`Write`] trait, for use in situations where calling
//! [`write`] would be useful.
use std::io::Write;

use hex::encode;
#[cfg(windows)]
use memchr::memmem;

pub trait Digest {
    fn new() -> Self
    where
        Self: Sized;
    fn hash_update(&mut self, input: &[u8]);
    fn hash_finalize(&mut self, out: &mut [u8]);
    fn reset(&mut self);
    fn output_bits(&self) -> usize;
    fn output_bytes(&self) -> usize {
        (self.output_bits() + 7) / 8
    }
    fn result_str(&mut self) -> String {
        let mut buf: Vec<u8> = vec![0; self.output_bytes()];
        self.hash_finalize(&mut buf);
        encode(buf)
    }
}

/// first element of the tuple is the blake2b state
/// second is the number of output bits
pub struct Blake2b(blake2b_simd::State, usize);

// 定义最小和最大输出字节长度
const MIN_OUTPUT_BYTES: usize = 1;
const MAX_OUTPUT_BYTES: usize = 64;

impl Blake2b {
    /// Return a new Blake2b instance with a custom output bytes length.
    ///
    /// Panics if `output_bytes` is outside the allowed range of `[MIN_OUTPUT_BYTES, MAX_OUTPUT_BYTES]`.
    pub fn with_output_bytes(output_bytes: usize) -> Self {
        assert!(
            (MIN_OUTPUT_BYTES..=MAX_OUTPUT_BYTES).contains(&output_bytes),
            "Invalid output bytes length"
        );

        let mut params = blake2b_simd::Params::new();
        params.hash_length(output_bytes);

        let state = params.to_state();
        Self(state, output_bytes * 8)
    }
}

impl Digest for Blake2b {
    fn new() -> Self {
        // by default, Blake2b output is 512 bits long (= 64B)
        Self::with_output_bytes(64)
    }

    fn hash_update(&mut self, input: &[u8]) {
        self.0.update(input);
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        let hash_result = &self.0.finalize();
        out.copy_from_slice(hash_result.as_bytes());
    }

    fn reset(&mut self) {
        *self = Self::with_output_bytes(self.output_bytes());
    }

    fn output_bits(&self) -> usize {
        self.1
    }
}

pub struct Blake3(blake3::Hasher);
impl Digest for Blake3 {
    fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    fn hash_update(&mut self, input: &[u8]) {
        self.0.update(input);
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        let hash_result = &self.0.finalize();
        out.copy_from_slice(hash_result.as_bytes());
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn output_bits(&self) -> usize {
        256
    }
}

pub struct Sm3(sm3::Sm3);
impl Digest for Sm3 {
    fn new() -> Self {
        Self(<sm3::Sm3 as sm3::Digest>::new())
    }

    fn hash_update(&mut self, input: &[u8]) {
        <sm3::Sm3 as sm3::Digest>::update(&mut self.0, input);
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        out.copy_from_slice(&<sm3::Sm3 as sm3::Digest>::finalize(self.0.clone()));
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn output_bits(&self) -> usize {
        256
    }
}

// NOTE: CRC_TABLE_LEN *must* be <= 256 as we cast 0..CRC_TABLE_LEN to u8
const CRC_TABLE_LEN: usize = 256;

pub struct CRC {
    state: u32,
    size: usize,
    crc_table: [u32; CRC_TABLE_LEN],
}
impl CRC {
    fn generate_crc_table() -> [u32; CRC_TABLE_LEN] {
        let mut table = [0; CRC_TABLE_LEN];

        for (i, elt) in table.iter_mut().enumerate().take(CRC_TABLE_LEN) {
            *elt = Self::crc_entry(i as u8);
        }

        table
    }

    fn crc_entry(input: u8) -> u32 {
        let crc = (input as u32) << 24;

        #[allow(clippy::identity_op)] // Suppress Clippy warning about unnecessary bit shifting
        fn inner(crc: u32, remaining_iterations: u8) -> u32 {
            match remaining_iterations {
                0 => crc,
                _ => {
                    let if_condition = crc & 0x8000_0000;
                    let if_body = (crc << 1) ^ 0x04c1_1db7;
                    let else_body = crc << 1;

                    // Emulate if statement using a lookup table
                    let condition_table = [else_body, if_body];
                    inner(
                        condition_table[(if_condition != 0) as usize],
                        remaining_iterations - 1,
                    )
                }
            }
        }

        inner(crc, 8)
    }

    fn update(&mut self, input: u8) {
        self.state = (self.state << 8)
            ^ self.crc_table[((self.state >> 24) as usize ^ input as usize) & 0xFF];
    }
}

impl Digest for CRC {
    fn new() -> Self {
        Self {
            state: 0,
            size: 0,
            crc_table: Self::generate_crc_table(),
        }
    }

    fn hash_update(&mut self, input: &[u8]) {
        for &elt in input {
            self.update(elt);
        }
        self.size += input.len();
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        let mut sz = self.size;
        while sz != 0 {
            self.update(sz as u8);
            sz >>= 8;
        }
        self.state = !self.state;
        out.copy_from_slice(&self.state.to_ne_bytes());
    }

    fn result_str(&mut self) -> String {
        let mut _out: Vec<u8> = vec![0; 4];
        self.hash_finalize(&mut _out);
        format!("{}", self.state)
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn output_bits(&self) -> usize {
        256
    }
}

// This can be replaced with usize::div_ceil once it is stabilized.
// This implementation approach is optimized for when `b` is a constant,
// particularly a power of two.
pub fn div_ceil(a: usize, b: usize) -> usize {
    (a + b - 1) / b
}

pub struct BSD {
    state: u16,
}
impl Digest for BSD {
    fn new() -> Self {
        Self { state: 0 }
    }

    fn hash_update(&mut self, input: &[u8]) {
        for &byte in input {
            self.state = (self.state >> 1) + ((self.state & 1) << 15);
            self.state = self.state.wrapping_add(u16::from(byte));
        }
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        out.copy_from_slice(&self.state.to_ne_bytes());
    }

    fn result_str(&mut self) -> String {
        let mut _out: Vec<u8> = vec![0; 2];
        self.hash_finalize(&mut _out);
        format!("{}", self.state)
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn output_bits(&self) -> usize {
        128
    }
}

pub struct SYSV {
    state: u32,
}
impl Digest for SYSV {
    fn new() -> Self {
        Self { state: 0 }
    }

    fn hash_update(&mut self, input: &[u8]) {
        for &byte in input {
            self.state = self.state.wrapping_add(u32::from(byte));
        }
    }

    fn hash_finalize(&mut self, out: &mut [u8]) {
        self.state = (self.state & 0xffff) + (self.state >> 16);
        self.state = (self.state & 0xffff) + (self.state >> 16);
        out.copy_from_slice(&(self.state as u16).to_ne_bytes());
    }

    fn result_str(&mut self) -> String {
        let mut _out: Vec<u8> = vec![0; 2];
        self.hash_finalize(&mut _out);
        format!("{}", self.state)
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn output_bits(&self) -> usize {
        512
    }
}

// Implements the Digest trait for sha2 / sha3 algorithms with fixed output
macro_rules! impl_digest_common {
    ($algo_type: ty, $size: expr) => {
        impl Digest for $algo_type {
            fn new() -> Self {
                Self(Default::default())
            }

            fn hash_update(&mut self, input: &[u8]) {
                digest::Digest::update(&mut self.0, input);
            }

            fn hash_finalize(&mut self, out: &mut [u8]) {
                digest::Digest::finalize_into_reset(&mut self.0, out.into());
            }

            fn reset(&mut self) {
                *self = Self::new();
            }

            fn output_bits(&self) -> usize {
                $size
            }
        }
    };
}

// Implements the Digest trait for sha2 / sha3 algorithms with variable output
macro_rules! impl_digest_shake {
    ($algo_type: ty) => {
        impl Digest for $algo_type {
            fn new() -> Self {
                Self(Default::default())
            }

            fn hash_update(&mut self, input: &[u8]) {
                digest::Update::update(&mut self.0, input);
            }

            fn hash_finalize(&mut self, out: &mut [u8]) {
                digest::ExtendableOutputReset::finalize_xof_reset_into(&mut self.0, out);
            }

            fn reset(&mut self) {
                *self = Self::new();
            }

            fn output_bits(&self) -> usize {
                0
            }
        }
    };
}

pub struct Md5(md5::Md5);
pub struct Sha1(sha1::Sha1);
pub struct Sha224(sha2::Sha224);
pub struct Sha256(sha2::Sha256);
pub struct Sha384(sha2::Sha384);
pub struct Sha512(sha2::Sha512);
impl_digest_common!(Md5, 128);
impl_digest_common!(Sha1, 160);
impl_digest_common!(Sha224, 224);
impl_digest_common!(Sha256, 256);
impl_digest_common!(Sha384, 384);
impl_digest_common!(Sha512, 512);

pub struct Sha3_224(sha3::Sha3_224);
pub struct Sha3_256(sha3::Sha3_256);
pub struct Sha3_384(sha3::Sha3_384);
pub struct Sha3_512(sha3::Sha3_512);
impl_digest_common!(Sha3_224, 224);
impl_digest_common!(Sha3_256, 256);
impl_digest_common!(Sha3_384, 384);
impl_digest_common!(Sha3_512, 512);

pub struct Shake128(sha3::Shake128);
#[derive(Debug)]
pub struct Shake256(sha3::Shake256);
impl_digest_shake!(Shake128);
impl_digest_shake!(Shake256);

/// A struct that writes to a digest.
///
/// This struct wraps a [`Digest`] and provides a [`Write`]
/// implementation that passes input bytes directly to the
/// [`Digest::hash_update`].
///
/// On Windows, if `binary` is `false`, then the [`write`]
/// implementation replaces instances of "\r\n" with "\n" before passing
/// the input bytes to the [`digest`].
pub struct DigestWriter<'a> {
    digest: &'a mut Box<dyn Digest>,

    /// Whether to write to the digest in binary mode or text mode on Windows.
    ///
    /// If this is `false`, then instances of "\r\n" are replaced with
    /// "\n" before passing input bytes to the [`digest`].
    #[allow(dead_code)]
    binary: bool,

    /// Whether the previous
    #[allow(dead_code)]
    was_last_character_carriage_return: bool,
    // TODO These are dead code only on non-Windows operating systems.
    // It might be better to use a `#[cfg(windows)]` guard here.
}

impl<'a> DigestWriter<'a> {
    pub fn new(digest: &'a mut Box<dyn Digest>, binary: bool) -> DigestWriter {
        let was_last_character_carriage_return = false;
        DigestWriter {
            digest,
            binary,
            was_last_character_carriage_return,
        }
    }

    pub fn finalize(&mut self) -> bool {
        if self.was_last_character_carriage_return {
            self.digest.hash_update(&[b'\r']);
            true
        } else {
            false
        }
    }
}

impl<'a> Write for DigestWriter<'a> {
    #[cfg(not(windows))]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.digest.hash_update(buf);
        Ok(buf.len())
    }

    #[cfg(likelinux)]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.binary {
            self.digest.hash_update(buf);
            return Ok(buf.len());
        }

        // The remaining code handles Windows text mode, where we must
        // replace each occurrence of "\r\n" with "\n".
        //
        // First, if the last character written was "\r" and the first
        // character in the current buffer to write is not "\n", then we
        // need to write the "\r" that we buffered from the previous
        // call to `write()`.
        let n = buf.len();
        if self.was_last_character_carriage_return && n > 0 && buf[0] != b'\n' {
            self.digest.hash_update(&[b'\r']);
        }

        // Next, find all occurrences of "\r\n", inputting the slice
        // just before the "\n" in the previous instance of "\r\n" and
        // the beginning of this "\r\n".
        let mut i_prev = 0;
        for i in memmem::find_iter(buf, b"\r\n") {
            self.digest.hash_update(&buf[i_prev..i]);
            i_prev = i + 1;
        }

        // Finally, check whether the last character is "\r". If so,
        // buffer it until we know that the next character is not "\n",
        // which can only be known on the next call to `write()`.
        //
        // This all assumes that `write()` will be called on adjacent
        // blocks of the input.
        if n > 0 && buf[n - 1] == b'\r' {
            self.was_last_character_carriage_return = true;
            self.digest.hash_update(&buf[i_prev..n - 1]);
        } else {
            self.was_last_character_carriage_return = false;
            self.digest.hash_update(&buf[i_prev..n]);
        }

        // Even though we dropped a "\r" for each "\r\n" we found, we
        // still report the number of bytes written as `n`. This is
        // because the meaning of the returned number is supposed to be
        // the number of bytes consumed by the writer, so that if the
        // calling code were calling `write()` in a loop, it would know
        // where the next contiguous slice of the buffer starts.
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
     use super::*;

    #[test]
    fn test_blake2b_with_custom_output_bytes() {
        let mut digest = Blake2b::with_output_bytes(32);
        let input = b"test input";
        digest.hash_update(input);
        let result = digest.result_str();
        // 这里假设我们有一个已知的哈希值，用于比较
        assert_eq!(
            result,
            "14f9a1375564fd6e1753125374374be21fb72319d989381a443e5a11de6687e4"
        );
    }

    #[test]
    fn test_blake3_default_behavior() {
        let mut digest = Blake3::new();
        let input = b"test input";
        digest.hash_update(input);
        let result = digest.result_str();
        assert_eq!(
            result,
            "aa4909e14f1389afc428e481ea20ffd9673604711f5afb60a747fec57e4c267c"
        );
    }

    #[test]
    fn test_sm3_digest() {
        let mut digest = Sm3::new();
        let input = b"test input";
        digest.hash_update(input);
        let mut result_vec = vec![0; digest.output_bytes()];

        digest.hash_finalize(&mut result_vec);

        let result = encode(result_vec);
        assert_eq!(
            result,
            "9f38f758d5002744886d47b296194606d1cac672c95ca3a0c89dfa4134007b9b"
        );
    }

    #[test]
    fn test_crc_update() {
        let mut crc = CRC::new();
        crc.update(0x01);
        crc.update(0x02);
        crc.update(0x03);
        assert_eq!(crc.state, 2892567633);
        assert_eq!(crc.size, 0);
    }

    #[test]
    fn test_crc_hash_update() {
        let mut crc = CRC::new();
        crc.hash_update(&[0x01, 0x02, 0x03]);
        assert_eq!(crc.state, 2892567633);
        assert_eq!(crc.size, 3);
    }

    #[test]
    fn test_crc_hash_finalize() {
        let mut crc = CRC::new();
        crc.hash_update(&[0x01, 0x02, 0x03]);
        let mut output = [0u8; 4];
        crc.hash_finalize(&mut output);
        assert_eq!(output, [76, 69, 139, 95]);
    }

    #[test]
    fn test_crc_result_str() {
        let mut crc = CRC::new();
        crc.hash_update(&[0x01, 0x02, 0x03]);
        let result = crc.result_str();
        assert_eq!(result, "1602962764");
    }

    #[test]
    fn test_crc_reset() {
        let mut crc = CRC::new();
        crc.hash_update(&[0x01, 0x02, 0x03]);
        crc.reset();
        assert_eq!(crc.state, 0);
        assert_eq!(crc.size, 0);
    }

    #[test]
    fn test_crc_output_bits() {
        let crc = CRC::new();
        assert_eq!(crc.output_bits(), 256);
    }

    /// Test for replacing a "\r\n" sequence with "\n" when the "\r" is
    /// at the end of one block and the "\n" is at the beginning of the
    /// next block, when reading in blocks.
    #[cfg(likelinux)]
    #[test]
    fn test_crlf_across_blocks() {
        use std::io::Write;

        use super::Digest;
        use super::DigestWriter;
        use super::Md5;

        // Writing "\r" in one call to `write()`, and then "\n" in another.
        let mut digest = Box::new(Md5::new()) as Box<dyn Digest>;
        let mut writer_crlf = DigestWriter::new(&mut digest, false);
        writer_crlf.write_all(&[b'\r']).unwrap();
        writer_crlf.write_all(&[b'\n']).unwrap();
        writer_crlf.finalize();
        let result_crlf = digest.result_str();

        // We expect "\r\n" to be replaced with "\n" in text mode on Windows.
        let mut digest = Box::new(Md5::new()) as Box<dyn Digest>;
        let mut writer_lf = DigestWriter::new(&mut digest, false);
        writer_lf.write_all(&[b'\n']).unwrap();
        writer_lf.finalize();
        let result_lf = digest.result_str();

        assert_eq!(result_crlf, result_lf);
    }

    #[test]
    fn test_blake3() {
        let mut blake3 = Blake3::new();
        blake3.hash_update(b"hello");
        let mut output = [0u8; 32];
        blake3.hash_finalize(&mut output);

        // Replace the expected hash value with the actual expected hash value
        let expected_hash = [
            234, 143, 22, 61, 179, 134, 130, 146, 94, 68, 145, 197, 229, 141, 75, 179, 80, 110,
            248, 193, 78, 183, 138, 134, 233, 8, 197, 98, 74, 103, 32, 15,
        ];
        println!("{:?}", output);
        assert_eq!(output, expected_hash);

        // Test reset
        blake3.reset();
        let mut output = [0u8; 32];
        blake3.hash_finalize(&mut output);
        let expected_hash = [
            175, 19, 73, 185, 245, 249, 161, 166, 160, 64, 77, 234, 54, 220, 201, 73, 155, 203, 37,
            201, 173, 193, 18, 183, 204, 154, 147, 202, 228, 31, 50, 98,
        ];
        println!("{:?}", output);
        assert_eq!(output, expected_hash);
    }

    #[test]
    fn test_sm3() {
        let mut sm3 = Sm3::new();
        sm3.hash_update(b"hello");
        let mut output = [0u8; 32];
        sm3.hash_finalize(&mut output);

        // Replace the expected hash value with the actual expected hash value
        let expected_hash = [
            190, 203, 191, 170, 230, 84, 139, 139, 240, 207, 202, 213, 162, 113, 131, 205, 27, 230,
            9, 59, 28, 206, 204, 195, 3, 217, 198, 29, 10, 100, 82, 104,
        ];

        println!("{:?}", output);
        assert_eq!(output, expected_hash);

        // Test reset
        sm3.reset();
        let mut output = [0u8; 32];
        sm3.hash_finalize(&mut output);
        let expected_hash = [
            26, 178, 29, 131, 85, 207, 161, 127, 142, 97, 25, 72, 49, 232, 26, 143, 34, 190, 200,
            199, 40, 254, 251, 116, 126, 208, 53, 235, 80, 130, 170, 43,
        ];
        println!("{:?}", output);
        assert_eq!(output, expected_hash);
    }

    #[test]
    fn test_blake2b_with_output_bytes() {
        let output_bytes = 32; // custom output bytes length
        let blake2b = Blake2b::with_output_bytes(output_bytes);

        // Assert that the output bytes length is correct
        assert_eq!(blake2b.output_bits() / 8, output_bytes);
    }

    #[test]
    fn test_blake2b_new() {
        let blake2b = Blake2b::new();

        // Assert that the default output bytes length is 64 bits
        assert_eq!(blake2b.output_bits() / 8, 64);
    }

    #[test]
    fn test_blake2b_hash_update() {
        let mut blake2b = Blake2b::new();
        let input = b"Hello, world!";

        // Update the hash with the input
        blake2b.hash_update(input);

        // Finalize the hash and store it in `output`
        let mut output = [0u8; 64];
        blake2b.hash_finalize(&mut output);

        // Assert that the output is not all zeros (since we updated the hash with input)
        assert_ne!(output, [0u8; 64]);
    }

    #[test]
    fn test_blake2b_hash_finalize() {
        let mut blake2b = Blake2b::new();
        let input = b"Hello, world!";

        // Update the hash with the input
        blake2b.hash_update(input);

        // Finalize the hash and store it in `output`
        let mut output = [0u8; 64];
        blake2b.hash_finalize(&mut output);

        let _expected_hash = [
            120, 106, 2, 247, 66, 1, 89, 3, 198, 198, 253, 133, 37, 82, 210, 114, 145, 47, 71, 64,
            225, 88, 71, 97, 138, 134, 226, 23, 247, 31, 84, 25, 210, 94, 16, 49, 175, 238, 88, 83,
            19, 137, 100, 68, 147, 78, 176, 75, 144, 58, 104, 91, 20, 72, 183, 85, 213, 111, 112,
            26, 254, 155, 226, 206,
        ];
        // Assert that the output is not all zeros
        assert_ne!(output, [0u8; 64]);
    }

    #[test]
    fn test_blake2b_reset() {
        let mut blake2b = Blake2b::new();
        let input = b"Hello, world!";

        // Update the hash with the input
        blake2b.hash_update(input);

        // Reset the hash
        blake2b.reset();

        // Finalize the hash and store it in `output`
        let mut output = [0u8; 64];
        blake2b.hash_finalize(&mut output);

        let expected_hash = [
            120, 106, 2, 247, 66, 1, 89, 3, 198, 198, 253, 133, 37, 82, 210, 114, 145, 47, 71, 64,
            225, 88, 71, 97, 138, 134, 226, 23, 247, 31, 84, 25, 210, 94, 16, 49, 175, 238, 88, 83,
            19, 137, 100, 68, 147, 78, 176, 75, 144, 58, 104, 91, 20, 72, 183, 85, 213, 111, 112,
            26, 254, 155, 226, 206,
        ];
        // Assert that the output is all zeros (since we reset the hash)
        assert_eq!(output, expected_hash);
    }
    #[test]
    fn test_div_ceil() {
        assert_eq!(div_ceil(5, 2), 3);
        assert_eq!(div_ceil(10, 3), 4);
        assert_eq!(div_ceil(8, 4), 2);
        assert_eq!(div_ceil(0, 1), 0);
        assert_eq!(div_ceil(1, 1), 1);
    }

    #[test]
    fn test_bsd() {
        let mut bsd = BSD::new();
        bsd.hash_update(&[0x61, 0x62, 0x63]);
        let mut result = [0u8; 2];
        bsd.hash_finalize(&mut result);
        println!("{:?}", result);
        assert_eq!(result, [172, 64]);

        let mut bsd = BSD::new();
        bsd.hash_update(&[0x31, 0x32, 0x33]);
        let mut result = [0u8; 2];
        bsd.hash_finalize(&mut result);
        println!("{:?}", result);
        assert_eq!(result, [88, 64]);

        let mut bsd = BSD::new();
        bsd.hash_update(&[]);
        let mut result = [0u8; 2];
        bsd.hash_finalize(&mut result);
        println!("{:?}", result);
        assert_eq!(result, [0x0, 0x0]);

        let mut bsd = BSD::new();
        bsd.hash_update(&[0xff, 0xfe, 0xfd]);
        let mut result = [0u8; 2];
        bsd.hash_finalize(&mut result);
        println!("{:?}", result);
        assert_eq!(result, [187, 193]);
    }

    #[test]
    fn test_bsd_result_str() {
        let mut bsd = BSD::new();
        bsd.hash_update(&[0x61, 0x62, 0x63]);
        println!("{:?}", bsd.result_str());
        assert_eq!(bsd.result_str(), "16556");

        let mut bsd = BSD::new();
        bsd.hash_update(&[0x31, 0x32, 0x33]);
        println!("{:?}", bsd.result_str());
        assert_eq!(bsd.result_str(), "16472");

        let mut bsd = BSD::new();
        bsd.hash_update(&[]);
        println!("{:?}", bsd.result_str());
        assert_eq!(bsd.result_str(), "0");

        let mut bsd = BSD::new();
        bsd.hash_update(&[0xff, 0xfe, 0xfd]);
        println!("{:?}", bsd.result_str());
        assert_eq!(bsd.result_str(), "49595");
    }

}
