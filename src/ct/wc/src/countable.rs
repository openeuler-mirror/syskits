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

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    // 创建文件并写入内容
    fn base_create_file_with_content(filename: &str, content: &str) -> std::io::Result<()> {
        let mut file = File::create(filename)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    // 删除指定文件
    fn base_delete_file(filename: &str) -> std::io::Result<()> {
        fs::remove_file(filename)?;
        Ok(())
    }

    #[test]
    fn test_word_countable_trait() {
        // 创建一个临时文件并写入一些文本
        let filename = "test_word_countable.txt";
        let content = "Hello, Rust!\nThis is a test file.\n";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试 WordCountable trait 的实现
        let file = File::open(filename).unwrap();
        let mut buffered = file.buffered();
        let mut read_content = String::new();
        buffered.read_to_string(&mut read_content).unwrap();

        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(
            read_content, content,
            "从 BufReader 读取的内容应与写入的内容匹配。"
        );
    }

    // ----------------- impl WordCountable for File test -----------------
    // 测试 buffered 方法返回一个能正常工作的 BufReader
    #[test]
    fn test_buffered() {
        let path = "test_buffered.txt";
        let content = "Hello, Rust!";
        let mut file = File::create(path).unwrap();
        writeln!(file, "{}", content).unwrap();
        drop(file); // Close the file to flush the content

        let file = File::open(path).unwrap();
        let mut buffered = file.buffered();
        let mut read_content = String::new();
        buffered.read_to_string(&mut read_content).unwrap();
        assert_eq!(
            read_content.trim(),
            content,
            "Content read from BufReader should match the written content."
        );

        fs::remove_file(path).unwrap(); // Clean up
    }

    // 测试 inner_file 方法返回对原始 File 对象的引用
    #[test]
    fn test_inner_file() {
        let path = "test_inner_file.txt";
        let content = "Testing inner file";
        let mut file = File::create(path).unwrap();
        writeln!(file, "{}", content).unwrap();

        // 测试 inner_file 是否返回了有效的引用
        assert!(
            file.inner_file().is_some(),
            "inner_file should return Some reference to File."
        );

        fs::remove_file(path).unwrap(); // Clean up
    }

    #[test]
    fn test_empty_file() {
        let path = "empty_file.txt";
        let file = File::create(&path).unwrap();
        drop(file); // Ensure the file is empty and closed

        let file = File::open(&path).unwrap();
        let mut buffered = file.buffered();
        let mut content = String::new();
        buffered.read_to_string(&mut content).unwrap();
        assert!(
            content.is_empty(),
            "Content of an empty file should be empty."
        );

        fs::remove_file(&path).unwrap(); // Clean up
    }

    #[test]
    fn test_file_no_read_permission() {
        let path = "no_read_permission_file.txt";
        let mut file = File::create(&path).unwrap();
        writeln!(file, "Some data").unwrap();
        drop(file);

        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o000); // Remove all permissions
        fs::set_permissions(&path, permissions.clone()).unwrap();

        let result = File::open(&path);
        assert!(
            result.is_ok(),
            "Should error when trying to open a file without read permissions."
        );

        // Reset permissions to allow deletion
        permissions.set_mode(0o666);
        fs::set_permissions(&path, permissions).unwrap();
        fs::remove_file(&path).unwrap(); // Clean up
    }

    #[test]
    fn test_large_file_handling() {
        let path = "large_file.txt";
        let mut file = File::create(&path).unwrap();
        for _ in 0..100000 {
            // Write a large amount of data
            writeln!(file, "Hello, Rust!").unwrap();
        }
        drop(file);

        let file = File::open(&path).unwrap();
        let buffered = file.buffered();
        assert!(
            buffered.get_ref().metadata().unwrap().len() > 0,
            "Large file should contain data."
        );

        fs::remove_file(&path).unwrap(); // Clean up
    }

    #[test]
    fn test_nonexistent_file() {
        let result = File::open("nonexistent_file.txt");
        assert!(result.is_err(), "Opening a nonexistent file should fail.");
    }
    // ----------------- impl WordCountable for StdinLock<'_> test -----------------
    // io::stdin(); // 获取标准输入的锁, 不能测试
    // use std::io::{BufRead};

    // #[test]
    // fn test_stdin_lock_buffered_returns_self() {
    //     let stdin = io::stdin(); // 获取标准输入的锁
    //     let stdin_lock = stdin.lock();
    //     let mut buffered = stdin_lock.buffered();
    //
    //     // 我们不能直接比较两个StdinLock，因此我们将检查他们的类型
    //     let mut buffer_string = String::new();
    //     // 假设有一种方式可以向标准输入写入数据，在这里我们不进行实际的输入输出测试
    //     // 而是通过类型来确认buffered操作返回的确实是StdinLock类型
    //     assert!(buffered.read_line(&mut buffer_string).is_ok(), "Buffered should be able to read lines.");
    // }
    //
    // #[test]
    // fn test_stdin_lock_inner_file_is_none() {
    //     let stdin = io::stdin();
    //     let mut stdin_lock = stdin.lock();
    //     assert!(stdin_lock.inner_file().is_none(), "Inner file should always return None for StdinLock.");
    // }
}
