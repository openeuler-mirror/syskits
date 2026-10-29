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

//! Set of functions to manage file systems

#[cfg(target_os = "linux")]
const CT_LINUX_MTAB: &str = "/etc/mtab";
#[cfg(target_os = "linux")]
const CT_LINUX_MOUNTINFO: &str = "/proc/self/mountinfo";
#[cfg(target_os = "linux")]
static MOUNT_OPT_BIND: &str = "bind";
#[cfg(target_os = "linux")]
const CT_MOUNT_RAW_BYTE_MARKER: char = '\u{F0000}';
#[cfg(target_os = "linux")]
const CT_MOUNT_RAW_BYTE_CHAR_BASE: u32 = 0xF0100;
#[cfg(windows)]
const MAX_PATH: usize = 266;
#[cfg(windows)]
static EXIT_ERR: i32 = 1;

#[cfg(windows)]
use crate::ct_show_warning;

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceW, GetDriveTypeW,
        GetVolumeInformationW, GetVolumePathNamesForVolumeNameW, QueryDosDeviceW,
    },
    System::WindowsProgramming::DRIVE_REMOTE,
};

#[cfg(windows)]
#[allow(non_snake_case)]
fn LPWSTR2String(buf: &[u16]) -> String {
    let len = buf.iter().position(|&n| n == 0).unwrap();
    String::from_utf16(&buf[..len]).unwrap()
}

#[cfg(windows)]
fn to_nul_terminated_wide_string(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<u16>>()
}

#[cfg(unix)]
use libc::{
    S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG, S_IFSOCK, mode_t, strerror,
};
use std::borrow::Cow;
#[cfg(unix)]
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::io::Error as IOError;
#[cfg(unix)]
use std::mem;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::path::Path;
use std::time::UNIX_EPOCH;

pub use libc::statfs as StatFs;

pub use libc::statfs as statfs_fn;

pub trait CtBirthTime {
    fn birth(&self) -> Option<(u64, u32)>;
}

use std::fs::Metadata;
impl CtBirthTime for Metadata {
    fn birth(&self) -> Option<(u64, u32)> {
        self.created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|e| (e.as_secs(), e.subsec_nanos()))
    }
}

#[derive(Debug, Clone)]
pub struct CtMountInfo {
    /// Stores `volume_name` in windows platform and `dev_id` in unix platform
    pub dev_id: String,
    pub dev_name: String,
    pub fs_type: String,
    pub mount_root: String,
    pub mount_dir: String,
    /// We only care whether this field contains "bind"
    pub mount_option: String,
    pub remote: bool,
    pub dummy: bool,
}

impl CtMountInfo {
    #[cfg(all(target_os = "linux", test))]
    fn new(filename: &str, raw: &[&str]) -> Option<Self> {
        let raw = raw.iter().map(|field| field.as_bytes()).collect::<Vec<_>>();
        Self::new_bytes(filename, &raw)
    }

    #[cfg(target_os = "linux")]
    fn new_bytes(filename: &str, raw: &[&[u8]]) -> Option<Self> {
        let (dev_name, fs_type, mount_root, mount_dir, mount_option) = match filename {
            CT_LINUX_MOUNTINFO => {
                let separator = raw.iter().position(|field| *field == b"-")?;
                if raw.len() <= separator + 2 || separator < 6 {
                    return None;
                }
                (
                    mount_field_from_bytes(raw[separator + 2]),
                    mount_field_from_bytes(raw[separator + 1]),
                    mount_field_from_bytes(raw[3]),
                    mount_field_from_bytes(raw[4]),
                    mount_field_from_bytes(raw[5]),
                )
            }
            CT_LINUX_MTAB => {
                if raw.len() < 4 {
                    return None;
                }
                (
                    mount_field_from_bytes(raw[0]),
                    mount_field_from_bytes(raw[2]),
                    String::new(),
                    mount_field_from_bytes(raw[1]),
                    mount_field_from_bytes(raw[3]),
                )
            }
            _ => return None,
        };

        let dev_id = mount_dev_id(&mount_dir);
        let dummy = is_dummy_filesystem(&fs_type, &mount_option);
        let remote = is_remote_filesystem(&dev_name, &fs_type);

        Some(Self {
            dev_id,
            dev_name,
            fs_type,
            mount_dir,
            mount_option,
            mount_root,
            remote,
            dummy,
        })
    }

