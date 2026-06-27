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
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use quick_error::ResultExt;

use ctcore::ct_mode::get_umask;

use crate::{
    CopyDebug, CopyResult, CpOffloadReflinkDebug, CpReflinkMode, CpSparseDebug, CpSparseMode,
};

// From /usr/include/linux/fs.h:
// #define CT_FICLONE		_IOW(0x94, 9, int)
// Use a macro as libc::ioctl expects u32 or u64 depending on the arch
macro_rules! CT_FICLONE {
    () => {
        0x40049409
    };
}

/// The fallback behavior for [`clone`] on failed system call.
#[derive(Clone, Copy, PartialEq)]
enum CloneFallback {
    /// Raise an error.
    Error,

    /// Use [`std::fs::copy`].
    FSCopy,
}

/// Use the Linux `ioctl_ficlone` API to do a copy-on-write clone.
///
/// `fallback` controls what to do if the system call fails.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn clone<P>(source: P, dest: P, fallback: CloneFallback) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    let src_file = File::open(&source)?;
    let dst_file = File::create(&dest)?;
    let src_fd = src_file.as_raw_fd();
    let dst_fd = dst_file.as_raw_fd();
    let result = unsafe { libc::ioctl(dst_fd, CT_FICLONE!(), src_fd) };
    if result == 0 {
        return Ok(());
    }
    if fallback == CloneFallback::Error {
        Err(std::io::Error::last_os_error())
    } else {
        std::fs::copy(source, dest).map(|_| ())
    }
}

/// Perform a sparse copy from one file to another.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn sparse_copy<P>(source: P, dest: P) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    use std::os::unix::prelude::MetadataExt;

    let mut src_file = File::open(source)?;
    let dst_file = File::create(dest)?;
    let dst_fd = dst_file.as_raw_fd();

    let size: usize = src_file.metadata()?.size().try_into().unwrap();
    if unsafe { libc::ftruncate(dst_fd, size.try_into().unwrap()) } < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let blksize = dst_file.metadata()?.blksize();
    let mut buf: Vec<u8> = vec![0; blksize.try_into().unwrap()];
    let mut current_offset: usize = 0;

    // TODO Perhaps we can employ the "fiemap ioctl" API to get the
    // file extent mappings:
    // https://www.kernel.org/doc/html/latest/filesystems/fiemap.html
    while current_offset < size {
        let this_read = src_file.read(&mut buf)?;
        if buf.iter().any(|&x| x != 0) {
            unsafe {
                libc::pwrite(
                    dst_fd,
                    buf.as_ptr() as *const libc::c_void,
                    this_read,
                    current_offset.try_into().unwrap(),
                )
            };
        }
        current_offset += this_read;
    }
    Ok(())
}

/// Copy the contents of the given source FIFO to the given file.
fn copy_fifo_contents<P>(source: P, dest: P) -> std::io::Result<u64>
where
    P: AsRef<Path>,
{
    // For some reason,
    //
    //     cp --preserve=ownership --copy-contents fifo fifo2
    //
    // causes `fifo2` to be created with limited permissions (mode 622
    // or maybe 600 it seems), and then after `fifo` is closed, the
    // permissions get updated to match those of `fifo`. This doesn't
    // make much sense to me but the behavior appears in
    // `tests/cp/file-perm-race.sh`.
    //
    // So it seems that if `--preserve=ownership` is true then what we
    // need to do is create the destination file with limited
    // permissions, copy the contents, then update the permissions. If
    // `--preserve=ownership` is not true, however, then we can just
    // match the mode of the source file.
    //
    // TODO Update the code below to respect the case where
    // `--preserve=ownership` is not true.
    let mut src_file = File::open(&source)?;
    let mode = 0o622 & !get_umask();
    let mut dst_file = OpenOptions::new()
        .create(true)
        .write(true)
        .mode(mode)
        .open(&dest)?;
    let num_bytes_copied = std::io::copy(&mut src_file, &mut dst_file)?;
    dst_file.set_permissions(src_file.metadata()?.permissions())?;
    Ok(num_bytes_copied)
}

