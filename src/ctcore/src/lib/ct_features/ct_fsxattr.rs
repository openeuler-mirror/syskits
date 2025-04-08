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

//! Set of functions to manage xattr on files and dirs
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

/// Copies extended attributes (xattrs) from one file or directory to another.
///
/// # Arguments
///
/// * `source` - A reference to the source path.
/// * `dest` - A reference to the destination path.
///
/// # Returns
///
/// A result indicating success or failure.
pub fn ct_copy_xattrs<P: AsRef<Path>>(source: P, dest: P) -> std::io::Result<()> {
    let ct_source = source.as_ref();
    let ct_dest = dest.as_ref();

    let attrs = xattr::list(ct_source)?;

    for attr in attrs {
        let value = match xattr::get(ct_source, &attr) {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(err) => return Err(err),
        };

        match xattr::set(ct_dest, &attr, &value) {
            Ok(_) => (),
            Err(err) => return Err(err),
        };
    }

    Ok(())
}
/// Retrieves the extended attributes (xattrs) of a given file or directory.
///
/// # Arguments
///
/// * `source` - A reference to the path of the file or directory.
///
/// # Returns
///
/// A result containing a HashMap of attributes names and values, or an error.
///
pub fn ct_retrieve_xattrs<P: AsRef<Path>>(
    source: P,
) -> std::io::Result<HashMap<OsString, Vec<u8>>> {
    let source_path = source.as_ref();

    // 获取源文件的所有扩展属性名称列表
    let ct_attr_names = xattr::list(source_path);
    match ct_attr_names {
        Ok(ct_attrs) => {
            let mut attrs = HashMap::new();
            for attr_name in ct_attrs {
                // 对每个属性名称，获取其对应的值
                let value_result = xattr::get(source_path, &attr_name);
                match value_result {
                    Ok(value_option) => {
                        // 如果存在属性值，则将其插入到HashMap中
                        if let Some(value) = value_option {
                            attrs.insert(attr_name, value);
                        }
                    }
                    Err(err) => {
                        // 如果在获取属性值时发生错误，则返回该错误
                        return Err(err);
                    }
                }
            }
            // 所有属性成功获取后，返回包含所有属性的HashMap
            Ok(attrs)
        }
        Err(err) => {
            // 如果在获取属性名称列表时发生错误，则返回该错误
            Err(err)
        }
    }
}

/// Applies extended attributes (xattrs) to a given file or directory.
///
/// # Arguments
///
/// * `dest` - A reference to the path of the file or directory.
/// * `xattrs` - A HashMap containing attribute names and their corresponding values.
///
/// # Returns
///
/// A result indicating success or failure.
pub fn ct_apply_xattrs<P: AsRef<Path>>(
    dest: P,
    xattrs: HashMap<OsString, Vec<u8>>,
) -> std::io::Result<()> {
    for (attr, value) in xattrs {
        xattr::set(&dest, &attr, &value)?;
    }
    Ok(())
}

/// Checks if a file has an Access Control List (ACL) based on its extended attributes.
///
/// # Arguments
///
/// * `file` - A reference to the path of the file.
///
/// # Returns
///
/// `true` if the file has extended attributes (indicating an ACL), `false` otherwise.
pub fn has_acl<P: AsRef<Path>>(file: P) -> bool {
    // don't use exacl here, it is doing more getxattr call then needed
    match xattr::list(file) {
        Ok(acl) => {
            // if we have extra attributes, we have an acl
            acl.count() > 0
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use std::fs::File;
    // use tempfile::tempdir;

    // #[test]
    // fn test_copy_xattrs() {
    //     let temp_dir = tempdir().unwrap();
    //     let source_path = temp_dir.path().join("source.txt");
    //     let dest_path = temp_dir.path().join("dest.txt");
    //
    //     File::create(&source_path).unwrap();
    //     File::create(&dest_path).unwrap();
    //
    //     let test_attr = "user.test";
    //     let test_value = b"test value";
    //     xattr::set(&source_path, test_attr, test_value).unwrap();
    //
    //     copy_xattrs(&source_path, &dest_path).unwrap();
    //
    //     let copied_value = xattr::get(&dest_path, test_attr).unwrap().unwrap();
    //     assert_eq!(copied_value, test_value);
    // }
    //
    // #[test]
    // fn test_apply_and_retrieve_xattrs() {
    //     let temp_dir = tempdir().unwrap();
    //     let file_path = temp_dir.path().join("test_file.txt");
    //
    //     File::create(&file_path).unwrap();
    //
    //     let mut test_xattrs = HashMap::new();
    //     let test_attr = "user.test_attr";
    //     let test_value = b"test value";
    //     test_xattrs.insert(OsString::from(test_attr), test_value.to_vec());
    //     apply_xattrs(&file_path, test_xattrs).unwrap();
    //
    //     let retrieved_xattrs = retrieve_xattrs(&file_path).unwrap();
    //     assert!(retrieved_xattrs.contains_key(OsString::from(test_attr).as_os_str()));
    //     assert_eq!(
    //         retrieved_xattrs
    //             .get(OsString::from(test_attr).as_os_str())
    //             .unwrap(),
    //         test_value
    //     );
    // }
    //
    // #[test]
    // fn test_file_has_acl() {
    //     let temp_dir = tempdir().unwrap();
    //     let file_path = temp_dir.path().join("test_file.txt");
    //
    //     File::create(&file_path).unwrap();
    //
    //     assert!(!has_acl(&file_path));
    //
    //     let test_attr = "user.test_acl";
    //     let test_value = b"test value";
    //     xattr::set(&file_path, test_attr, test_value).unwrap();
    //
    //     assert!(has_acl(&file_path));
    // }
}