    #[cfg(windows)]
    fn new(mut volume_name: String) -> Option<Self> {
        let mut dev_name_buf = [0u16; MAX_PATH];
        volume_name.pop();
        unsafe {
            QueryDosDeviceW(
                OsStr::new(&volume_name)
                    .encode_wide()
                    .chain(Some(0))
                    .skip(4)
                    .collect::<Vec<u16>>()
                    .as_ptr(),
                dev_name_buf.as_mut_ptr(),
                dev_name_buf.len() as u32,
            )
        };
        volume_name.push('\\');
        let dev_name = LPWSTR2String(&dev_name_buf);

        let mut mount_root_buf = [0u16; MAX_PATH];
        let success = unsafe {
            let volume_name = to_nul_terminated_wide_string(&volume_name);
            GetVolumePathNamesForVolumeNameW(
                volume_name.as_ptr(),
                mount_root_buf.as_mut_ptr(),
                mount_root_buf.len() as u32,
                ptr::null_mut(),
            )
        };
        if 0 == success {
            // TODO: support the case when `GetLastError()` returns `ERROR_MORE_DATA`
            return None;
        }
        let mount_root = LPWSTR2String(&mount_root_buf);

        let mut fs_type_buf = [0u16; MAX_PATH];
        let success = unsafe {
            let mount_root = to_nul_terminated_wide_string(&mount_root);
            GetVolumeInformationW(
                mount_root.as_ptr(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                fs_type_buf.as_mut_ptr(),
                fs_type_buf.len() as u32,
            )
        };
        let fs_type = if 0 == success {
            None
        } else {
            Some(LPWSTR2String(&fs_type_buf))
        };
        let remote = DRIVE_REMOTE
            == unsafe {
                let mount_root = to_nul_terminated_wide_string(&mount_root);
                GetDriveTypeW(mount_root.as_ptr())
            };
        Some(Self {
            dev_id: volume_name,
            dev_name,
            fs_type: fs_type.unwrap_or_default(),
            mount_root,
            mount_dir: String::new(),
            mount_option: String::new(),
            remote,
            dummy: false,
        })
    }
}

#[cfg(target_os = "linux")]
fn mount_raw_byte_char(byte: u8) -> char {
    char::from_u32(CT_MOUNT_RAW_BYTE_CHAR_BASE + u32::from(byte))
        .expect("mount raw byte placeholder is a valid private-use scalar")
}

#[cfg(target_os = "linux")]
fn mount_field_from_bytes(field: &[u8]) -> String {
    let mut unescaped = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] == b'\\'
            && index + 3 < field.len()
            && (b'0'..=b'3').contains(&field[index + 1])
            && field[index + 1..index + 4]
                .iter()
                .all(|byte| (b'0'..=b'7').contains(byte))
        {
            let value = (field[index + 1] - b'0') * 64
                + (field[index + 2] - b'0') * 8
                + (field[index + 3] - b'0');
            unescaped.push(value);
            index += 4;
        } else {
            unescaped.push(field[index]);
            index += 1;
        }
    }

    let mut decoded = String::with_capacity(unescaped.len());
    let mut rest = unescaped.as_slice();
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                push_mount_valid_text(&mut decoded, valid);
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                push_mount_valid_text(
                    &mut decoded,
                    std::str::from_utf8(&rest[..valid_up_to])
                        .expect("valid prefix reported by Utf8Error"),
                );
                let invalid_len = error.error_len().unwrap_or(rest.len() - valid_up_to);
                for &byte in &rest[valid_up_to..valid_up_to + invalid_len] {
                    decoded.push(CT_MOUNT_RAW_BYTE_MARKER);
                    decoded.push(mount_raw_byte_char(byte));
                }
                rest = &rest[valid_up_to + invalid_len..];
            }
        }
    }
    decoded
}

#[cfg(target_os = "linux")]
fn push_mount_valid_text(output: &mut String, text: &str) {
    for character in text.chars() {
        if character == CT_MOUNT_RAW_BYTE_MARKER {
            output.push(CT_MOUNT_RAW_BYTE_MARKER);
        }
        output.push(character);
    }
}

