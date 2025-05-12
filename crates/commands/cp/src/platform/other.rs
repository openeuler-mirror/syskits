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
use std::fs;
use std::path::Path;

use quick_error::ResultExt;

use crate::{
    CopyDebug, CopyResult, CpOffloadReflinkDebug, CpReflinkMode, CpSparseDebug, CpSparseMode,
};

/// Copies `source` to `dest` for systems without copy-on-write
pub(crate) fn copy_on_write(
    source: &Path,
    dest: &Path,
    reflink_mode: CpReflinkMode,
    sparse_mode: CpSparseMode,
    context: &str,
) -> CopyResult<CopyDebug> {
    if reflink_mode != CpReflinkMode::Never {
        return Err("--reflink is only supported on linux".to_string().into());
    }
    if sparse_mode != CpSparseMode::Auto {
        return Err("--sparse is only supported on linux".to_string().into());
    }
    let copy_debug = CopyDebug {
        offload: CpOffloadReflinkDebug::Unsupported,
        reflink: CpOffloadReflinkDebug::Unsupported,
        sparse_detection: CpSparseDebug::Unsupported,
    };
    fs::copy(source, dest).context(context)?;

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
