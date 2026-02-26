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

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

#[derive(Debug)]
pub enum Device {
    Stdin(std::io::Stdin),
    File(std::fs::File),
}

impl AsFd for Device {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::File(f) => f.as_fd(),
            Self::Stdin(stdin) => stdin.as_fd(),
        }
    }
}

impl AsRawFd for Device {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(f) => f.as_raw_fd(),
            Self::Stdin(stdin) => stdin.as_raw_fd(),
        }
    }
}

#[allow(dead_code)]
impl Device {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Device::Stdin(_) => Ok(Device::Stdin(std::io::stdin())),
            Device::File(file) => Ok(Device::File(file.try_clone()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::stdin;
    use std::os::unix::io::AsRawFd;

    #[test]
    fn test_device_stdin() {
        let device = Device::Stdin(stdin());
        assert!(device.as_fd().as_raw_fd() >= 0);
    }

    #[test]
    fn test_device_file() {
        // 创建一个临时文件用于测试
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open("/tmp/test_stty")
            .unwrap();
        let device = Device::File(file);
        assert!(device.as_fd().as_raw_fd() > 0);
    }

    #[test]
    fn test_device_as_raw_fd() {
        let stdin_device = Device::Stdin(stdin());
        let stdin_fd = stdin_device.as_raw_fd();
        assert!(stdin_fd >= 0);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open("/tmp/test_stty")
            .unwrap();
        let file_fd = file.as_raw_fd();
        let file_device = Device::File(file);
        assert_eq!(file_device.as_raw_fd(), file_fd);
    }

    #[test]
    fn test_device_as_fd() {
        let stdin_device = Device::Stdin(stdin());
        let stdin_fd = stdin_device.as_fd().as_raw_fd();
        assert!(stdin_fd >= 0);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open("/tmp/test_stty")
            .unwrap();
        let file_fd = file.as_raw_fd();
        let file_device = Device::File(file);
        assert_eq!(file_device.as_fd().as_raw_fd(), file_fd);
    }

    #[test]
    fn test_device_clone_stdin() {
        let device = Device::Stdin(stdin());
        let cloned = device.try_clone().unwrap();
        assert!(matches!(cloned, Device::Stdin(_)));
    }

    #[test]
    fn test_device_clone_file() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open("/tmp/test_stty")
            .unwrap();
        let device = Device::File(file);
        let cloned = device.try_clone().unwrap();
        assert!(matches!(cloned, Device::File(_)));
    }

    #[test]
    fn test_device_debug() {
        let stdin_device = Device::Stdin(stdin());
        let debug_str = format!("{stdin_device:?}");
        assert!(debug_str.contains("Stdin"));

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open("/tmp/test_stty")
            .unwrap();
        let file_device = Device::File(file);
        let debug_str = format!("{file_device:?}");
        assert!(debug_str.contains("File"));
    }
}