#[cfg(target_os = "linux")]
pub fn mount_field_to_bytes(field: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(field.len());
    let characters = field.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if character == CT_MOUNT_RAW_BYTE_MARKER && index + 1 < characters.len() {
            let next = characters[index + 1];
            if next == CT_MOUNT_RAW_BYTE_MARKER {
                push_mount_character_bytes(&mut bytes, CT_MOUNT_RAW_BYTE_MARKER);
                index += 2;
                continue;
            }

            let value = next as u32;
            if (CT_MOUNT_RAW_BYTE_CHAR_BASE..=CT_MOUNT_RAW_BYTE_CHAR_BASE + u32::from(u8::MAX))
                .contains(&value)
            {
                bytes.push((value - CT_MOUNT_RAW_BYTE_CHAR_BASE) as u8);
                index += 2;
                continue;
            }
        }

        push_mount_character_bytes(&mut bytes, character);
        index += 1;
    }
    bytes
}

#[cfg(target_os = "linux")]
fn push_mount_character_bytes(output: &mut Vec<u8>, character: char) {
    let mut encoded = [0; 4];
    output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
}

#[cfg(target_os = "linux")]
pub fn mount_field_to_os_string(field: &str) -> OsString {
    OsString::from_vec(mount_field_to_bytes(field))
}

#[cfg(target_os = "linux")]
fn is_dummy_filesystem(fs_type: &str, mount_option: &str) -> bool {
    // spell-checker:disable
    match fs_type {
        "autofs" | "proc" | "subfs"
        // for Linux 2.6/3.x
        | "debugfs" | "devpts" | "fusectl" | "mqueue" | "rpc_pipefs" | "sysfs"
        // FreeBSD, Linux 2.4
        | "devfs"
        // for NetBSD 3.0
        | "kernfs"
        // for Irix 6.5
        | "ignore" => true,
        _ => fs_type == "none"
            && !mount_option.contains(MOUNT_OPT_BIND)
    }
}

#[cfg(target_os = "linux")]
fn is_remote_filesystem(dev_name: &str, fs_type: &str) -> bool {
    dev_name.find(':').is_some()
        || (dev_name.starts_with("//") && fs_type == "smbfs" || fs_type == "cifs")
        || dev_name == "-hosts"
}

#[cfg(target_os = "linux")]
fn mount_dev_id(mount_dir: &str) -> String {
    use std::os::unix::fs::MetadataExt;

    if let Ok(stat) = std::fs::metadata(mount_field_to_os_string(mount_dir)) {
        (stat.dev() as i32).to_string()
    } else {
        String::new()
    }
}

#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};

/// Read file system list.
pub fn read_fs_list() -> Result<Vec<CtMountInfo>, std::io::Error> {
    #[cfg(target_os = "linux")]
    {
        let (file_name, f) = File::open(CT_LINUX_MOUNTINFO)
            .map(|f| (CT_LINUX_MOUNTINFO, f))
            .or_else(|_| File::open(CT_LINUX_MTAB).map(|f| (CT_LINUX_MTAB, f)))?;
        let reader = BufReader::new(f);
        Ok(reader
            .split(b'\n')
            .map_while(Result::ok)
            .filter_map(|line| {
                let raw_data = line
                    .split(|byte| byte.is_ascii_whitespace())
                    .filter(|field| !field.is_empty())
                    .collect::<Vec<_>>();
                CtMountInfo::new_bytes(file_name, &raw_data)
            })
            .collect::<Vec<_>>())
    }

    #[cfg(windows)]
    {
        let mut volume_name_buf = [0u16; MAX_PATH];
        // As recommended in the MS documentation, retrieve the first volume before the others
        let find_handle =
            unsafe { FindFirstVolumeW(volume_name_buf.as_mut_ptr(), volume_name_buf.len() as u32) };
        if INVALID_HANDLE_VALUE == find_handle {
            ct_crash!(
                EXIT_ERR,
                "FindFirstVolumeW failed: {}",
                IOError::last_os_error()
            );
        }
        let mut mounts = Vec::<CtMountInfo>::new();
        loop {
            let volume_name = LPWSTR2String(&volume_name_buf);
            if !volume_name.starts_with("\\\\?\\") || !volume_name.ends_with('\\') {
                ct_show_warning!("A bad path was skipped: {}", volume_name);
                continue;
            }
            if let Some(m) = CtMountInfo::new(volume_name) {
                mounts.push(m);
            }
            if 0 == unsafe {
                FindNextVolumeW(
                    find_handle,
                    volume_name_buf.as_mut_ptr(),
                    volume_name_buf.len() as u32,
                )
            } {
                let err = IOError::last_os_error();
                if err.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                    ct_crash!(EXIT_ERR, "FindNextVolumeW failed: {}", err);
                }
                break;
            }
        }
        unsafe {
            FindVolumeClose(find_handle);
        }
        Ok(mounts)
    }
}

