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

extern crate rust_i18n;
use ctcore::ct_display::Quotable;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
pub use ctcore::ct_entries::{self, CtPasswd, Group, Locate};
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_perms::{CtGidUidOwnerFilter, CtIfFrom, chown_base, opt_flags};
use sys_locale::get_locale;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;


pub fn chown_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    chown_base(
        ct_app(),
        args,
        opt_flags::ARG_OWNER,
        chown_parsing_gid_uid_and_filter,
        false,
    )
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("chown.about");
    let usage_description = t!("chown.usage");

    let args = vec![
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help(t!("chown.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("chown.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::verbosity::CHANGES)
            .short('c')
            .long(opt_flags::verbosity::CHANGES)
            .help(t!("chown.clap.changes"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::DEREFERENCE)
            .long(opt_flags::dereference::DEREFERENCE)
            .help(
                "affect the referent of each symbolic link (this is the default), \
                     rather than the symbolic link itself",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::NO_DEREFERENCE)
            .short('h')
            .long(opt_flags::dereference::NO_DEREFERENCE)
            .help(
                "affect symbolic links instead of any referenced file \
                     (useful only on systems that can change the ownership of a symlink)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FROM)
            .long(opt_flags::FROM)
            .help(
                "change the owner and/or group of each file only if its \
                     current owner and/or group match those specified here. \
                     Either may be omitted, in which case a match is not required \
                     for the omitted attribute",
            )
            .value_name("CURRENT_OWNER:CURRENT_GROUP"),
        Arg::new(opt_flags::preserve_root::PRESERVE)
            .long(opt_flags::preserve_root::PRESERVE)
            .help(t!("chown.clap.preserve"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::NO_PRESERVE)
            .long(opt_flags::preserve_root::NO_PRESERVE)
            .help(t!("chown.clap.no_preserve"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::verbosity::QUIET)
            .long(opt_flags::verbosity::QUIET)
            .help(t!("chown.clap.quiet"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::RECURSIVE)
            .short('R')
            .long(opt_flags::RECURSIVE)
            .help(t!("chown.clap.recursive"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::REFERENCE)
            .long(opt_flags::REFERENCE)
            .help(t!("chown.clap.reference"))
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath)
            .num_args(1..),
        Arg::new(opt_flags::verbosity::SILENT)
            .short('f')
            .long(opt_flags::verbosity::SILENT)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::TRAVERSE)
            .short(opt_flags::traverse::TRAVERSE.chars().next().unwrap())
            .help("if a command line argument is a symbolic link to a directory, traverse it")
            .overrides_with_all([opt_flags::traverse::EVERY, opt_flags::traverse::NO_TRAVERSE])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::EVERY)
            .short(opt_flags::traverse::EVERY.chars().next().unwrap())
            .help("traverse every symbolic link to a directory encountered")
            .overrides_with_all([
                opt_flags::traverse::TRAVERSE,
                opt_flags::traverse::NO_TRAVERSE,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::NO_TRAVERSE)
            .short(opt_flags::traverse::NO_TRAVERSE.chars().next().unwrap())
            .help("do not traverse any symbolic links (default)")
            .overrides_with_all([opt_flags::traverse::TRAVERSE, opt_flags::traverse::EVERY])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::verbosity::VERBOSE)
            .long(opt_flags::verbosity::VERBOSE)
            .short('v')
            .help(t!("chown.clap.verbose"))
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

/**
 * 根据命令行参数解析目标用户ID（UID）和组ID（GID），以及应用这些变更的条件过滤器。
 *
 * 该函数主要用于处理与文件所有者（用户和组）相关的命令行参数，并根据这些参数构建一个
 */
fn chown_parsing_gid_uid_and_filter(args_match: &ArgMatches) -> CTResult<CtGidUidOwnerFilter> {
    // 解析 `-from` 参数，确定UID和GID的变更条件。
    let filter_info = if let Some(spec) = args_match.get_one::<String>(opt_flags::FROM) {
        match chown_parse_spec(spec, ':')? {
            (Some(uid), None) => CtIfFrom::User(uid),
            (None, Some(gid)) => CtIfFrom::Group(gid),
            (Some(uid), Some(gid)) => CtIfFrom::UserGroup(uid, gid),
            (None, None) => CtIfFrom::All,
        }
    } else {
        CtIfFrom::All
    };

    // 定义用于存储目标UID和GID的变量，以及原始所有者信息。
    let dest_uid: Option<u32>;
    let dest_gid: Option<u32>;
    let raw_owner: String;
    // 处理 `-reference` 参数，若存在，则从指定文件获取UID和GID。
    if let Some(file) = args_match.get_one::<String>(opt_flags::REFERENCE) {
        let meta = fs::metadata(file)
            .map_err_context(|| format!("failed to get attributes of {}", file.quote()))?;
        let gid = meta.gid();
        let uid = meta.uid();
        dest_gid = Some(gid);
        dest_uid = Some(uid);
        // 格式化文件的所有者信息（用户:组）。
        raw_owner = format!(
            "{}:{}",
            ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string())
        );
    } else {
        // 如果没有指定 `-reference` 参数，则从 `-owner` 参数中解析UID和GID。
        raw_owner = args_match
            .get_one::<String>(opt_flags::ARG_OWNER)
            .unwrap()
            .into();
        let (u, g) = chown_parse_spec(&raw_owner, ':')?;
        dest_uid = u;
        dest_gid = g;
    }
    // 构建并返回 `CtGidUidOwnerFilter` 结构体。
    Ok(CtGidUidOwnerFilter {
        dest_gid,
        dest_uid,
        raw_owner,
        filter: filter_info,
    })
}

/// Parses the user string to extract the UID.
/**
 * 解析给定的用户标识符，尝试获取其UID。
 *
 * @param user 指定的用户名称或数字用户ID，为字符串格式。
 * @param spec 特定的规格字符串，可能包含用户组信息。
 * @param sep 规格字符串中用于分隔用户和组的字符。
 * @return 返回一个结果选项，可能包含解析出的u32类型的UID，或者在无法解析时返回None。
 */
fn chown_parse_uid(username: &str, spec_info: &str, sep: char) -> CTResult<Option<u32>> {
    // 如果用户名称为空，则直接返回None
    if username.is_empty() {
        return Ok(None);
    }
    // 尝试根据提供的用户名称定位用户信息
    match CtPasswd::locate(username) {
        Ok(u) => Ok(Some(u.uid)), // 成功找到用户，返回其UID
        Err(_) => {
            // 未能找到用户，考虑其他解析方法
            // 检查spec字符串是否包含'.'但不包含':'，尝试以用户名.组名的方式解析
            if spec_info.contains('.') && !spec_info.contains(':') && sep == ':' {
                chown_parse_spec(spec_info, '.').map(|(uid_str, _)| uid_str) // 尝试解析规格字符串为UID
            } else {
                // 如果'user'字符串包含数字，尝试将其解析为UID
                match username.parse() {
                    Ok(uid_num) => Ok(Some(uid_num)), // 成功解析为数字UID
                    Err(_) => Err(CtSimpleError::new(
                        1,
                        format!("invalid user: {}", spec_info.quote()),
                    )), // 解析失败，返回错误
                }
            }
        }
    }
}

/// Parses the group string to extract the GID.
/**
 * 解析指定的组ID。
 *
 * 此函数尝试根据提供的组名 `group` 来查找对应的组ID。首先，它会尝试直接定位该组，如果失败，则尝试将组名解析为u32类型的ID。
 * 如果组名为空，则直接返回 `None`。
 *
 * @param group 指定的组名，为字符串格式。
 * @param spec 附加的规范信息，用于错误消息中，非直接参数。
 * @return `CTResult<Option<u32>>`。成功时返回组ID的 `Option` 包装（可能为 `None`），失败时返回错误信息。
 */
fn chown_parse_gid(chown_group: &str, spec_str: &str) -> CTResult<Option<u32>> {
    // 如果组名为空，则直接返回None
    if chown_group.is_empty() {
        return Ok(None);
    }
    // 尝试根据组名定位组，成功则返回组的gid，失败则尝试将组名解析为u32
    match Group::locate(chown_group) {
        Ok(g) => Ok(Some(g.gid)), // 成功定位组，返回组的gid
        Err(_) => match chown_group.parse() {
            Ok(gid) => Ok(Some(gid)), // 成功将组名解析为u32，返回解析后的gid
            Err(_) => Err(CtSimpleError::new(
                1,                                              // 错误码
                format!("invalid group: {}", spec_str.quote()), // 构造错误消息
            )),
        },
    }
}

/// Parse the owner/group specifier string into a user ID and a group ID.
///
/// The `spec` can be of the form:
///
/// * `"owner:group"`,
/// * `"owner"`,
/// * `":group"`,
///
/// and the owner or group can be specified either as an ID or a
/// name. The `sep` argument specifies which character to use as a
/// separator between the owner and group; calling code should set
/// this to `':'`.
/**
 * 解析指定格式的字符串来获取用户ID和组ID。
 *
 * 此函数按照指定的分隔符将输入字符串分割为用户和组两部分，并尝试将它们解析为对应的ID。
 * 如果用户或组部分可以成功解析为u32，则返回相应的Option值；如果无法解析或遇到错误，则返回错误结果。
 *
 * @param spec 待解析的字符串，格式为"用户[分隔符组]"，其中分隔符由sep参数指定。
 * @param sep 用于分割用户和组的字符，只能是点('.')或冒号(':')。
 * @return 返回一个CTResult，其中包含解析出的用户ID和组ID的Option值。如果解析成功，则用户ID和组ID以(Some(_), Some(_))的形式返回；如果解析失败，则返回错误信息。
 */
fn chown_parse_spec(spec_str: &str, sep: char) -> CTResult<(Option<u32>, Option<u32>)> {
    // 确保分隔符是有效的
    assert!(['.', ':'].contains(&sep));
    // 使用指定的分隔符分割字符串为用户和组两部分
    let mut argments = spec_str.splitn(2, sep);
    let username = argments.next().unwrap_or("");
    let group_info = argments.next().unwrap_or("");

    // 尝试解析用户和组部分为ID
    let uid_value = chown_parse_uid(username, spec_str, sep)?;
    let gid_value = chown_parse_gid(group_info, spec_str)?;

    // 检查特殊情况：如果用户ID是以数字开头且未指定组ID，但提供了分隔符，则视为错误
    if username
        .chars()
        .next()
        .map(char::is_numeric)
        .unwrap_or(false)
        && group_info.is_empty()
        && spec_str != username
    {
        return Err(CtSimpleError::new(
            1,
            format!("invalid spec: {}", spec_str.quote()),
        ));
    }

    // 返回解析结果
    Ok((uid_value, gid_value))
}

#[derive(Default)]
pub struct Chown;
impl Tool for Chown {
    fn name(&self) -> &'static str {
        "chown"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        chown_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::fs;
    use std::fs::File;

    use std::io::Write;

    #[test]
    fn test_parse_uid() {
        assert!(matches!(chown_parse_uid("", "", ':'), Ok(None)));
        assert!(matches!(chown_parse_uid("", ":", ':'), Ok(None)));
        assert!(matches!(chown_parse_uid("", ".", '.'), Ok(None)));
        assert!(matches!(chown_parse_uid("", ".", ':'), Ok(None)));
        assert!(matches!(chown_parse_uid("", ":", '.'), Ok(None)));
    }
    #[test]
    fn test_parse_gid() {
        assert!(matches!(chown_parse_gid("", ""), Ok(None)));
        assert!(matches!(chown_parse_gid("", ":"), Ok(None)));
        assert!(matches!(chown_parse_gid("", "."), Ok(None)));
    }
    #[test]
    fn test_parse_spec_with_dot() {
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));

        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));

        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot_or_colon_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot_or_colon_or_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot_or_colon_or_dot_or_colon_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_or_dot_or_colon_or_dot_or_colon_or_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_test_olon_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_test_or_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_ctest_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colof_test_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_dot_or_colon_sft_colon_or_dot() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec_with_test_or_colon_or_dot_or_colon() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
    }
    #[test]
    fn test_parse_spec() {
        assert!(matches!(chown_parse_spec(":", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", ':'), Ok((None, None))));
        assert!(matches!(chown_parse_spec(".", '.'), Ok((None, None))));
        assert!(
            format!("{}", chown_parse_spec("::", ':').err().unwrap())
                .starts_with("invalid group: ")
        );
        assert!(
            format!("{}", chown_parse_spec("..", ':').err().unwrap())
                .starts_with("invalid group: ")
        );
    }

    /// Test for parsing IDs that don't correspond to a named user or group.
    #[test]
    fn test_parse_spec_nameless_ids() {
        // This assumes that there is no named user with ID 12345.
        assert!(matches!(
            chown_parse_spec("12345", ':'),
            Ok((Some(12345), None))
        ));
        // This assumes that there is no named group with ID 54321.
        assert!(matches!(
            chown_parse_spec(":54321", ':'),
            Ok((None, Some(54321)))
        ));
        assert!(matches!(
            chown_parse_spec("12345:54321", ':'),
            Ok((Some(12345), Some(54321)))
        ));
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--help"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--version"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_version_valid() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "-V"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_dereference_true() {
        let command = ct_app();

        // 测试用例：有效输入 --dereference
        let args = vec![ctcore::ct_util_name(), "--dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "-h"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_whole_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "--no-dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_true() {
        let command = ct_app();

        // 测试用例：有效输入 --preserve-root
        let args = vec![ctcore::ct_util_name(), "--preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::PRESERVE));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-preserve-root
        let args = vec![ctcore::ct_util_name(), "--no-preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::NO_PRESERVE));
        assert!(!matches.get_flag(opt_flags::preserve_root::PRESERVE));
    }

    #[test]
    fn test_ct_app_execution_recursive() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "-R"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    #[test]
    fn test_ct_app_execution_recursive_whole() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "--recursive"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    #[test]
    fn test_version_ctmain() {
        let args = vec![ctcore::ct_util_name(), "--version"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_help_ctmain() {
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_help_invalid_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-H"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_version_valid_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-V"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_dereference_true_ctmain() {
        let args = vec![ctcore::ct_util_name(), "--dereference"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_dereference_false_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-h"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_dereference_whole_false_ctmain() {
        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "--no-dereference"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_preserve_root_true_ctmain() {
        // 测试用例：有效输入 --preserve-root
        let args = vec![ctcore::ct_util_name(), "--preserve-root"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_preserve_root_false_ctmain() {
        // 测试用例：有效输入 --no-preserve-root
        let args = vec![ctcore::ct_util_name(), "--no-preserve-root"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_recursive_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "-R"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_recursive_whole_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "--recursive"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "-v"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_whole_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "--verbose"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_chgrp_r_ctmain() {
        let dir_path = "test_chgrp_r_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chgrp_r_ctmain_w.txt";

        // 创建二级目录
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{}/{}", subdir_path, file_name);

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![ctcore::ct_util_name(), "-R", "1000", dir_path];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
        // 删除目录及其内容
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }

    #[test]
    fn test_chgrp_single_file_ctmain() {
        let file_name = "test_chcon_single_file.txt";
        let file_path = file_name.to_owned();

        // Create a file and write content
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![ctcore::ct_util_name(), "1000", file_name];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));

        // Remove the file
        fs::remove_file(file_path).expect("Failed to delete file");
        assert_eq!(result, 0);
    }

    #[test]
    fn test_chgrp_recursive_ctmain() {
        let dir_path = "test_chgrp__recursive_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chgrp_no_recursive_ctmain.txt";

        // Create a directory hierarchy
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // Create a file in the subdirectory and write content
        let file_path = format!("{}/{}", subdir_path, file_name);
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![ctcore::ct_util_name(), "--recursive", "1000", dir_path];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }

    #[test]
    fn test_chgrp_invalid_user_id_ctmain() {
        let dir_path = "test_chgrp_invalid_user_id_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{}/{}", subdir_path, file_name);

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![ctcore::ct_util_name(), "-R", "invalid_user_id", dir_path];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_ne!(result, 0); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }
    #[test]
    fn test_chgrp_h_r_ctmain() {
        let dir_path = "test_chgrp_h_r_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{}/{}", subdir_path, file_name);

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![
            ctcore::ct_util_name(),
            "-h",
            "-R",
            "invalid_user_id",
            dir_path,
        ];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_ne!(result, 0); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }

    // 新增：测试 Tool trait 的基本实现
    #[test]
    fn test_tool_implementation() {
        let tool = Chown::default();

        // Test name method
        assert_eq!(tool.name(), "chown");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("chown"));

        // Test execute method with no arguments
        let args = vec![OsString::from("chown")];
        let result = tool.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 1);

        // Test execute method with help flag
        let args = vec![OsString::from("chown"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);

        // Test execute method with version flag
        let args = vec![OsString::from("chown"), OsString::from("--version")];
        let result = tool.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);
    }
}
