/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use std::ffi::OsString;
use std::path::Path;

use clap::builder::{NonEmptyStringValueParser, ValueParser};
use clap::{crate_version, Arg, ArgAction, Command};

use ct_ls::{LsConfig, LsFormat, PathData};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
//use ctcore::ct_error::UError;
use ctcore::ct_error::CTResult;
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle};

const VDIR_ABOUT: &str = ct_help_about!("vdir.md");
const VDIR_AFTER_HELP: &str = ct_help_section!("after help", "vdir.md");
const VDIR_USAGE: &str = ct_help_usage!("vdir.md");
#[cfg(not(feature = "selinux"))]
static VDIR_CONTEXT_HELP_TEXT: &str = "print any security context of each file (not enabled)";
#[cfg(feature = "selinux")]
static VDIR_CONTEXT_HELP_TEXT: &str = "print any security context of each file";

pub mod vdir_flags {
    pub mod format {
        pub static VDIR_ONE_LINE: &str = "1";
        pub static VDIR_LONG: &str = "long";
        pub static VDIR_COLUMNS: &str = "C";
        pub static VDIR_ACROSS: &str = "x";
        pub static VDIR_TAB_SIZE: &str = "tabsize";
        pub static VDIR_COMMAS: &str = "m";
        pub static VDIR_LONG_NO_OWNER: &str = "g";
        pub static VDIR_LONG_NO_GROUP: &str = "o";
        pub static VDIR_LONG_NUMERIC_UID_GID: &str = "numeric-uid-gid";
    }

    pub mod files {
        pub static VDIR_ALL: &str = "all";
        pub static VDIR_ALMOST_ALL: &str = "almost-all";
    }

    pub mod sort {
        pub static VDIR_SIZE: &str = "S";
        pub static VDIR_TIME: &str = "t";
        pub static VDIR_NONE: &str = "U";
        pub static VDIR_VERSION: &str = "v";
        pub static VDIR_EXTENSION: &str = "X";
    }

    pub mod time {
        pub static VDIR_ACCESS: &str = "u";
        pub static VDIR_CHANGE: &str = "c";
    }

    pub mod size {
        pub static VDIR_ALLOCATION_SIZE: &str = "size";
        pub static VDIR_BLOCK_SIZE: &str = "block-size";
        pub static VDIR_HUMAN_READABLE: &str = "human-readable";
        pub static VDIR_SI: &str = "si";
        pub static VDIR_KIBIBYTES: &str = "kibibytes";
    }

    pub mod quoting {
        pub static VDIR_ESCAPE: &str = "escape";
        pub static VDIR_LITERAL: &str = "literal";
        pub static VDIR_C: &str = "quote-name";
    }

    pub mod indicator_style {
        pub static VDIR_SLASH: &str = "p";
        pub static VDIR_FILE_TYPE: &str = "file-type";
        pub static VDIR_CLASSIFY: &str = "classify";
    }

    pub mod dereference {
        pub static VDIR_ALL: &str = "dereference";
        pub static VDIR_ARGS: &str = "dereference-command-line";
        pub static VDIR_DIR_ARGS: &str = "dereference-command-line-symlink-to-dir";
    }