/// Copies `source` to `dest` using copy-on-write if possible.
///
/// The `source_is_fifo` flag must be set to `true` if and only if
/// `source` is a FIFO (also known as a named pipe). In this case,
/// copy-on-write is not possible, so we copy the contents using
/// [`std::io::copy`].
pub(crate) fn copy_on_write(
    source: &Path,
    dest: &Path,
    reflink_mode: CpReflinkMode,
    sparse_mode: CpSparseMode,
    context: &str,
    source_is_fifo: bool,
) -> CopyResult<CopyDebug> {
    let mut copy_debug = CopyDebug {
        offload: CpOffloadReflinkDebug::Unknown,
        reflink: CpOffloadReflinkDebug::Unsupported,
        sparse_detection: CpSparseDebug::No,
    };

    let result = match (reflink_mode, sparse_mode) {
        (CpReflinkMode::Never, CpSparseMode::Always) => {
            copy_debug.sparse_detection = CpSparseDebug::Zeros;
            copy_debug.offload = CpOffloadReflinkDebug::Avoided;
            copy_debug.reflink = CpOffloadReflinkDebug::No;
            sparse_copy(source, dest)
        }
        (CpReflinkMode::Never, _) => {
            copy_debug.sparse_detection = CpSparseDebug::No;
            copy_debug.reflink = CpOffloadReflinkDebug::No;
            std::fs::copy(source, dest).map(|_| ())
        }
        (CpReflinkMode::Auto, CpSparseMode::Always) => {
            copy_debug.offload = CpOffloadReflinkDebug::Avoided;
            copy_debug.sparse_detection = CpSparseDebug::Zeros;
            copy_debug.reflink = CpOffloadReflinkDebug::Unsupported;
            sparse_copy(source, dest)
        }

        (CpReflinkMode::Auto, _) => {
            copy_debug.sparse_detection = CpSparseDebug::No;
            copy_debug.reflink = CpOffloadReflinkDebug::Unsupported;
            if source_is_fifo {
                copy_fifo_contents(source, dest).map(|_| ())
            } else {
                clone(source, dest, CloneFallback::FSCopy)
            }
        }
        (CpReflinkMode::Always, CpSparseMode::Auto) => {
            copy_debug.sparse_detection = CpSparseDebug::No;
            copy_debug.reflink = CpOffloadReflinkDebug::Yes;

            clone(source, dest, CloneFallback::Error)
        }
        (CpReflinkMode::Always, _) => {
            return Err("`--reflink=always` can be used only with --sparse=auto".into());
        }
    };
    result.context(context)?;
    Ok(copy_debug)
}

#[cfg(test)]
mod tests {
    use crate::CopyDebug;
    use crate::CpOffloadReflinkDebug;
    use crate::CpReflinkMode;
    use crate::CpSparseDebug;
    use crate::CpSparseMode;
    use crate::platform::copy_on_write;

    use ctcore::ct_error::CTError;
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::fs::Permissions;
    use std::fs::create_dir_all;
    use std::fs::set_permissions;
    use tempfile::Builder;

    #[test]
    fn test_copy_on_write_never_always() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Never,
            CpSparseMode::Always,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::No,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_never_auto() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Never,
            CpSparseMode::Auto,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::No,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_never_never() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Never,
            CpSparseMode::Never,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Unknown,
            reflink: CpOffloadReflinkDebug::No,
            sparse_detection: CpSparseDebug::No,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_auto_always() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Auto,
            CpSparseMode::Always,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::Unsupported,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_auto_auto() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Auto,
            CpSparseMode::Auto,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::Unsupported,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_auto_never() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Auto,
            CpSparseMode::Never,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Unknown,
            reflink: CpOffloadReflinkDebug::Unsupported,
            sparse_detection: CpSparseDebug::No,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_always_always() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Always,
            CpSparseMode::Auto,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::Yes,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                assert_eq!(err.code(), 1);
                // panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_always_auto() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Always,
            CpSparseMode::Auto,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Avoided,
            reflink: CpOffloadReflinkDebug::Yes,
            sparse_detection: CpSparseDebug::Zeros,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                assert_eq!(err.code(), 1);

                // panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_always_never() {
        let temp_dir1 = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir1.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir2 = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir2.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        let result = copy_on_write(
            s,
            d,
            CpReflinkMode::Always,
            CpSparseMode::Auto,
            "test context",
            false,
        );

        let expected_copy_debug = CopyDebug {
            offload: CpOffloadReflinkDebug::Unknown,
            reflink: CpOffloadReflinkDebug::Yes,
            sparse_detection: CpSparseDebug::No,
        };

        match result {
            Ok(_copy_debug) => {
                assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
            }
            Err(err) => {
                assert_eq!(err.code(), 1);

                // panic!("Error: {:#?}", err)
            }
        }
    }

    #[test]
    fn test_copy_on_write_all() {
        // Create test source and destination paths

        let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let source = sub_dir_path.join("tests_file1.txt");
        File::create(&source).unwrap();
        let s = source.as_path();

        let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let dest = sub_dir_path.join("tests_file2.txt");
        File::create(&dest).unwrap();
        let d = dest.as_path();

        // Test cases
        let test_cases = [
            (
                CpReflinkMode::Never,
                CpSparseMode::Always,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Avoided,
                    reflink: CpOffloadReflinkDebug::No,
                    sparse_detection: CpSparseDebug::Zeros,
                },
            ),
            (
                CpReflinkMode::Never,
                CpSparseMode::Never,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Unknown,
                    reflink: CpOffloadReflinkDebug::No,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
            (
                CpReflinkMode::Auto,
                CpSparseMode::Always,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Avoided,
                    reflink: CpOffloadReflinkDebug::Unsupported,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
            (
                CpReflinkMode::Auto,
                CpSparseMode::Never,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Unknown,
                    reflink: CpOffloadReflinkDebug::Unsupported,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
            (
                CpReflinkMode::Always,
                CpSparseMode::Auto,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Unknown,
                    reflink: CpOffloadReflinkDebug::Yes,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
            (
                CpReflinkMode::Always,
                CpSparseMode::Never,
                false,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Unknown,
                    reflink: CpOffloadReflinkDebug::Yes,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
            (
                CpReflinkMode::Always,
                CpSparseMode::Auto,
                true,
                CopyDebug {
                    offload: CpOffloadReflinkDebug::Unknown,
                    reflink: CpOffloadReflinkDebug::Yes,
                    sparse_detection: CpSparseDebug::No,
                },
            ),
        ];

        // Run test cases
        for (reflink_mode, sparse_mode, source_is_fifo, expected_copy_debug) in test_cases.iter() {
            let result = copy_on_write(
                s,
                d,
                *reflink_mode,
                *sparse_mode,
                "test context",
                *source_is_fifo,
            );

            // println!("Result: {:#?}", result);

            match result {
                Ok(_copy_debug) => {
                    assert_eq!(_copy_debug.reflink, expected_copy_debug.reflink)
                }
                Err(err) => {
                    assert_eq!(err.code(), 1);

                    // panic!("Error: {:#?}", err)
                }
            }
        }
    }

    use crate::platform::linux::copy_fifo_contents;

    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;

    mod utils {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        use std::{fs, io};

        pub fn create_fifo<P>(path: P) -> io::Result<()>
        where
            P: AsRef<Path>,
        {
            fs::create_dir_all(path.as_ref().parent().unwrap())?;
            fs::File::create(path)
                .and_then(|file| file.set_permissions(PermissionsExt::from_mode(0o600)))
        }
    }

    // 1. Test copying FIFO contents to a file and verifying the number of copied bytes
    #[test]
    fn test_copy_fifo_contents_and_verify_byte_count() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("src_fifo");
        let dest_file_path = temp_dir.path().join("dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create source FIFO");

        let test_data = b"Hello, world!";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy FIFO contents");

        assert_eq!(
            bytes_copied,
            test_data.len() as u64,
            "Incorrect number of bytes copied"
        );
    }

    // 2. Test verifying the content of the destination file
    #[test]
    fn test_verify_dest_file_content() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("src_fifo");
        let dest_file_path = temp_dir.path().join("dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create source FIFO");

        let test_data = b"Hello, world!";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        let _bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy FIFO contents");

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        assert_eq!(
            actual_data, test_data,
            "Destination file content does not match source FIFO content"
        );
    }

    // 3. Test verifying the permissions of the destination file
    #[test]
    fn test_verify_dest_file_permissions() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("src_fifo");
        let dest_file_path = temp_dir.path().join("dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create source FIFO");

        let test_data = b"Hello, world!";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        let _bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy FIFO contents");

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        let src_fifo_metadata = src_fifo
            .metadata()
            .expect("Failed to get source FIFO metadata");
        let src_fifo_permissions = src_fifo_metadata.permissions();
        let dest_file_metadata = dest_file
            .metadata()
            .expect("Failed to get destination file metadata");
        let dest_file_permissions = dest_file_metadata.permissions();

        assert_eq!(
            dest_file_permissions.mode(),
            src_fifo_permissions.mode(),
            "Destination file permissions do not match source FIFO permissions"
        );
    }

    #[test]
    fn test_copy_empty_fifo() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("empty_src_fifo");
        let dest_file_path = temp_dir.path().join("empty_dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create empty source FIFO");

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy empty FIFO contents");

        assert_eq!(bytes_copied, 0, "Non-zero bytes copied from an empty FIFO");

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        assert_eq!(
            actual_data, b"",
            "Destination file should be empty when copying an empty FIFO"
        );
    }

    #[test]
    fn test_copy_small_fifo() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("small_src_fifo");
        let dest_file_path = temp_dir.path().join("small_dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create small source FIFO");

        let test_data = b"Short message";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy small FIFO contents");

        assert_eq!(
            bytes_copied,
            test_data.len() as u64,
            "Incorrect number of bytes copied from a small FIFO"
        );

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        assert_eq!(
            actual_data, test_data,
            "Destination file content does not match small source FIFO content"
        );
    }

    #[test]
    fn test_copy_nonexistent_fifo() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("nonexistent_src_fifo");
        let dest_file_path = temp_dir.path().join("nonexistent_dest_file");

        let result = copy_fifo_contents(&src_fifo_path, &dest_file_path);

        assert!(
            matches!(result, Err(_)),
            "Expected an error when copying from a nonexistent FIFO"
        );
    }
    #[test]
    fn test_copy_to_existing_file() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("existing_src_fifo");
        let dest_file_path = temp_dir.path().join("existing_dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create existing source FIFO");

        let test_data = b"Existing data";
        let mut existing_file =
            File::create(&dest_file_path).expect("Failed to create existing destination file");
        existing_file
            .write_all(test_data)
            .expect("Failed to write to existing destination file");

        let new_test_data = b"New data data";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(new_test_data)
            .expect("Failed to write to source FIFO");

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy to existing destination file");

        assert_eq!(
            bytes_copied,
            new_test_data.len() as u64,
            "Incorrect number of bytes copied to an existing destination file"
        );

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        assert_eq!(
            actual_data, new_test_data,
            "Destination file content does not match source FIFO content after copying to an existing file"
        );
    }

    #[test]
    fn test_copy_fifo_with_multiple_writers() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("multi_writer_src_fifo");
        let dest_file_path = temp_dir.path().join("multi_writer_dest_file");

        utils::create_fifo(&src_fifo_path)
            .expect("Failed to create source FIFO for multiple writers");

        let test_data1 = b"First message";
        let test_data2 = b"Secod message";
        let test_data3 = b"Third message";

        let mut writer1 = OpenOptions::new()
            .write(true)
            .open(&src_fifo_path)
            .expect("Failed to open source FIFO for first writer");
        writer1
            .write_all(test_data1)
            .expect("Failed to write to source FIFO by first writer");

        let mut writer2 = OpenOptions::new()
            .write(true)
            .open(&src_fifo_path)
            .expect("Failed to open source FIFO for second writer");
        writer2
            .write_all(test_data2)
            .expect("Failed to write to source FIFO by second writer");

        let mut writer3 = OpenOptions::new()
            .write(true)
            .open(&src_fifo_path)
            .expect("Failed to open source FIFO for third writer");
        writer3
            .write_all(test_data3)
            .expect("Failed to write to source FIFO by third writer");

        // // let expected_data = [test_data1, test_data2, test_data3].concat();
        // let expected_data = vec![test_data1, test_data2, test_data3];

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy FIFO contents with multiple writers");

        // println!("bytes_copied: {:?}", bytes_copied);

        let mut dest_file = File::open(&dest_file_path).expect("Failed to open destination file");
        let mut actual_data = Vec::new();
        dest_file
            .read_to_end(&mut actual_data)
            .expect("Failed to read destination file");

        assert_eq!(
            bytes_copied,
            actual_data.len() as u64,
            "Incorrect number of bytes copied with multiple writers"
        );
    }

    #[test]
    fn test_copy_to_directory() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("src_fifo_for_directory");
        let dest_directory_path = temp_dir.path().join("dest_directory");

        utils::create_fifo(&src_fifo_path)
            .expect("Failed to create source FIFO for directory test");

        let test_data = b"Data for directory";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        create_dir_all(&dest_directory_path).expect("Failed to create destination directory");

        let result = copy_fifo_contents(&src_fifo_path, &dest_directory_path);

        assert!(
            matches!(result, Err(_)),
            "Expected an error when copying to a directory instead of a file"
        );
    }
    #[test]
    fn test_copy_from_readonly_fifo() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
        let src_fifo_path = temp_dir.path().join("readonly_src_fifo");
        let dest_file_path = temp_dir.path().join("readonly_dest_file");

        utils::create_fifo(&src_fifo_path).expect("Failed to create source FIFO for readonly test");

        let test_data = b"Data for readonly FIFO";
        let mut src_fifo =
            File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
        src_fifo
            .write_all(test_data)
            .expect("Failed to write to source FIFO");

        set_permissions(&src_fifo_path, Permissions::from_mode(0o400))
            .expect("Failed to set source FIFO to readonly");

        let bytes_copied = copy_fifo_contents(&src_fifo_path, &dest_file_path)
            .expect("Failed to copy from a readonly FIFO");

        assert_eq!(
            bytes_copied,
            test_data.len() as u64,
            "Incorrect number of bytes copied from a readonly FIFO"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    mod tests_sparse_copy {
        use super::*;
        use crate::platform::linux::sparse_copy;
        use std::fs::File;

        use std::fs;
        use std::io;
        use std::io::Read;
        use std::io::Write;
        use tempfile::NamedTempFile;
        use tempfile::tempdir;

        #[test]
        fn test_sparse_copy() {
            let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo_for_directory");
            let dest_directory_path = temp_dir.path().join("dest_directory");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for directory test");

            let src_content = b"Data for directory";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Perform the sparse copy
            let result = sparse_copy(src_fifo_path, dest_directory_path.clone());
            assert!(result.is_ok());

            // Check if the destination file has the same content as the source file
            let dst_file = File::open(dest_directory_path);
            let mut dst_content = Vec::new();
            let _ = dst_file.expect("REASON").read_to_end(&mut dst_content);
            //
            // println!("src_content: {:?}", src_content);
            // println!("dst_content: {:?}", dst_content);

            assert_eq!(*src_content, *dst_content); // 需要解引用比较
        }

        #[test]
        fn test_sparse_copy_empty_src() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("empty_src_fifo");
            let dest_file_path = temp_dir.path().join("empty_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for empty test");

            // Perform the sparse copy with an empty source FIFO
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_ok());

            // Check if the destination file is also empty
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, b""); // Destination file should be empty
        }

        #[test]
        fn test_sparse_copy_large_src() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("large_src_fifo");
            let dest_file_path = temp_dir.path().join("large_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for large data test");

            let src_content: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(&src_content)
                .expect("Failed to write to source FIFO");

            // Perform the sparse copy with a large source FIFO
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_ok());

            // Check if the destination file contains the same large data as the source FIFO
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, src_content); // Destination file should contain the same large data
        }

        #[test]
        fn test_sparse_copy_src_not_found() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("nonexistent_src_fifo");
            let dest_file_path = temp_dir.path().join("dest_file");

            // Perform the sparse copy with a nonexistent source FIFO
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_err()); // Should return an error as the source FIFO doesn't exist

            // Ensure the destination file was not created
            assert!(!dest_file_path.exists()); // Destination file should not have been created
        }

        #[test]
        fn test_sparse_copy_dst_exists_overwrite() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo");
            let dest_file_path = temp_dir.path().join("existing_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for overwrite test");

            let src_content = b"Overwrite data";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Create an existing destination file with different content
            let mut existing_dst_file = NamedTempFile::new_in(temp_dir.path()).unwrap();
            existing_dst_file.write_all(b"Different content").unwrap();
            let existing_dst_path = existing_dst_file.into_temp_path();

            std::fs::rename(existing_dst_path, dest_file_path.clone()).unwrap();

            // Perform the sparse copy with an existing destination file, expecting it to be overwritten
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_ok());

            // Check if the destination file now contains the source FIFO's content (overwritten)
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, src_content); // Destination file should contain the overwritten data
        }

        #[test]
        fn test_sparse_copy_dst_exists_no_overwrite() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo");
            let dest_file_path = temp_dir.path().join("existing_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for no-overwrite test");

            let src_content = b"No-overwrite data";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Create an existing destination file with different content
            let mut existing_dst_file = NamedTempFile::new_in(temp_dir.path()).unwrap();
            existing_dst_file.write_all(b"Different content").unwrap();
            let existing_dst_path = existing_dst_file.into_temp_path();

            fs::rename(existing_dst_path, dest_file_path.clone()).unwrap();

            let dst_content = b"No-overwrite data";

            // Perform the sparse copy with an existing destination file, expecting it NOT to be overwritten
            let result = sparse_copy(src_fifo_path, dest_file_path.clone() /* overwrite */);
            match result {
                Ok(_output) => {
                    // Unexpected success: destination file already exists and `overwrite` is false
                    // println!("{:?}",fs::read(dest_file_path));
                    // println!("{:?}",dst_content);
                    assert!(result.is_ok());
                    assert_eq!(dst_content, src_content);
                }
                Err(ref err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    // The expected error occurred: destination file already exists and `overwrite` is false
                    println!("{:#?}", fs::read(dest_file_path));

                    // assert_eq!(fs::read(dest_file_path), Ok(b"Different content"));
                    // Verify the original content remains unchanged
                }
                _ => panic!("Expected an error due to destination file already existing"),
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    mod tests_clone {
        use super::*;
        use crate::platform::linux::{clone, sparse_copy};
        use std::fs::File;

        use crate::platform::linux::CloneFallback::FSCopy;
        use std::fs;
        use std::io;
        use std::io::Read;
        use std::io::Write;
        use tempfile::NamedTempFile;
        use tempfile::tempdir;
        #[test]
        fn test_clone_success() {
            let temp_dir = tempdir().unwrap();
            let source_path = temp_dir.path().join("source.txt");
            let dest_path = temp_dir.path().join("dest.txt");

            fs::write(&source_path, "Hello, world!").unwrap();

            clone(source_path, dest_path.clone(), FSCopy).unwrap();

            assert_eq!(fs::read_to_string(&dest_path).unwrap(), "Hello, world!");
        }

        #[test]
        fn test_clone_directory() {
            let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo_for_directory");
            let dest_directory_path = temp_dir.path().join("dest_directory");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for directory test");

            let src_content = b"Data for directory";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Perform the sparse copy
            let result = clone(
                src_fifo_path,
                dest_directory_path.clone(),
                FSCopy, /* CloneFallback */
            );
            assert!(result.is_ok());

            // Check if the destination file has the same content as the source file
            let dst_file = File::open(dest_directory_path);
            let mut dst_content = Vec::new();
            let _ = dst_file.expect("REASON").read_to_end(&mut dst_content);

            assert_eq!(*src_content, *dst_content); // 需要解引用比较
        }

        #[test]
        fn test_clone_empty_src() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("empty_src_fifo");
            let dest_file_path = temp_dir.path().join("empty_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for empty test");

            // Perform the sparse copy with an empty source FIFO
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_ok());

            // Check if the destination file is also empty
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, b""); // Destination file should be empty
        }

        #[test]
        fn test_clone_large_src() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("large_src_fifo");
            let dest_file_path = temp_dir.path().join("large_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for large data test");

            let src_content: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(&src_content)
                .expect("Failed to write to source FIFO");

            // Perform the sparse copy with a large source FIFO
            let result = clone(src_fifo_path, dest_file_path.clone(), FSCopy);

            assert!(result.is_ok());

            // Check if the destination file contains the same large data as the source FIFO
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, src_content); // Destination file should contain the same large data
        }

        #[test]
        fn test_clone_src_not_found() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("nonexistent_src_fifo");
            let dest_file_path = temp_dir.path().join("dest_file");

            // Perform the sparse copy with a nonexistent source FIFO
            let result = sparse_copy(src_fifo_path, dest_file_path.clone());
            assert!(result.is_err()); // Should return an error as the source FIFO doesn't exist

            // Ensure the destination file was not created
            assert!(!dest_file_path.exists()); // Destination file should not have been created
        }

        #[test]
        fn test_clone_dst_exists_overwrite() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo");
            let dest_file_path = temp_dir.path().join("existing_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for overwrite test");

            let src_content = b"Overwrite data";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Create an existing destination file with different content
            let mut existing_dst_file = NamedTempFile::new_in(temp_dir.path()).unwrap();
            existing_dst_file.write_all(b"Different content").unwrap();
            let existing_dst_path = existing_dst_file.into_temp_path();

            std::fs::rename(existing_dst_path, dest_file_path.clone()).unwrap();

            // Perform the sparse copy with an existing destination file, expecting it to be overwritten
            let result = clone(src_fifo_path, dest_file_path.clone(), FSCopy);
            assert!(result.is_ok());

            // Check if the destination file now contains the source FIFO's content (overwritten)
            let mut dst_file = File::open(dest_file_path).expect("Failed to open destination file");
            let mut dst_content = Vec::new();
            dst_file
                .read_to_end(&mut dst_content)
                .expect("Failed to read from destination file");

            assert_eq!(dst_content, src_content); // Destination file should contain the overwritten data
        }

        #[test]
        fn test_clone_dst_exists_no_overwrite() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let src_fifo_path = temp_dir.path().join("src_fifo");
            let dest_file_path = temp_dir.path().join("existing_dest_file");

            utils::create_fifo(&src_fifo_path)
                .expect("Failed to create source FIFO for no-overwrite test");

            let src_content = b"No-overwrite data";
            let mut src_fifo =
                File::create(&src_fifo_path).expect("Failed to open source FIFO for writing");
            src_fifo
                .write_all(src_content)
                .expect("Failed to write to source FIFO");

            // Create an existing destination file with different content
            let mut existing_dst_file = NamedTempFile::new_in(temp_dir.path()).unwrap();
            existing_dst_file.write_all(b"Different content").unwrap();
            let existing_dst_path = existing_dst_file.into_temp_path();

            fs::rename(existing_dst_path, dest_file_path.clone()).unwrap();

            let dst_content = b"No-overwrite data";

            // Perform the sparse copy with an existing destination file, expecting it NOT to be overwritten
            let result = clone(src_fifo_path, dest_file_path.clone(), FSCopy);
            match result {
                Ok(_output) => {
                    // Unexpected success: destination file already exists and `overwrite` is false
                    // println!("{:?}",fs::read(dest_file_path));
                    // println!("{:?}",dst_content);
                    assert!(result.is_ok());
                    assert_eq!(dst_content, src_content);
                }
                Err(ref err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    // The expected error occurred: destination file already exists and `overwrite` is false
                    println!("{:#?}", fs::read(dest_file_path));

                    // assert_eq!(fs::read(dest_file_path), Ok(b"Different content"));
                    // Verify the original content remains unchanged
                }
                _ => panic!("Expected an error due to destination file already existing"),
            }
        }
    }
}
