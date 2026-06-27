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

//! Get password/group file entry
//!
//! # Examples:
//!
//! ```
//! use ctcore::ct_entries::{self, Locate};
//!
//! let root_group = if cfg!(target_os = "linux") {
//!     "root"
//! } else {
//!     "wheel"
//! };
//!
//! assert_eq!("root", ct_entries::uid2usr(0).unwrap());
//! assert_eq!(0, ct_entries::usr2uid("root").unwrap());
//! assert!(ct_entries::gid2grp(0).is_ok());
//! assert!(ct_entries::grp2gid(root_group).is_ok());
//!
//! assert!(ct_entries::CtPasswd::locate(0).is_ok());
//! assert!(ct_entries::CtPasswd::locate("0").is_ok());
//! assert!(ct_entries::CtPasswd::locate("root").is_ok());
//!
//! assert!(ct_entries::Group::locate(0).is_ok());
//! assert!(ct_entries::Group::locate("0").is_ok());
//! assert!(ct_entries::Group::locate(root_group).is_ok());
//! ```

use libc::{c_char, c_int, gid_t, uid_t};
use libc::{getgrgid, getgrnam, getgroups};
use libc::{getpwnam, getpwuid, group, passwd};

use std::ffi::{CStr, CString};
use std::io::Error as IOError;
use std::io::ErrorKind;
use std::io::Result as IOResult;
use std::ptr;
use std::sync::Mutex;

use once_cell::sync::Lazy;

unsafe extern "C" {
    /// From: `<https://man7.org/linux/man-pages/man3/getgrouplist.3.html>`
    /// > The getgrouplist() function scans the group database to obtain
    /// > the list of groups that user belongs to.
    pub fn getgrouplist(
        user: *const c_char,
        group: gid_t,
        groups: *mut gid_t,
        ngroups: *mut c_int,
    ) -> c_int;
}

/// From: `<https://man7.org/linux/man-pages/man2/getgroups.2.html>`
/// > getgroups() returns the supplementary group IDs of the calling
/// > process in list.
/// > If size is zero, list is not modified, but the total number of
/// > supplementary group IDs for the process is returned.  This allows
/// > the caller to determine the size of a dynamically allocated list
/// > to be used in a further call to getgroups().
pub fn get_groups() -> IOResult<Vec<gid_t>> {
    let mut groups = Vec::new();
    loop {
        let ngroups = match unsafe { getgroups(0, ptr::null_mut()) } {
            -1 => return Err(IOError::last_os_error()),
            // Not just optimization; 0 would mess up the next call
            0 => return Ok(Vec::new()),
            n => n,
        };

        // 这是一个小缓冲区，所以我们能够负担得起对其进行零初始化，并使用安全的Vec操作
        groups.resize(ngroups.try_into().unwrap(), 0);
        let res = unsafe { getgroups(ngroups, groups.as_mut_ptr()) };
        if res == -1 {
            let err = IOError::last_os_error();
            if err.raw_os_error() == Some(libc::EINVAL) {
                // 更改的组数量，重试
                continue;
            } else {
                return Err(err);
            }
        } else {
            groups.truncate(ngroups.try_into().unwrap());
            return Ok(groups);
        }
    }
}

/// The list of group IDs returned from GNU's `groups` and GNU's `id --groups`
/// starts with the effective group ID (egid).
/// This is a wrapper for `get_groups()` to mimic this behavior.
///
/// If `arg_id` is `None` (default), `get_groups_gnu` moves the effective
/// group id (egid) to the first entry in the returned Vector.
/// If `arg_id` is `Some(x)`, `get_groups_gnu` moves the id with value `x`
/// to the first entry in the returned Vector. This might be necessary
/// for `id --groups --real` if `gid` and `egid` are not equal.
///
/// From: `<https://www.man7.org/linux/man-pages/man3/getgroups.3p.html>`
/// > As implied by the definition of supplementary groups, the
/// > effective group ID may appear in the array returned by
/// > getgroups() or it may be returned only by getegid().  Duplication
/// > may exist, but the application needs to call getegid() to be sure
/// > of getting all of the information. Various implementation
/// > variations and administrative sequences cause the set of groups
/// > appearing in the result of getgroups() to vary in order and as to
/// > whether the effective group ID is included, even when the set of
/// > groups is the same (in the mathematical sense of ``set''). (The
/// > history of a process and its parents could affect the details of
/// > the result.)
#[cfg(all(target_os = "linux", feature = "process"))]
pub fn get_groups_gnu(arg_id: Option<u32>) -> IOResult<Vec<gid_t>> {
    let groups = get_groups()?;
    let egid = arg_id.unwrap_or_else(crate::ct_features::ct_process::getegid);
    Ok(sort_groups(groups, egid))
}