    pub static VDIR_HELP: &str = "help";
    pub static VDIR_QUOTING_STYLE: &str = "quoting-style";
    pub static VDIR_HIDE_CONTROL_CHARS: &str = "hide-control-chars";
    pub static VDIR_SHOW_CONTROL_CHARS: &str = "show-control-chars";
    pub static VDIR_WIDTH: &str = "width";
    pub static VDIR_AUTHOR: &str = "author";
    pub static VDIR_NO_GROUP: &str = "no-group";
    pub static VDIR_FORMAT: &str = "format";
    pub static VDIR_SORT: &str = "sort";
    pub static VDIR_TIME: &str = "time";
    pub static VDIR_IGNORE_BACKUPS: &str = "ignore-backups";
    pub static VDIR_DIRECTORY: &str = "directory";
    pub static VDIR_INODE: &str = "inode";
    pub static VDIR_REVERSE: &str = "reverse";
    pub static VDIR_RECURSIVE: &str = "recursive";
    pub static VDIR_COLOR: &str = "color";
    pub static VDIR_PATHS: &str = "paths";
    pub static VDIR_INDICATOR_STYLE: &str = "indicator-style";
    pub static VDIR_TIME_STYLE: &str = "time-style";
    pub static VDIR_FULL_TIME: &str = "full-time";
    pub static VDIR_HIDE: &str = "hide";
    pub static VDIR_IGNORE: &str = "ignore";
    pub static VDIR_CONTEXT: &str = "context";
    pub static VDIR_GROUP_DIRECTORIES_FIRST: &str = "group-directories-first";
    pub static VDIR_ZERO: &str = "zero";
    pub static VDIR_DIRED: &str = "dired";
    pub static VDIR_HYPERLINK: &str = "hyperlink";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    vdir_main(args).map(|_| ())
}

pub fn vdir_main(args: impl ctcore::Args) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
    let command = ct_app();
    let matches = command.get_matches_from(args);

    let mut default_quoting_style = false;
    let mut default_format_style = false;

    // 我们会检查是否给出了格式化或引号样式标志。
    // 如果没有，我们将使用 dir 默认的格式化和引用样式标志
    if !matches.contains_id(vdir_flags::VDIR_QUOTING_STYLE)
        && !matches.get_flag(vdir_flags::quoting::VDIR_C)
        && !matches.get_flag(vdir_flags::quoting::VDIR_ESCAPE)
        && !matches.get_flag(vdir_flags::quoting::VDIR_LITERAL)
    {
        default_quoting_style = true;
    }
    if !matches.contains_id(vdir_flags::VDIR_FORMAT)
        && !matches.get_flag(vdir_flags::format::VDIR_ACROSS)
        && !matches.get_flag(vdir_flags::format::VDIR_COLUMNS)
        && !matches.get_flag(vdir_flags::format::VDIR_COMMAS)
        && !matches.get_flag(vdir_flags::format::VDIR_LONG)
        && !matches.get_flag(vdir_flags::format::VDIR_LONG_NO_GROUP)
        && !matches.get_flag(vdir_flags::format::VDIR_LONG_NO_OWNER)
        && !matches.get_flag(vdir_flags::format::VDIR_LONG_NUMERIC_UID_GID)
        && !matches.get_flag(vdir_flags::format::VDIR_ONE_LINE)
    {
        default_format_style = true;
    }

    let mut config = LsConfig::from(&matches)?;

    if default_quoting_style {
        config.quoting_style = CtQuotingStyle::C {
            quotes: CtQuotes::None,
        };
    }
    if default_format_style {
        config.format = LsFormat::Long;
    }

    let paths_list = matches.get_many::<OsString>(vdir_flags::VDIR_PATHS);
    let paths_from_args: Vec<_> = paths_list
        .map(|v| v.map(Path::new).collect())
        .unwrap_or_else(|| vec![Path::new(".")]);

    ct_ls::list(paths_from_args, &config)
}

