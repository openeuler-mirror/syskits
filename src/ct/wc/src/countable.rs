/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */
//!
//! 本模块为一些常见的类文件对象提供了 [`WcWordCountable`]特质和实现。
//! 使用 [`WcWordCountable::buffered`]。
//! method to get an iterator over lines of a file-like object.
use std::fs::File;
use std::io::{BufRead, BufReader, Read, StdinLock};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(unix)]
pub trait WcWordCountable: AsRawFd + Read {
    type Buffered: BufRead;
    fn buffered(self) -> Self::Buffered;
    fn inner_file(&mut self) -> Option<&mut File>;
}

#[cfg(not(unix))]
pub trait WcWordCountable: Read {
    type Buffered: BufRead;
    fn buffered(self) -> Self::Buffered;
    fn inner_file(&mut self) -> Option<&mut File>;
}

impl WcWordCountable for StdinLock<'_> {
    type Buffered = Self;

    fn buffered(self) -> Self::Buffered {
        self
    }
    fn inner_file(&mut self) -> Option<&mut File> {
        None
    }
}

impl WcWordCountable for File {
    type Buffered = BufReader<Self>;

    fn buffered(self) -> Self::Buffered {
        BufReader::new(self)
    }

    fn inner_file(&mut self) -> Option<&mut File> {
        Some(self)
    }
}

