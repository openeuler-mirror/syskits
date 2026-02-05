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
    Stdout(std::io::Stdout),
    File(std::fs::File),
}

impl AsFd for Device {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::File(f) => f.as_fd(),
            Self::Stdout(stdout) => stdout.as_fd(),
        }
    }
}

impl AsRawFd for Device {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(f) => f.as_raw_fd(),
            Self::Stdout(stdout) => stdout.as_raw_fd(),
        }
    }
}

#[allow(dead_code)]
impl Device {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Device::Stdout(_) => Ok(Device::Stdout(std::io::stdout())),
            Device::File(file) => Ok(Device::File(file.try_clone()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::stdout;
    use std::os::unix::io::AsRawFd;

    #[test]
    fn test_device_stdout() {
        let device = Device::Stdout(stdout());
        assert!(device.as_fd().as_raw_fd() > 0);
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
        let stdout_device = Device::Stdout(stdout());
        let stdout_fd = stdout_device.as_raw_fd();
        assert!(stdout_fd > 0);

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
        let stdout_device = Device::Stdout(stdout());
        let stdout_fd = stdout_device.as_fd().as_raw_fd();
        assert!(stdout_fd > 0);

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
    fn test_device_clone_stdout() {
        let device = Device::Stdout(stdout());
        let cloned = device.try_clone().unwrap();
        assert!(matches!(cloned, Device::Stdout(_)));
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
        let stdout_device = Device::Stdout(stdout());
        let debug_str = format!("{stdout_device:?}");
        assert!(debug_str.contains("Stdout"));

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