// 实现逻辑和ls一致
pub fn ct_app() -> Command {
    // ct_ls::ct_app()
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = VDIR_ABOUT;
    let usage_description = ct_format_usage(VDIR_USAGE);

    let args = vec![
         Arg::new(vdir_flags::VDIR_HELP)
             .long(vdir_flags::VDIR_HELP)
             .help("Print help information.")
             .action(ArgAction::Help),
         Arg::new(vdir_flags::VDIR_FORMAT)
             .long(vdir_flags::VDIR_FORMAT)
             .help("Set the display format.")
             .value_parser([
                 "long",
                 "verbose",
                 "single-column",
                 "columns",
                 "vertical",
                 "across",
                 "horizontal",
                 "commas",
             ])
             .hide_possible_values(true)
             .require_equals(true)
             .overrides_with_all([
                 vdir_flags::VDIR_FORMAT,
                 vdir_flags::format::VDIR_COLUMNS,
                 vdir_flags::format::VDIR_LONG,
                 vdir_flags::format::VDIR_ACROSS,
                 vdir_flags::format::VDIR_COLUMNS,
             ]),
         Arg::new(vdir_flags::format::VDIR_COLUMNS)
             .short('C')
             .help("Display the files in columns.")
             .overrides_with_all([
                 vdir_flags::VDIR_FORMAT,
                 vdir_flags::format::VDIR_COLUMNS,
                 vdir_flags::format::VDIR_LONG,
                 vdir_flags::format::VDIR_ACROSS,
                 vdir_flags::format::VDIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_LONG)
             .short('l')
             .long(vdir_flags::format::VDIR_LONG)
             .help("Display detailed information.")
             .overrides_with_all([
                 vdir_flags::VDIR_FORMAT,
                 vdir_flags::format::VDIR_COLUMNS,
                 vdir_flags::format::VDIR_LONG,
                 vdir_flags::format::VDIR_ACROSS,
                 vdir_flags::format::VDIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_ACROSS)
             .short('x')
             .help("List entries in rows instead of in columns.")
             .overrides_with_all([
                 vdir_flags::VDIR_FORMAT,
                 vdir_flags::format::VDIR_COLUMNS,
                 vdir_flags::format::VDIR_LONG,
                 vdir_flags::format::VDIR_ACROSS,
                 vdir_flags::format::VDIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_TAB_SIZE)
             .short('T')
             .long(vdir_flags::format::VDIR_TAB_SIZE)
             .env("TABSIZE")
             .value_name("COLS")
             .help("Assume tab stops at each COLS instead of 8 (unimplemented)"),
         Arg::new(vdir_flags::format::VDIR_COMMAS)
             .short('m')
             .help("List entries separated by commas.")
             .overrides_with_all([
                 vdir_flags::VDIR_FORMAT,
                 vdir_flags::format::VDIR_COLUMNS,
                 vdir_flags::format::VDIR_LONG,
                 vdir_flags::format::VDIR_ACROSS,
                 vdir_flags::format::VDIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_ZERO)
             .long(vdir_flags::VDIR_ZERO)
             .overrides_with(vdir_flags::VDIR_ZERO)
             .help("List entries separated by ASCII NUL characters.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_DIRED)
             .long(vdir_flags::VDIR_DIRED)
             .short('D')
             .help("generate output designed for Emacs' dired (Directory Editor) mode")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_HYPERLINK)
             .long(vdir_flags::VDIR_HYPERLINK)
             .help("hyperlink file names WHEN")
             .value_parser([
                 "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
             ])
             .require_equals(true)
             .num_args(0..=1)
             .default_missing_value("always")
             .default_value("never")
             .value_name("WHEN"),
         Arg::new(vdir_flags::format::VDIR_ONE_LINE)
             .short('1')
             .help("List one file per line.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_LONG_NO_GROUP)
             .short('o')
             .help(
                 "Long format without group information. \
                         Identical to --format=long with --no-group.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_LONG_NO_OWNER)
             .short('g')
             .help("Long format without owner information.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::format::VDIR_LONG_NUMERIC_UID_GID)
             .short('n')
             .long(vdir_flags::format::VDIR_LONG_NUMERIC_UID_GID)
             .help("-l with numeric UIDs and GIDs.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_QUOTING_STYLE)
             .long(vdir_flags::VDIR_QUOTING_STYLE)
             .help("Set quoting style.")
             .value_parser([
                 "literal",
                 "shell",
                 "shell-always",
                 "shell-escape",
                 "shell-escape-always",
                 "c",
                 "escape",
             ])
             .overrides_with_all([
                 vdir_flags::VDIR_QUOTING_STYLE,
                 vdir_flags::quoting::VDIR_LITERAL,
                 vdir_flags::quoting::VDIR_ESCAPE,
                 vdir_flags::quoting::VDIR_C,
             ]),
         Arg::new(vdir_flags::quoting::VDIR_LITERAL)
             .short('N')
             .long(vdir_flags::quoting::VDIR_LITERAL)
             .alias("l")
             .help("Use literal quoting style. Equivalent to `--quoting-style=literal`")
             .overrides_with_all([
                 vdir_flags::VDIR_QUOTING_STYLE,
                 vdir_flags::quoting::VDIR_LITERAL,
                 vdir_flags::quoting::VDIR_ESCAPE,
                 vdir_flags::quoting::VDIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::quoting::VDIR_ESCAPE)
             .short('b')
             .long(vdir_flags::quoting::VDIR_ESCAPE)
             .help("Use escape quoting style. Equivalent to `--quoting-style=escape`")
             .overrides_with_all([
                 vdir_flags::VDIR_QUOTING_STYLE,
                 vdir_flags::quoting::VDIR_LITERAL,
                 vdir_flags::quoting::VDIR_ESCAPE,
                 vdir_flags::quoting::VDIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::quoting::VDIR_C)
             .short('Q')
             .long(vdir_flags::quoting::VDIR_C)
             .help("Use VDIR_C quoting style. Equivalent to `--quoting-style=c`")
             .overrides_with_all([
                 vdir_flags::VDIR_QUOTING_STYLE,
                 vdir_flags::quoting::VDIR_LITERAL,
                 vdir_flags::quoting::VDIR_ESCAPE,
                 vdir_flags::quoting::VDIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_HIDE_CONTROL_CHARS)
             .short('q')
             .long(vdir_flags::VDIR_HIDE_CONTROL_CHARS)
             .help("Replace control characters with '?' if they are not escaped.")
             .overrides_with_all([vdir_flags::VDIR_HIDE_CONTROL_CHARS, vdir_flags::VDIR_SHOW_CONTROL_CHARS])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_SHOW_CONTROL_CHARS)
             .long(vdir_flags::VDIR_SHOW_CONTROL_CHARS)
             .help("Show control characters 'as is' if they are not escaped.")
             .overrides_with_all([vdir_flags::VDIR_HIDE_CONTROL_CHARS, vdir_flags::VDIR_SHOW_CONTROL_CHARS])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_TIME)
             .long(vdir_flags::VDIR_TIME)
             .help(
                 "Show time in <field>:\n\
                         \taccess time (-u): atime, access, use;\n\
                         \tchange time (-t): ctime, status.\n\
                         \tbirth time: birth, creation;",
             )
             .value_name("field")
             .value_parser([
                 "atime", "access", "use", "ctime", "status", "birth", "creation",
             ])
             .hide_possible_values(true)
             .require_equals(true)
             .overrides_with_all([vdir_flags::VDIR_TIME, vdir_flags::time::VDIR_ACCESS, vdir_flags::time::VDIR_CHANGE]),
         Arg::new(vdir_flags::time::VDIR_CHANGE)
             .short('c')
             .help(
                 "If the long listing format (e.g., -l, -o) is being used, print the \
                         status change time (the 'ctime' in the inode) instead of the modification \
                         time. When explicitly sorting by time (--sort=time or -t) or when not \
                         using a long listing format, sort according to the status change time.",
             )
             .overrides_with_all([vdir_flags::VDIR_TIME, vdir_flags::time::VDIR_ACCESS, vdir_flags::time::VDIR_CHANGE])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::time::VDIR_ACCESS)
             .short('u')
             .help(
                 "If the long listing format (e.g., -l, -o) is being used, print the \
                         status access time instead of the modification time. When explicitly \
                         sorting by time (--sort=time or -t) or when not using a long listing \
                         format, sort according to the access time.",
             )
             .overrides_with_all([vdir_flags::VDIR_TIME, vdir_flags::time::VDIR_ACCESS, vdir_flags::time::VDIR_CHANGE])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_HIDE)
             .long(vdir_flags::VDIR_HIDE)
             .action(ArgAction::Append)
             .value_name("PATTERN")
             .help(
                 "do not list implied entries matching shell PATTERN (overridden by -a or -A)",
             ),
         Arg::new(vdir_flags::VDIR_IGNORE)
             .short('I')
             .long(vdir_flags::VDIR_IGNORE)
             .action(ArgAction::Append)
             .value_name("PATTERN")
             .help("do not list implied entries matching shell PATTERN"),
         Arg::new(vdir_flags::VDIR_IGNORE_BACKUPS)
             .short('B')
             .long(vdir_flags::VDIR_IGNORE_BACKUPS)
             .help("Ignore entries which end with ~.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_SORT)
             .long(vdir_flags::VDIR_SORT)
             .help("Sort by <field>: name, none (-U), time (-t), size (-S), extension (-X) or width")
             .value_name("field")
             .value_parser(["name", "none", "time", "size", "version", "extension", "width"])
             .require_equals(true)
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ]),
         Arg::new(vdir_flags::sort::VDIR_SIZE)
             .short('S')
             .help("Sort by file size, largest first.")
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::sort::VDIR_TIME)
             .short('t')
             .help("Sort by modification time (the 'mtime' in the inode), newest first.")
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::sort::VDIR_VERSION)
             .short('v')
             .help("Natural sort of (version) numbers in the filenames.")
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::sort::VDIR_EXTENSION)
             .short('X')
             .help("Sort alphabetically by entry extension.")
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::sort::VDIR_NONE)
             .short('U')
             .help(
                 "Do not sort; list the files in whatever order they are stored in the \
                     directory.  This is especially useful when listing very large directories, \
                     since not doing any sorting can be noticeably faster.",
             )
             .overrides_with_all([
                 vdir_flags::VDIR_SORT,
                 vdir_flags::sort::VDIR_SIZE,
                 vdir_flags::sort::VDIR_TIME,
                 vdir_flags::sort::VDIR_NONE,
                 vdir_flags::sort::VDIR_VERSION,
                 vdir_flags::sort::VDIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::dereference::VDIR_ALL)
             .short('L')
             .long(vdir_flags::dereference::VDIR_ALL)
             .help(
                 "When showing file information for a symbolic link, show information for the \
                     file the link references rather than the link itself.",
             )
             .overrides_with_all([
                 vdir_flags::dereference::VDIR_ALL,
                 vdir_flags::dereference::VDIR_DIR_ARGS,
                 vdir_flags::dereference::VDIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::dereference::VDIR_DIR_ARGS)
             .long(vdir_flags::dereference::VDIR_DIR_ARGS)
             .help(
                 "Do not follow symlinks except when they link to directories and are \
                     given as command line arguments.",
             )
             .overrides_with_all([
                 vdir_flags::dereference::VDIR_ALL,
                 vdir_flags::dereference::VDIR_DIR_ARGS,
                 vdir_flags::dereference::VDIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::dereference::VDIR_ARGS)
             .short('H')
             .long(vdir_flags::dereference::VDIR_ARGS)
             .help("Do not follow symlinks except when given as command line arguments.")
             .overrides_with_all([
                 vdir_flags::dereference::VDIR_ALL,
                 vdir_flags::dereference::VDIR_DIR_ARGS,
                 vdir_flags::dereference::VDIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_NO_GROUP)
             .long(vdir_flags::VDIR_NO_GROUP)
             .short('G')
             .help("Do not show group in long format.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_AUTHOR).long(vdir_flags::VDIR_AUTHOR).help(
             "Show author in long format. On the supported platforms, \
                 the author always matches the file owner.",
         ).action(ArgAction::SetTrue),
         Arg::new(vdir_flags::files::VDIR_ALL)
             .short('a')
             .long(vdir_flags::files::VDIR_ALL)
             // Overrides -A (as the order matters)
             .overrides_with_all([vdir_flags::files::VDIR_ALL, vdir_flags::files::VDIR_ALMOST_ALL])
             .help("Do not ignore hidden files (files with names that start with '.').")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::files::VDIR_ALMOST_ALL)
             .short('A')
             .long(vdir_flags::files::VDIR_ALMOST_ALL)
             // Overrides -a (as the order matters)
             .overrides_with_all([vdir_flags::files::VDIR_ALL, vdir_flags::files::VDIR_ALMOST_ALL])
             .help(
                 "In a directory, do not ignore all file names that start with '.', \
                     only ignore '.' and '..'.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_DIRECTORY)
             .short('d')
             .long(vdir_flags::VDIR_DIRECTORY)
             .help(
                 "Only list the names of directories, rather than listing directory contents. \
                     This will not follow symbolic links unless one of `--dereference-command-line \
                     (-H)`, `--dereference (-L)`, or `--dereference-command-line-symlink-to-dir` is \
                     specified.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::size::VDIR_HUMAN_READABLE)
             .short('h')
             .long(vdir_flags::size::VDIR_HUMAN_READABLE)
             .help("Print human readable file sizes (e.g. 1K 234M 56G).")
             .overrides_with_all([vdir_flags::size::VDIR_BLOCK_SIZE, vdir_flags::size::VDIR_SI])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::size::VDIR_KIBIBYTES)
             .short('k')
             .long(vdir_flags::size::VDIR_KIBIBYTES)
             .help(
                 "default to 1024-byte blocks for file system usage; used only with -s and per \
                     directory totals",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::size::VDIR_SI)
             .long(vdir_flags::size::VDIR_SI)
             .help("Print human readable file sizes using powers of 1000 instead of 1024.")
             .overrides_with_all([vdir_flags::size::VDIR_BLOCK_SIZE, vdir_flags::size::VDIR_HUMAN_READABLE])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::size::VDIR_BLOCK_SIZE)
             .long(vdir_flags::size::VDIR_BLOCK_SIZE)
             .require_equals(true)
             .value_name("VDIR_BLOCK_SIZE")
             .help("scale sizes by VDIR_BLOCK_SIZE when printing them")
             .overrides_with_all([vdir_flags::size::VDIR_SI, vdir_flags::size::VDIR_HUMAN_READABLE]),
         Arg::new(vdir_flags::VDIR_INODE)
             .short('i')
             .long(vdir_flags::VDIR_INODE)
             .help("print the index number of each file")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_REVERSE)
             .short('r')
             .long(vdir_flags::VDIR_REVERSE)
             .help(
                 "Reverse whatever the sorting method is e.g., list files in reverse \
             alphabetical order, youngest first, smallest first, or whatever.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_RECURSIVE)
             .short('R')
             .long(vdir_flags::VDIR_RECURSIVE)
             .help("List the contents of all directories recursively.")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_WIDTH)
             .long(vdir_flags::VDIR_WIDTH)
             .short('w')
             .help("Assume that the terminal is COLS columns wide.")
             .value_name("COLS"),
         Arg::new(vdir_flags::size::VDIR_ALLOCATION_SIZE)
             .short('s')
             .long(vdir_flags::size::VDIR_ALLOCATION_SIZE)
             .help("print the allocated size of each file, in blocks")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_COLOR)
             .long(vdir_flags::VDIR_COLOR)
             .help("Color output based on file type.")
             .value_parser([
                 "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
             ])
             .require_equals(true)
             .num_args(0..=1),
         Arg::new(vdir_flags::VDIR_INDICATOR_STYLE)
             .long(vdir_flags::VDIR_INDICATOR_STYLE)
             .help(
                 "Append indicator with style WORD to entry names: \
                 none (default),  slash (-p), file-type (--file-type), classify (-F)",
             )
             .value_parser(["none", "slash", "file-type", "classify"])
             .overrides_with_all([
                 vdir_flags::indicator_style::VDIR_FILE_TYPE,
                 vdir_flags::indicator_style::VDIR_SLASH,
                 vdir_flags::indicator_style::VDIR_CLASSIFY,
                 vdir_flags::VDIR_INDICATOR_STYLE,
             ]),
         Arg::new(vdir_flags::indicator_style::VDIR_CLASSIFY)
             .short('F')
             .long(vdir_flags::indicator_style::VDIR_CLASSIFY)
             .help(
                 "Append a character to each file name indicating the file type. Also, for \
                     regular files that are executable, append '*'. The file type indicators are \
                     '/' for directories, '@' for symbolic links, '|' for FIFOs, '=' for sockets, \
                     '>' for doors, and nothing for regular files. when may be omitted, or one of:\n\
                         \tnone - Do not classify. This is the default.\n\
                         \tauto - Only classify if standard output is a terminal.\n\
                         \talways - Always classify.\n\
                     Specifying --classify and no when is equivalent to --classify=always. This will \
                     not follow symbolic links listed on the command line unless the \
                     --dereference-command-line (-H), --dereference (-L), or \
                     --dereference-command-line-symlink-to-dir flags are specified.",
             )
             .value_name("when")
             .value_parser([
                 "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
             ])
             .default_missing_value("always")
             .require_equals(true)
             .num_args(0..=1)
             .overrides_with_all([
                 vdir_flags::indicator_style::VDIR_FILE_TYPE,
                 vdir_flags::indicator_style::VDIR_SLASH,
                 vdir_flags::indicator_style::VDIR_CLASSIFY,
                 vdir_flags::VDIR_INDICATOR_STYLE,
             ]),
         Arg::new(vdir_flags::indicator_style::VDIR_FILE_TYPE)
             .long(vdir_flags::indicator_style::VDIR_FILE_TYPE)
             .help("Same as --classify, but do not append '*'")
             .overrides_with_all([
                 vdir_flags::indicator_style::VDIR_FILE_TYPE,
                 vdir_flags::indicator_style::VDIR_SLASH,
                 vdir_flags::indicator_style::VDIR_CLASSIFY,
                 vdir_flags::VDIR_INDICATOR_STYLE,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::indicator_style::VDIR_SLASH)
             .short('p')
             .help("Append / indicator to directories.")
             .overrides_with_all([
                 vdir_flags::indicator_style::VDIR_FILE_TYPE,
                 vdir_flags::indicator_style::VDIR_SLASH,
                 vdir_flags::indicator_style::VDIR_CLASSIFY,
                 vdir_flags::VDIR_INDICATOR_STYLE,
             ])
             .action(ArgAction::SetTrue),
         //This still needs support for posix-*
         Arg::new(vdir_flags::VDIR_TIME_STYLE)
             .long(vdir_flags::VDIR_TIME_STYLE)
             .help("time/date format with -l; see VDIR_TIME_STYLE below")
             .value_name("VDIR_TIME_STYLE")
             .env("VDIR_TIME_STYLE")
             .value_parser(NonEmptyStringValueParser::new())
             .overrides_with_all([vdir_flags::VDIR_TIME_STYLE]),
         Arg::new(vdir_flags::VDIR_FULL_TIME)
             .long(vdir_flags::VDIR_FULL_TIME)
             .overrides_with(vdir_flags::VDIR_FULL_TIME)
             .help("like -l --time-style=full-iso")
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_CONTEXT)
             .short('Z')
             .long(vdir_flags::VDIR_CONTEXT)
             .help(VDIR_CONTEXT_HELP_TEXT)
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_GROUP_DIRECTORIES_FIRST)
             .long(vdir_flags::VDIR_GROUP_DIRECTORIES_FIRST)
             .help(
                 "group directories before files; can be augmented with \
                     a --sort option, but any use of --sort=none (-U) disables grouping",
             )
             .action(ArgAction::SetTrue),
         Arg::new(vdir_flags::VDIR_PATHS)
             .action(ArgAction::Append)
             .value_hint(clap::ValueHint::AnyPath)
             .value_parser(ValueParser::os_string()),
     ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .infer_long_args(true)
        .disable_help_flag(true)
        .args_override_self(true)
        .args(args)
        .after_help(VDIR_AFTER_HELP)
}