#[cfg(all(target_os = "linux", feature = "process"))]
fn sort_groups(mut groups: Vec<gid_t>, egid: gid_t) -> Vec<gid_t> {
    if let Some(index) = groups.iter().position(|&x| x == egid) {
        groups[..=index].rotate_right(1);
    } else {
        groups.insert(0, egid);
    }
    groups
}

#[derive(Default, Clone, Debug)]
pub struct CtPasswd {
    /// AKA passwd.pw_name
    pub name: String,
    /// AKA passwd.pw_uid
    pub uid: uid_t,
    /// AKA passwd.pw_gid
    pub gid: gid_t,
    /// AKA passwd.pw_gecos
    pub user_info: Option<String>,
    /// AKA passwd.pw_shell
    pub user_shell: Option<String>,
    /// AKA passwd.pw_dir
    pub user_dir: Option<String>,
    /// AKA passwd.pw_passwd
    pub user_passwd: Option<String>,
}

/// SAFETY: ptr must point to a valid C string.
/// Returns None if ptr is null.
unsafe fn cstr2string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        unsafe { Some(CStr::from_ptr(ptr).to_string_lossy().into_owned()) }
    }
}

impl CtPasswd {
    /// SAFETY: All the pointed-to strings must be valid and not change while
    /// the function runs. That means PW_LOCK must be held.
    unsafe fn from_raw(raw: passwd) -> Self {
        Self {
            name: unsafe { cstr2string(raw.pw_name) }.expect("passwd without name"),
            uid: raw.pw_uid,
            gid: raw.pw_gid,
            user_info: unsafe { cstr2string(raw.pw_gecos) },
            user_shell: unsafe { cstr2string(raw.pw_shell) },
            user_dir: unsafe { cstr2string(raw.pw_dir) },
            user_passwd: unsafe { cstr2string(raw.pw_passwd) },
        }
    }