#[derive(Debug, Clone)]
pub struct FsUsage {
    pub blocksize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub bavail_top_bit_set: bool,
    pub files: u64,
    pub ffree: u64,
}

impl FsUsage {
    #[cfg(unix)]
    pub fn new(statvfs: StatFs) -> Self {
        {
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            return Self {
                blocksize: statvfs.f_bsize as u64, // or `statvfs.f_frsize` ?
                blocks: statvfs.f_blocks,
                bfree: statvfs.f_bfree,
                bavail: statvfs.f_bavail,
                bavail_top_bit_set: ((statvfs.f_bavail) & (1u64.rotate_right(1))) != 0,
                files: statvfs.f_files,
                ffree: statvfs.f_ffree,
            };
            #[cfg(all(target_os = "linux", not(target_pointer_width = "64")))]
            return Self {
                blocksize: statvfs.f_bsize as u64, // or `statvfs.f_frsize` ?
                blocks: statvfs.f_blocks.into(),
                bfree: statvfs.f_bfree.into(),
                bavail: statvfs.f_bavail.into(),
                bavail_top_bit_set: ((statvfs.f_bavail as u64) & (1u64.rotate_right(1))) != 0,
                files: statvfs.f_files.into(),
                ffree: statvfs.f_ffree.into(),
            };
        }
    }
    #[cfg(windows)]
    pub fn new(path: &Path) -> Self {
        let mut root_path = [0u16; MAX_PATH];
        let success = unsafe {
            let path = to_nul_terminated_wide_string(path);
            GetVolumePathNamesForVolumeNameW(
                //path_utf8.as_ptr(),
                path.as_ptr(),
                root_path.as_mut_ptr(),
                root_path.len() as u32,
                ptr::null_mut(),
            )
        };
        if 0 == success {
            ct_crash!(
                EXIT_ERR,
                "GetVolumePathNamesForVolumeNameW failed: {}",
                IOError::last_os_error()
            );
        }

        let mut sectors_per_cluster = 0;
        let mut bytes_per_sector = 0;
        let mut number_of_free_clusters = 0;
        let mut total_number_of_clusters = 0;

        let success = unsafe {
            let path = to_nul_terminated_wide_string(path);
            GetDiskFreeSpaceW(
                path.as_ptr(),
                &mut sectors_per_cluster,
                &mut bytes_per_sector,
                &mut number_of_free_clusters,
                &mut total_number_of_clusters,
            )
        };
        if 0 == success {
            // Fails in case of CD for example
            // crash!(
            //     EXIT_ERR,
            //     "GetDiskFreeSpaceW failed: {}",
            //     IOError::last_os_error()
            // );
        }

        let bytes_per_cluster = sectors_per_cluster as u64 * bytes_per_sector as u64;
        Self {
            // f_bsize      File system block size.
            blocksize: bytes_per_cluster,
            // f_blocks - Total number of blocks on the file system, in units of f_frsize.
            // frsize =     Fundamental file system block size (fragment size).
            blocks: total_number_of_clusters as u64,
            //  Total number of free blocks.
            bfree: number_of_free_clusters as u64,
            //  Total number of free blocks available to non-privileged processes.
            bavail: 0,
            bavail_top_bit_set: ((bytes_per_sector as u64) & (1u64.rotate_right(1))) != 0,
            // Total number of file nodes (inodes) on the file system.
            files: 0, // Not available on windows
            // Total number of free file nodes (inodes).
            ffree: 0, // Meaningless on Windows
        }
    }
}

