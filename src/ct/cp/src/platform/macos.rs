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
use std::ffi::CString;
use std::fs::{self, File};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use quick_error::ResultExt;

use crate::{
    CopyDebug, CopyResult, CpOffloadReflinkDebug, CpReflinkMode, CpSparseDebug, CpSparseMode,
};

/// Copies `source` to `dest` using copy-on-write if possible.
///
/// The `source_is_fifo` flag must be set to `true` if and only if
/// `source` is a FIFO (also known as a named pipe).
pub(crate) fn copy_on_write(
    source: &Path,
    dest: &Path,
    reflink_mode: CpReflinkMode,
    sparse_mode: CpSparseMode,
    context: &str,
    source_is_fifo: bool,
) -> CopyResult<CopyDebug> {
    if sparse_mode != CpSparseMode::Auto {
        return Err("--sparse is only supported on linux".to_string().into());
    }
    let mut copy_debug = CopyDebug {
        offload: CpOffloadReflinkDebug::Unknown,
        reflink: CpOffloadReflinkDebug::Unsupported,
        sparse_detection: CpSparseDebug::Unsupported,
    };

    // Extract paths in a form suitable to be passed to a syscall.
    // The unwrap() is safe because they come from the command-line and so contain non nul
    // character.
    let src = CString::new(source.as_os_str().as_bytes()).unwrap();
    let dst = CString::new(dest.as_os_str().as_bytes()).unwrap();

    // clonefile(2) was introduced in macOS 10.12 so we cannot statically link against it
    // for backward compatibility.
    let clonefile = CString::new("clonefile").unwrap();
    let raw_pfn = unsafe { libc::dlsym(libc::RTLD_NEXT, clonefile.as_ptr()) };

    let mut error = 0;
    if !raw_pfn.is_null() {
        // Call clonefile(2).
        // Safety: Casting a C function pointer to a rust function value is one of the few
        // blessed uses of `transmute()`.
        unsafe {
            let pfn: extern "C" fn(
                src: *const libc::c_char,
                dst: *const libc::c_char,
                flags: u32,
            ) -> libc::c_int = std::mem::transmute(raw_pfn);
            error = pfn(src.as_ptr(), dst.as_ptr(), 0);
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists
                // Only remove the `dest` if the `source` and `dest` are not the same
                && source != dest
            {
                // clonefile(2) fails if the destination exists.  Remove it and try again.  Do not
                // bother to check if removal worked because we're going to try to clone again.
                // first lets make sure the dest file is not read only
                if fs::metadata(dest).map_or(false, |md| !md.permissions().readonly()) {
                    // remove and copy again
                    // TODO: rewrite this to better match linux behavior
                    // linux first opens the source file and destination file then uses the file
                    // descriptors to do the clone.
                    let _ = fs::remove_file(dest);
                    error = pfn(src.as_ptr(), dst.as_ptr(), 0);
                }
            }
        }
    }

    if raw_pfn.is_null() || error != 0 {
        // clonefile(2) is either not supported or it errored out (possibly because the FS does not
        // support COW).
        match reflink_mode {
            CpReflinkMode::Always => {
                return Err(format!("failed to clone {source:?} from {dest:?}: {error}").into());
            }
            _ => {
                copy_debug.reflink = CpOffloadReflinkDebug::Yes;
                if source_is_fifo {
                    let mut src_file = File::open(source)?;
                    let mut dst_file = File::create(dest)?;
                    io::copy(&mut src_file, &mut dst_file).context(context)?
                } else {
                    fs::copy(source, dest).context(context)?
                }
            }
        };
    }

    Ok(copy_debug)
}
#[cfg(test)]
mod tests {
    use crate::CopyDebug;
    use crate::CpOffloadReflinkDebug;
    use crate::CpReflinkMode;
    use crate::CpSparseDebug;
    use crate::CpSparseMode;
    use crate::copy_on_write;

    use ctcore::ct_error::CTError;
    use std::fs;
    use std::fs::File;
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
            reflink: CpOffloadReflinkDebug::Unsupported,
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
            reflink: CpOffloadReflinkDebug::Unsupported,
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
            reflink: CpOffloadReflinkDebug::Unsupported,
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
}
