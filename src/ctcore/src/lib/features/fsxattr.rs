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

// spell-checker:ignore getxattr

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
pub fn copy_xattrs<P: AsRef<Path>>(source: P, dest: P) -> std::io::Result<()> {
    for attr_name in xattr::list(&source)? {
        if let Some(value) = xattr::get(&source, &attr_name)? {
            xattr::set(&dest, &attr_name, &value)?;
        }
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
pub fn retrieve_xattrs<P: AsRef<Path>>(source: P) -> std::io::Result<HashMap<OsString, Vec<u8>>> {
    let mut attrs = HashMap::new();
    for attr_name in xattr::list(&source)? {
        if let Some(value) = xattr::get(&source, &attr_name)? {
            attrs.insert(attr_name, value);
        }
    }
    Ok(attrs)
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
pub fn apply_xattrs<P: AsRef<Path>>(
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