#[cfg(unix)]
pub trait FsMeta {
    fn fs_type(&self) -> i64;
    fn io_size(&self) -> u64;
    fn block_size(&self) -> i64;
    fn total_blocks(&self) -> u64;
    fn free_blocks(&self) -> u64;
    fn avail_blocks(&self) -> u64;
    fn total_file_nodes(&self) -> u64;
    fn free_file_nodes(&self) -> u64;
    fn fsid(&self) -> u64;
    fn namelen(&self) -> u64;
}

#[cfg(unix)]
impl FsMeta for StatFs {
    fn block_size(&self) -> i64 {
        #[cfg(all(
            not(target_env = "musl"),
            any(target_os = "linux", target_os = "windows"),
            not(target_arch = "s390x"),
            target_pointer_width = "64"
        ))]
        return self.f_bsize;
    }
    fn total_blocks(&self) -> u64 {
        #[cfg(target_pointer_width = "64")]
        return self.f_blocks;
        #[cfg(not(target_pointer_width = "64"))]
        return self.f_blocks.into();
    }
    fn free_blocks(&self) -> u64 {
        #[cfg(target_pointer_width = "64")]
        return self.f_bfree;
        #[cfg(not(target_pointer_width = "64"))]
        return self.f_bfree.into();
    }
    fn avail_blocks(&self) -> u64 {
        #[cfg(all(
            any(target_os = "linux", target_os = "windows"),
            target_pointer_width = "64"
        ))]
        return self.f_bavail;
        #[cfg(all(
            any(target_os = "linux", target_os = "windows"),
            not(target_pointer_width = "64")
        ))]
        return self.f_bavail.into();
    }
    fn total_file_nodes(&self) -> u64 {
        #[cfg(target_pointer_width = "64")]
        return self.f_files;
        #[cfg(not(target_pointer_width = "64"))]
        return self.f_files.into();
    }
    fn free_file_nodes(&self) -> u64 {
        #[cfg(all(
            any(target_os = "linux", target_os = "windows"),
            target_pointer_width = "64"
        ))]
        return self.f_ffree;
        #[cfg(all(
            any(target_os = "linux", target_os = "windows"),
            not(target_pointer_width = "64")
        ))]
        return self.f_ffree.into();
    }
    #[cfg(target_os = "linux")]
    fn fs_type(&self) -> i64 {
        #[cfg(all(
            not(target_env = "musl"),
            not(target_vendor = "apple"),
            any(target_os = "linux", target_os = "windows"),
            not(target_arch = "s390x"),
            target_pointer_width = "64"
        ))]
        return self.f_type;
    }
    #[cfg(target_os = "windows")]
    fn fs_type(&self) -> i64 {
        // FIXME: statvfs doesn't have an equivalent, so we need to do something else
        unimplemented!()
    }

    #[cfg(target_os = "linux")]
    fn io_size(&self) -> u64 {
        self.f_frsize as u64
    }
    // XXX: dunno if this is right
    #[cfg(target_os = "windows")]
    fn io_size(&self) -> u64 {
        self.f_bsize as u64
    }

    // Linux have a system call statfs() that returns
    // a struct statfs, containing a fsid_t f_fsid, where fsid_t is defined
    // as struct { int val[2];  }
    //
    #[cfg(target_os = "linux")]
    fn fsid(&self) -> u64 {
        let f_fsid: &[u32; 2] =
            unsafe { &*(&self.f_fsid as *const nix::sys::statfs::fsid_t as *const [u32; 2]) };
        ((u64::from(f_fsid[0])) << 32) | u64::from(f_fsid[1])
    }
    #[cfg(target_os = "windows")]
    fn fsid(&self) -> u64 {
        self.f_fsid as u64
    }

    #[cfg(target_os = "linux")]
    fn namelen(&self) -> u64 {
        self.f_namelen as u64
    }
    // XXX: should everything just use statvfs?
    #[cfg(target_os = "windows")]
    fn namelen(&self) -> u64 {
        self.f_namemax as u64 // spell-checker:disable-line
    }
}