    /// This is a wrapper function for `libc::getgrouplist`.
    ///
    /// From: `<https://man7.org/linux/man-pages/man3/getgrouplist.3.html>`
    /// > If the number of groups of which user is a member is less than or
    /// > equal to *ngroups, then the value *ngroups is returned.
    /// > If the user is a member of more than *ngroups groups, then
    /// > getgrouplist() returns -1.  In this case, the value returned in
    /// > *ngroups can be used to resize the buffer passed to a further
    /// > call getgrouplist().
    ///
    /// However, on macOS/darwin (and maybe others?) `getgrouplist` does
    /// not update `ngroups` if `ngroups` is too small. Therefore, if not
    /// updated by `getgrouplist`, `ngroups` needs to be increased in a
    /// loop until `getgrouplist` stops returning -1.
    pub fn belongs_to(&self) -> Vec<gid_t> {
        let mut ngroups: c_int = 8;
        let mut ngroups_old: c_int;
        let mut groups = vec![0; ngroups.try_into().unwrap()];
        let name = CString::new(self.name.as_bytes()).unwrap();
        loop {
            ngroups_old = ngroups;
            if unsafe { getgrouplist(name.as_ptr(), self.gid, groups.as_mut_ptr(), &mut ngroups) }
                == -1
            {
                if ngroups == ngroups_old {
                    ngroups *= 2;
                }
                groups.resize(ngroups.try_into().unwrap(), 0);
            } else {
                break;
            }
        }
        let ngroups = ngroups.try_into().unwrap();
        assert!(ngroups <= groups.len());
        groups.truncate(ngroups);
        groups
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    /// AKA group.gr_name
    pub name: String,
    /// AKA group.gr_gid
    pub gid: gid_t,
}

impl Group {
    /// SAFETY: gr_name must be valid and not change while
    /// the function runs. That means PW_LOCK must be held.
    unsafe fn from_raw(raw: group) -> Self {
        Self {
            name: unsafe { cstr2string(raw.gr_name) }.expect("group without name"),
            gid: raw.gr_gid,
        }
    }
}

/// Fetch desired entry.
pub trait Locate<K> {
    fn locate(key: K) -> IOResult<Self>
    where
        Self: ::std::marker::Sized;
}

// 这些函数不是线程安全的：
// > 返回值可能会指向静态区域，并且可能会被后续调用 getpwent(3)，getpwnam() 或 getpwuid() 覆写。
// 这不仅适用于结构体，还适用于它所指向的字符串，因此我们必须在释放锁之前复制所有想要的数据。
// （从技术上讲，我们也必须确保程序其他地方没有调用这些原始函数。）
static PW_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

macro_rules! f {
    ($fnam:ident, $fid:ident, $t:ident, $st:ident) => {
        impl Locate<$t> for $st {
            fn locate(k: $t) -> IOResult<Self> {
                let _guard = PW_LOCK.lock();
                // 安全性：我们持有PW_LOCK。
                unsafe {
                    let data = $fid(k);
                    if !data.is_null() {
                        Ok($st::from_raw(ptr::read(data as *const _)))
                    } else {
                        // FIXME: 资源限制、信号和 I/O 失败也可能导致这种情况。
                        // 参见 getpwnam(3)。在调用前必须将 errno 设置为零。
                        // 我们可以在某些平台上使用 libc::__errno_location()。
                        // 下面两种情况也适用这一点。
                        Err(IOError::new(
                            ErrorKind::NotFound,
                            format!("No such id: {}", k),
                        ))
                    }
                }
            }
        }

        impl<'a> Locate<&'a str> for $st {
            fn locate(k: &'a str) -> IOResult<Self> {
                let _guard = PW_LOCK.lock();
                if let Ok(id) = k.parse::<$t>() {
                    // 安全性：我们持有PW_LOCK。
                    unsafe {
                        let data = $fid(id);
                        if !data.is_null() {
                            Ok($st::from_raw(ptr::read(data as *const _)))
                        } else {
                            Err(IOError::new(
                                ErrorKind::NotFound,
                                format!("No such id: {}", id),
                            ))
                        }
                    }
                } else {
                    // 安全性：我们持有PW_LOCK。
                    unsafe {
                        let cstring = CString::new(k).unwrap();
                        let data = $fnam(cstring.as_ptr());
                        if !data.is_null() {
                            Ok($st::from_raw(ptr::read(data as *const _)))
                        } else {
                            Err(IOError::new(
                                ErrorKind::NotFound,
                                format!("Not found: {}", k),
                            ))
                        }
                    }
                }
            }
        }
    };
}

f!(getpwnam, getpwuid, uid_t, CtPasswd);
f!(getgrnam, getgrgid, gid_t, Group);

#[inline]
pub fn uid2usr(id: uid_t) -> IOResult<String> {
    CtPasswd::locate(id).map(|p| p.name)
}

#[inline]
pub fn gid2grp(id: gid_t) -> IOResult<String> {
    Group::locate(id).map(|p| p.name)
}

#[inline]
pub fn usr2uid(name: &str) -> IOResult<uid_t> {
    CtPasswd::locate(name).map(|p| p.uid)
}

#[inline]
pub fn grp2gid(name: &str) -> IOResult<gid_t> {
    Group::locate(name).map(|p| p.gid)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sort_groups() {
        assert_eq!(sort_groups(vec![1, 2, 3], 4), vec![4, 1, 2, 3]);
        assert_eq!(sort_groups(vec![1, 2, 3], 3), vec![3, 1, 2]);
        assert_eq!(sort_groups(vec![1, 2, 3], 2), vec![2, 1, 3]);
        assert_eq!(sort_groups(vec![1, 2, 3], 1), vec![1, 2, 3]);
        assert_eq!(sort_groups(vec![1, 2, 3], 0), vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_entries_get_groups_gnu() {
        if let Ok(mut groups) = get_groups() {
            if let Some(last) = groups.pop() {
                groups.insert(0, last);
                assert_eq!(get_groups_gnu(Some(last)).unwrap(), groups);
            }
        }
    }
    #[test]
    fn test_belongs_to() {
        let test_user = CtPasswd {
            name: "test_user".to_string(),
            uid: 1000,
            gid: 1000,
            ..Default::default()
        };

        let groups = test_user.belongs_to();
        assert_eq!(groups, vec![1000]); // Assuming test_user only belongs to one group
    }

    #[test]
    fn test_get_groups() {
        /*
        // Test case 1: Successful retrieval of groups
        {
            let mut expected_groups = vec![1, 2, 3];

            // Mock the `getgroups` function to return the expected groups
            unsafe {
                let mut groups = expected_groups.as_mut_ptr();
                let ngroups = expected_groups.len() as libc::gid_t;
                libc::getgroups = Some(mock_getgroups);
                let result = get_groups();
                libc::getgroups = None;

                assert_eq!(result, Ok(expected_groups));
            }
        }

        // Test case 2: Error when `getgroups` returns -1

        {
            let expected_error = io::Error::from_raw_os_error libc::ENOMEM);

            // Mock the `getgroups` function to return an error
            unsafe {
                libc::getgroups = Some(mock_getgroups_error);
                let result = get_groups();
                libc::getgroups = None;

                assert_eq!(result, Err(expected_error));
            }
        }

        // Test case 3: Error when `getgroups` returns EINVAL
        {
            let expected_error = io::Error::from_raw_os_error libc::EINVAL);

            // Mock the `getgroups` function to return an error
            unsafe {
                libc::getgroups = Some(mock_getgroups_error);
                let result = get_groups();
                libc::getgroups = None;

                assert_eq!(result, Err(expected_error));
            }
        }
        */
    }
}