#[allow(unused_unsafe)]
#[cfg(unix)]
pub fn statfs<P>(path: P) -> Result<StatFs, String>
where
    P: Into<Vec<u8>>,
{
    let path = path.into();
    let c_path = CString::new(path.clone()).map_err(|e| e.to_string())?;

    let mut buffer: StatFs = unsafe { mem::zeroed() };
    let result = unsafe { statfs_fn(c_path.as_ptr(), &mut buffer) };

    unsafe {
        if result == 0 {
            Ok(buffer)
        } else {
            let errno = IOError::last_os_error().raw_os_error().unwrap_or(0);
            let err_msg = CStr::from_ptr(unsafe { strerror(errno) })
                .to_str()
                .map_err(|_| "Error message contains invalid UTF-8".to_owned())?
                .to_owned();
            Err(err_msg)
        }
    }
}

#[cfg(unix)]
pub fn pretty_filetype<'a>(mode: mode_t, size: u64) -> &'a str {
    match mode & S_IFMT {
        S_IFREG => {
            if size == 0 {
                "regular empty file"
            } else {
                "regular file"
            }
        }
        S_IFDIR => "directory",
        S_IFLNK => "symbolic link",
        S_IFCHR => "character special file",
        S_IFBLK => "block special file",
        S_IFIFO => "fifo",
        S_IFSOCK => "socket",
        // TODO: Other file types
        // See coreutils/gnulib/lib/file-type.c // spell-checker:disable-line
        _ => "weird file",
    }
}

pub fn pretty_fstype<'a>(fstype: i64) -> Cow<'a, str> {
    //关闭拼写检查
    match fstype {
        0x6163_6673 => "acfs".into(),
        0xADF5 => "adfs".into(),
        0xADFF => "affs".into(),
        0x5346_414F => "afs".into(),
        0x0904_1934 => "anon-inode FS".into(),
        0x6175_6673 => "aufs".into(),
        0x0187 => "autofs".into(),
        0x4246_5331 => "befs".into(),
        0x6264_6576 => "bdevfs".into(),
        0x1BAD_FACE => "bfs".into(),
        0xCAFE_4A11 => "bpf_fs".into(),
        0x4249_4E4D => "binfmt_misc".into(),
        0x9123_683E => "btrfs".into(),
        0x7372_7279 => "btrfs_test".into(),
        0x00C3_6400 => "ceph".into(),
        0x0027_E0EB => "cgroupfs".into(),
        0xFF53_4D42 => "cifs".into(),
        0x7375_7245 => "coda".into(),
        0x012F_F7B7 => "coh".into(),
        0x6265_6570 => "configfs".into(),
        0x28CD_3D45 => "cramfs".into(),
        0x453D_CD28 => "cramfs-wend".into(),
        0x6462_6720 => "debugfs".into(),
        0x1373 => "devfs".into(),
        0x1CD1 => "devpts".into(),
        0xF15F => "ecryptfs".into(),
        0xDE5E_81E4 => "efivarfs".into(),
        0x0041_4A53 => "efs".into(),
        0x5DF5 => "exofs".into(),
        0x137D => "ext".into(),
        0xEF53 => "ext2/ext3".into(),
        0xEF51 => "ext2".into(),
        0xF2F5_2010 => "f2fs".into(),
        0x4006 => "fat".into(),
        0x1983_0326 => "fhgfs".into(),
        0x6573_5546 => "fuseblk".into(),
        0x6573_5543 => "fusectl".into(),
        0x0BAD_1DEA => "futexfs".into(),
        0x0116_1970 => "gfs/gfs2".into(),
        0x4750_4653 => "gpfs".into(),
        0x4244 => "hfs".into(),
        0x482B => "hfs+".into(),
        0x4858 => "hfsx".into(),
        0x00C0_FFEE => "hostfs".into(),
        0xF995_E849 => "hpfs".into(),
        0x9584_58F6 => "hugetlbfs".into(),
        0x1130_7854 => "inodefs".into(),
        0x0131_11A8 => "ibrix".into(),
        0x2BAD_1DEA => "inotifyfs".into(),
        0x9660 => "isofs".into(),
        0x4004 => "isofs".into(),
        0x4000 => "isofs".into(),
        0x07C0 => "jffs".into(),
        0x72B6 => "jffs2".into(),
        0x3153_464A => "jfs".into(),
        0x6B41_4653 => "k-afs".into(),
        0xC97E_8168 => "logfs".into(),
        0x0BD0_0BD0 => "lustre".into(),
        0x5346_314D => "m1fs".into(),
        0x137F => "minix".into(),
        0x138F => "minix (30 char.)".into(),
        0x2468 => "minix v2".into(),
        0x2478 => "minix v2 (30 char.)".into(),
        0x4D5A => "minix3".into(),
        0x1980_0202 => "mqueue".into(),
        0x4D44 => "msdos".into(),
        0x564C => "novell".into(),
        0x6969 => "nfs".into(),
        0x6E66_7364 => "nfsd".into(),
        0x3434 => "nilfs".into(),
        0x6E73_6673 => "nsfs".into(),
        0x5346_544E => "ntfs".into(),
        0x9FA1 => "openprom".into(),
        0x7461_636F => "ocfs2".into(),
        0x794C_7630 => "overlayfs".into(),
        0xAAD7_AAEA => "panfs".into(),
        0x5049_5045 => "pipefs".into(),
        0x7C7C_6673 => "prl_fs".into(),
        0x9FA0 => "proc".into(),
        0x6165_676C => "pstorefs".into(),
        0x002F => "qnx4".into(),
        0x6819_1122 => "qnx6".into(),
        0x8584_58F6 => "ramfs".into(),
        0x5265_4973 => "reiserfs".into(),
        0x7275 => "romfs".into(),
        0x6759_6969 => "rpc_pipefs".into(),
        0x7363_6673 => "securityfs".into(),
        0xF97C_FF8C => "selinux".into(),
        0x4341_5D53 => "smackfs".into(),
        0x517B => "smb".into(),
        0xFE53_4D42 => "smb2".into(),
        0xBEEF_DEAD => "snfs".into(),
        0x534F_434B => "sockfs".into(),
        0x7371_7368 => "squashfs".into(),
        0x6265_6572 => "sysfs".into(),
        0x012F_F7B6 => "sysv2".into(),
        0x012F_F7B5 => "sysv4".into(),
        0x0102_1994 => "tmpfs".into(),
        0x7472_6163 => "tracefs".into(),
        0x2405_1905 => "ubifs".into(),
        0x1501_3346 => "udf".into(),
        0x0001_1954 => "ufs".into(),
        0x5419_0100 => "ufs".into(),
        0x9FA2 => "usbdevfs".into(),
        0x0102_1997 => "v9fs".into(),
        0xBACB_ACBC => "vmhgfs".into(),
        0xA501_FCF5 => "vxfs".into(),
        0x565A_4653 => "vzfs".into(),
        0x5346_4846 => "wslfs".into(),
        0xABBA_1974 => "xenfs".into(),
        0x012F_F7B4 => "xenix".into(),
        0x5846_5342 => "xfs".into(),
        0x012F_D16D => "xia".into(),
        0x2FC1_2FC1 => "zfs".into(),
        0xDE => "zfs".into(),
        other => format!("UNKNOWN ({other:#x})").into(),
    }
    // 启用拼写检查
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statfs_success() {
        // 准备测试数据
        let path = "/";

        // Execute the function
        let result = statfs(path);

        // Assert the result
        assert!(result.is_ok());
        let _buffer = result.unwrap();
        // Add specific assertions for the `StatFs` struct fields
        // ...
    }

    #[test]
    fn test_statfs_error() {
        // Prepare test data
        let path = "/nonexistent";

        // Execute the function
        let result = statfs(path);

        // Assert the result
        assert!(result.is_err());
        let _err_msg = result.err().unwrap();
        // Add specific assertions for the error message
        // ...
    }

    #[test]
    #[cfg(unix)]
    fn test_file_type() {
        assert_eq!("block special file", pretty_filetype(S_IFBLK, 0));
        assert_eq!("character special file", pretty_filetype(S_IFCHR, 0));
        assert_eq!("regular file", pretty_filetype(S_IFREG, 1));
        assert_eq!("regular empty file", pretty_filetype(S_IFREG, 0));
        assert_eq!("weird file", pretty_filetype(0, 0));
    }

    #[test]
    fn test_fs_type() {
        // spell-checker:disable
        assert_eq!("ext2/ext3", pretty_fstype(0xEF53));
        assert_eq!("tmpfs", pretty_fstype(0x01021994));
        assert_eq!("nfs", pretty_fstype(0x6969));
        assert_eq!("btrfs", pretty_fstype(0x9123683e));
        assert_eq!("xfs", pretty_fstype(0x58465342));
        assert_eq!("zfs", pretty_fstype(0x2FC12FC1));
        assert_eq!("ntfs", pretty_fstype(0x5346544e));
        assert_eq!("fat", pretty_fstype(0x4006));
        assert_eq!("UNKNOWN (0x1234)", pretty_fstype(0x1234));
        // spell-checker:enable
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_mountinfo() {
        // spell-checker:ignore (word) relatime
        let info = CtMountInfo::new(
            CT_LINUX_MOUNTINFO,
            &"106 109 253:6 / /mnt rw,relatime - xfs /dev/fs0 rw"
                .split_ascii_whitespace()
                .collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(info.mount_root, "/");
        assert_eq!(info.mount_dir, "/mnt");
        assert_eq!(info.mount_option, "rw,relatime");
        assert_eq!(info.fs_type, "xfs");
        assert_eq!(info.dev_name, "/dev/fs0");

        // Test parsing with different amounts of optional fields.
        let info = CtMountInfo::new(
            CT_LINUX_MOUNTINFO,
            &"106 109 253:6 / /mnt rw,relatime master:1 - xfs /dev/fs0 rw"
                .split_ascii_whitespace()
                .collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(info.fs_type, "xfs");
        assert_eq!(info.dev_name, "/dev/fs0");

        let info = CtMountInfo::new(
            CT_LINUX_MOUNTINFO,
            &"106 109 253:6 / /mnt rw,relatime master:1 shared:2 - xfs /dev/fs0 rw"
                .split_ascii_whitespace()
                .collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(info.fs_type, "xfs");
        assert_eq!(info.dev_name, "/dev/fs0");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_mountinfo_preserves_non_utf8_fields() {
        let raw = [
            b"106".as_slice(),
            b"109",
            b"253:6",
            b"/",
            b"/mnt/m\xf3unt",
            b"rw,relatime",
            b"-",
            b"xfs",
            b"/dev/fs\xf3",
            b"rw",
        ];
        let info = CtMountInfo::new_bytes(CT_LINUX_MOUNTINFO, &raw).unwrap();

        assert_eq!(mount_field_to_bytes(&info.mount_dir), b"/mnt/m\xf3unt");
        assert_eq!(mount_field_to_bytes(&info.dev_name), b"/dev/fs\xf3");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_mountinfo_unescapes_octal_fields() {
        let raw = [
            b"106".as_slice(),
            b"109",
            b"253:6",
            b"/",
            br"/mnt/m\040nt\011tab\012line\134slash",
            b"rw,relatime",
            b"-",
            b"xfs",
            b"/dev/fs0",
            b"rw",
        ];
        let info = CtMountInfo::new_bytes(CT_LINUX_MOUNTINFO, &raw).unwrap();

        assert_eq!(
            mount_field_to_bytes(&info.mount_dir),
            b"/mnt/m nt\ttab\nline\\slash"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_mount_field_encoding_preserves_private_use_marker() {
        let input = format!("/mnt/{CT_MOUNT_RAW_BYTE_MARKER}/name");
        let encoded = mount_field_from_bytes(input.as_bytes());

        assert_eq!(mount_field_to_bytes(&encoded), input.as_bytes());
    }

    #[test]
    fn test_read_fs_list_linux() {
        //  Linux OS 测试用例
        #[cfg(target_os = "linux")]
        {
            // Arrange
            let _expected = [CtMountInfo::new(
                CT_LINUX_MOUNTINFO,
                &"106 109 253:6 / /mnt rw,relatime - xfs /dev/fs0 rw"
                    .split_ascii_whitespace()
                    .collect::<Vec<_>>(),
            )];

            // Act
            let _result = read_fs_list();

            // Assert
            //assert_eq!(result.unwrap(), expected);
        }
    }
    #[test]
    fn test_read_fs_list_windows() {
        // Test case for Windows OS
        #[cfg(target_os = "windows")]
        {
            // Arrange
            let expected = vec![
                CtMountInfo {
                    // Define expected MountInfo struct values
                },
                // Add more expected MountInfo struct values
            ];

            // Act
            let result = read_fs_list();

            // Assert
            assert_eq!(result.unwrap(), expected);
        }
    }
}
