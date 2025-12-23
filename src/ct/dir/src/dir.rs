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
use ctcore::ct_error::CTResult;
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};

const DIR_ABOUT: &str = ct_help_about!("dir.md");
const DIR_AFTER_HELP: &str = ct_help_section!("after help", "dir.md");
const DIR_USAGE: &str = ct_help_usage!("dir.md");
#[cfg(not(feature = "selinux"))]
static DIR_CONTEXT_HELP_TEXT: &str = "print any security context of each file (not enabled)";
#[cfg(feature = "selinux")]
static DIR_CONTEXT_HELP_TEXT: &str = "print any security context of each file";

pub mod dir_flags {
    pub mod format {
        pub static DIR_ONE_LINE: &str = "1";
        pub static DIR_LONG: &str = "long";
        pub static DIR_COLUMNS: &str = "C";
        pub static DIR_ACROSS: &str = "x";
        pub static DIR_TAB_SIZE: &str = "tabsize";
        pub static DIR_COMMAS: &str = "m";
        pub static DIR_LONG_NO_OWNER: &str = "g";
        pub static DIR_LONG_NO_GROUP: &str = "o";
        pub static DIR_LONG_NUMERIC_UID_GID: &str = "numeric-uid-gid";
    }

    pub mod files {
        pub static DIR_ALL: &str = "all";
        pub static DIR_ALMOST_ALL: &str = "almost-all";
    }

    pub mod sort {
        pub static DIR_SIZE: &str = "S";
        pub static DIR_TIME: &str = "t";
        pub static DIR_NONE: &str = "U";
        pub static DIR_VERSION: &str = "v";
        pub static DIR_EXTENSION: &str = "X";
    }

    pub mod time {
        pub static DIR_ACCESS: &str = "u";
        pub static DIR_CHANGE: &str = "c";
    }

    pub mod size {
        pub static DIR_ALLOCATION_SIZE: &str = "size";
        pub static DIR_BLOCK_SIZE: &str = "block-size";
        pub static DIR_HUMAN_READABLE: &str = "human-readable";
        pub static DIR_SI: &str = "si";
        pub static DIR_KIBIBYTES: &str = "kibibytes";
    }

    pub mod quoting {
        pub static DIR_ESCAPE: &str = "escape";
        pub static DIR_LITERAL: &str = "literal";
        pub static DIR_C: &str = "quote-name";
    }

    pub mod indicator_style {
        pub static DIR_SLASH: &str = "p";
        pub static DIR_FILE_TYPE: &str = "file-type";
        pub static DIR_CLASSIFY: &str = "classify";
    }

    pub mod dereference {
        pub static DIR_ALL: &str = "dereference";
        pub static DIR_ARGS: &str = "dereference-command-line";
        pub static DIR_DIR_ARGS: &str = "dereference-command-line-symlink-to-dir";
    }

    pub static DIR_HELP: &str = "help";
    pub static DIR_QUOTING_STYLE: &str = "quoting-style";
    pub static DIR_HIDE_CONTROL_CHARS: &str = "hide-control-chars";
    pub static DIR_SHOW_CONTROL_CHARS: &str = "show-control-chars";
    pub static DIR_WIDTH: &str = "width";
    pub static DIR_AUTHOR: &str = "author";
    pub static DIR_NO_GROUP: &str = "no-group";
    pub static DIR_FORMAT: &str = "format";
    pub static DIR_SORT: &str = "sort";
    pub static DIR_TIME: &str = "time";
    pub static DIR_IGNORE_BACKUPS: &str = "ignore-backups";
    pub static DIR_DIRECTORY: &str = "directory";
    pub static DIR_INODE: &str = "inode";
    pub static DIR_REVERSE: &str = "reverse";
    pub static DIR_RECURSIVE: &str = "recursive";
    pub static DIR_COLOR: &str = "color";
    pub static DIR_PATHS: &str = "paths";
    pub static DIR_INDICATOR_STYLE: &str = "indicator-style";
    pub static DIR_TIME_STYLE: &str = "time-style";
    pub static DIR_FULL_TIME: &str = "full-time";
    pub static DIR_HIDE: &str = "hide";
    pub static DIR_IGNORE: &str = "ignore";
    pub static DIR_CONTEXT: &str = "context";
    pub static DIR_GROUP_DIRECTORIES_FIRST: &str = "group-directories-first";
    pub static DIR_ZERO: &str = "zero";
    pub static DIR_DIRED: &str = "dired";
    pub static DIR_HYPERLINK: &str = "hyperlink";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    dir_main(args).map(|_| ())
}

pub fn dir_main(args: impl ctcore::Args) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
    let command = ct_app();
    let matches = command.get_matches_from(args);

    let mut default_quoting_style = false;
    let mut default_format_style = false;

    // 我们会检查是否给出了格式化或引号样式标志。
    // 如果没有，我们将使用 dir 默认的格式化和引用样式标志
    if !matches.contains_id(dir_flags::DIR_QUOTING_STYLE)
        && !matches.get_flag(dir_flags::quoting::DIR_C)
        && !matches.get_flag(dir_flags::quoting::DIR_ESCAPE)
        && !matches.get_flag(dir_flags::quoting::DIR_LITERAL)
    {
        default_quoting_style = true;
    }
    if !matches.contains_id(dir_flags::DIR_FORMAT)
        && !matches.get_flag(dir_flags::format::DIR_ACROSS)
        && !matches.get_flag(dir_flags::format::DIR_COLUMNS)
        && !matches.get_flag(dir_flags::format::DIR_COMMAS)
        && !matches.get_flag(dir_flags::format::DIR_LONG)
        && !matches.get_flag(dir_flags::format::DIR_LONG_NO_GROUP)
        && !matches.get_flag(dir_flags::format::DIR_LONG_NO_OWNER)
        && !matches.get_flag(dir_flags::format::DIR_LONG_NUMERIC_UID_GID)
        && !matches.get_flag(dir_flags::format::DIR_ONE_LINE)
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
        config.format = LsFormat::Columns;
    }

    let paths_list = matches.get_many::<OsString>(dir_flags::DIR_PATHS);
    let paths_from_args: Vec<_> = paths_list
        .map(|v| v.map(Path::new).collect())
        .unwrap_or_else(|| vec![Path::new(".")]);

    ct_ls::list(paths_from_args, &config)
}

// 实现逻辑和ls一致
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DIR_ABOUT;
    let usage_description = ct_format_usage(DIR_USAGE);

    let args = vec![
         Arg::new(dir_flags::DIR_HELP)
             .long(dir_flags::DIR_HELP)
             .help("Print help information.")
             .action(ArgAction::Help),
         Arg::new(dir_flags::DIR_FORMAT)
             .long(dir_flags::DIR_FORMAT)
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
                 dir_flags::DIR_FORMAT,
                 dir_flags::format::DIR_COLUMNS,
                 dir_flags::format::DIR_LONG,
                 dir_flags::format::DIR_ACROSS,
                 dir_flags::format::DIR_COLUMNS,
             ]),
         Arg::new(dir_flags::format::DIR_COLUMNS)
             .short('C')
             .help("Display the files in columns.")
             .overrides_with_all([
                 dir_flags::DIR_FORMAT,
                 dir_flags::format::DIR_COLUMNS,
                 dir_flags::format::DIR_LONG,
                 dir_flags::format::DIR_ACROSS,
                 dir_flags::format::DIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_LONG)
             .short('l')
             .long(dir_flags::format::DIR_LONG)
             .help("Display detailed information.")
             .overrides_with_all([
                 dir_flags::DIR_FORMAT,
                 dir_flags::format::DIR_COLUMNS,
                 dir_flags::format::DIR_LONG,
                 dir_flags::format::DIR_ACROSS,
                 dir_flags::format::DIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_ACROSS)
             .short('x')
             .help("List entries in rows instead of in columns.")
             .overrides_with_all([
                 dir_flags::DIR_FORMAT,
                 dir_flags::format::DIR_COLUMNS,
                 dir_flags::format::DIR_LONG,
                 dir_flags::format::DIR_ACROSS,
                 dir_flags::format::DIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_TAB_SIZE)
             .short('T')
             .long(dir_flags::format::DIR_TAB_SIZE)
             .env("TABSIZE")
             .value_name("COLS")
             .help("Assume tab stops at each COLS instead of 8 (unimplemented)"),
         Arg::new(dir_flags::format::DIR_COMMAS)
             .short('m')
             .help("List entries separated by commas.")
             .overrides_with_all([
                 dir_flags::DIR_FORMAT,
                 dir_flags::format::DIR_COLUMNS,
                 dir_flags::format::DIR_LONG,
                 dir_flags::format::DIR_ACROSS,
                 dir_flags::format::DIR_COLUMNS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_ZERO)
             .long(dir_flags::DIR_ZERO)
             .overrides_with(dir_flags::DIR_ZERO)
             .help("List entries separated by ASCII NUL characters.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_DIRED)
             .long(dir_flags::DIR_DIRED)
             .short('D')
             .help("generate output designed for Emacs' dired (Directory Editor) mode")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_HYPERLINK)
             .long(dir_flags::DIR_HYPERLINK)
             .help("hyperlink file names WHEN")
             .value_parser([
                 "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
             ])
             .require_equals(true)
             .num_args(0..=1)
             .default_missing_value("always")
             .default_value("never")
             .value_name("WHEN"),
         Arg::new(dir_flags::format::DIR_ONE_LINE)
             .short('1')
             .help("List one file per line.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_LONG_NO_GROUP)
             .short('o')
             .help(
                 "Long format without group information. \
                         Identical to --format=long with --no-group.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_LONG_NO_OWNER)
             .short('g')
             .help("Long format without owner information.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::format::DIR_LONG_NUMERIC_UID_GID)
             .short('n')
             .long(dir_flags::format::DIR_LONG_NUMERIC_UID_GID)
             .help("-l with numeric UIDs and GIDs.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_QUOTING_STYLE)
             .long(dir_flags::DIR_QUOTING_STYLE)
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
                 dir_flags::DIR_QUOTING_STYLE,
                 dir_flags::quoting::DIR_LITERAL,
                 dir_flags::quoting::DIR_ESCAPE,
                 dir_flags::quoting::DIR_C,
             ]),
         Arg::new(dir_flags::quoting::DIR_LITERAL)
             .short('N')
             .long(dir_flags::quoting::DIR_LITERAL)
             .alias("l")
             .help("Use literal quoting style. Equivalent to `--quoting-style=literal`")
             .overrides_with_all([
                 dir_flags::DIR_QUOTING_STYLE,
                 dir_flags::quoting::DIR_LITERAL,
                 dir_flags::quoting::DIR_ESCAPE,
                 dir_flags::quoting::DIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::quoting::DIR_ESCAPE)
             .short('b')
             .long(dir_flags::quoting::DIR_ESCAPE)
             .help("Use escape quoting style. Equivalent to `--quoting-style=escape`")
             .overrides_with_all([
                 dir_flags::DIR_QUOTING_STYLE,
                 dir_flags::quoting::DIR_LITERAL,
                 dir_flags::quoting::DIR_ESCAPE,
                 dir_flags::quoting::DIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::quoting::DIR_C)
             .short('Q')
             .long(dir_flags::quoting::DIR_C)
             .help("Use CT_C quoting style. Equivalent to `--quoting-style=c`")
             .overrides_with_all([
                 dir_flags::DIR_QUOTING_STYLE,
                 dir_flags::quoting::DIR_LITERAL,
                 dir_flags::quoting::DIR_ESCAPE,
                 dir_flags::quoting::DIR_C,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_HIDE_CONTROL_CHARS)
             .short('q')
             .long(dir_flags::DIR_HIDE_CONTROL_CHARS)
             .help("Replace control characters with '?' if they are not escaped.")
             .overrides_with_all([dir_flags::DIR_HIDE_CONTROL_CHARS, dir_flags::DIR_SHOW_CONTROL_CHARS])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_SHOW_CONTROL_CHARS)
             .long(dir_flags::DIR_SHOW_CONTROL_CHARS)
             .help("Show control characters 'as is' if they are not escaped.")
             .overrides_with_all([dir_flags::DIR_HIDE_CONTROL_CHARS, dir_flags::DIR_SHOW_CONTROL_CHARS])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_TIME)
             .long(dir_flags::DIR_TIME)
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
             .overrides_with_all([dir_flags::DIR_TIME, dir_flags::time::DIR_ACCESS, dir_flags::time::DIR_CHANGE]),
         Arg::new(dir_flags::time::DIR_CHANGE)
             .short('c')
             .help(
                 "If the long listing format (e.g., -l, -o) is being used, print the \
                         status change time (the 'ctime' in the inode) instead of the modification \
                         time. When explicitly sorting by time (--sort=time or -t) or when not \
                         using a long listing format, sort according to the status change time.",
             )
             .overrides_with_all([dir_flags::DIR_TIME, dir_flags::time::DIR_ACCESS, dir_flags::time::DIR_CHANGE])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::time::DIR_ACCESS)
             .short('u')
             .help(
                 "If the long listing format (e.g., -l, -o) is being used, print the \
                         status access time instead of the modification time. When explicitly \
                         sorting by time (--sort=time or -t) or when not using a long listing \
                         format, sort according to the access time.",
             )
             .overrides_with_all([dir_flags::DIR_TIME, dir_flags::time::DIR_ACCESS, dir_flags::time::DIR_CHANGE])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_HIDE)
             .long(dir_flags::DIR_HIDE)
             .action(ArgAction::Append)
             .value_name("PATTERN")
             .help(
                 "do not list implied entries matching shell PATTERN (overridden by -a or -A)",
             ),
         Arg::new(dir_flags::DIR_IGNORE)
             .short('I')
             .long(dir_flags::DIR_IGNORE)
             .action(ArgAction::Append)
             .value_name("PATTERN")
             .help("do not list implied entries matching shell PATTERN"),
         Arg::new(dir_flags::DIR_IGNORE_BACKUPS)
             .short('B')
             .long(dir_flags::DIR_IGNORE_BACKUPS)
             .help("Ignore entries which end with ~.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_SORT)
             .long(dir_flags::DIR_SORT)
             .help("Sort by <field>: name, none (-U), time (-t), size (-S), extension (-X) or width")
             .value_name("field")
             .value_parser(["name", "none", "time", "size", "version", "extension", "width"])
             .require_equals(true)
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ]),
         Arg::new(dir_flags::sort::DIR_SIZE)
             .short('S')
             .help("Sort by file size, largest first.")
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::sort::DIR_TIME)
             .short('t')
             .help("Sort by modification time (the 'mtime' in the inode), newest first.")
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::sort::DIR_VERSION)
             .short('v')
             .help("Natural sort of (version) numbers in the filenames.")
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::sort::DIR_EXTENSION)
             .short('X')
             .help("Sort alphabetically by entry extension.")
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::sort::DIR_NONE)
             .short('U')
             .help(
                 "Do not sort; list the files in whatever order they are stored in the \
                     directory.  This is especially useful when listing very large directories, \
                     since not doing any sorting can be noticeably faster.",
             )
             .overrides_with_all([
                 dir_flags::DIR_SORT,
                 dir_flags::sort::DIR_SIZE,
                 dir_flags::sort::DIR_TIME,
                 dir_flags::sort::DIR_NONE,
                 dir_flags::sort::DIR_VERSION,
                 dir_flags::sort::DIR_EXTENSION,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::dereference::DIR_ALL)
             .short('L')
             .long(dir_flags::dereference::DIR_ALL)
             .help(
                 "When showing file information for a symbolic link, show information for the \
                     file the link references rather than the link itself.",
             )
             .overrides_with_all([
                 dir_flags::dereference::DIR_ALL,
                 dir_flags::dereference::DIR_DIR_ARGS,
                 dir_flags::dereference::DIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::dereference::DIR_DIR_ARGS)
             .long(dir_flags::dereference::DIR_DIR_ARGS)
             .help(
                 "Do not follow symlinks except when they link to directories and are \
                     given as command line arguments.",
             )
             .overrides_with_all([
                 dir_flags::dereference::DIR_ALL,
                 dir_flags::dereference::DIR_DIR_ARGS,
                 dir_flags::dereference::DIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::dereference::DIR_ARGS)
             .short('H')
             .long(dir_flags::dereference::DIR_ARGS)
             .help("Do not follow symlinks except when given as command line arguments.")
             .overrides_with_all([
                 dir_flags::dereference::DIR_ALL,
                 dir_flags::dereference::DIR_DIR_ARGS,
                 dir_flags::dereference::DIR_ARGS,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_NO_GROUP)
             .long(dir_flags::DIR_NO_GROUP)
             .short('G')
             .help("Do not show group in long format.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_AUTHOR).long(dir_flags::DIR_AUTHOR).help(
             "Show author in long format. On the supported platforms, \
                 the author always matches the file owner.",
         ).action(ArgAction::SetTrue),
         Arg::new(dir_flags::files::DIR_ALL)
             .short('a')
             .long(dir_flags::files::DIR_ALL)
             // Overrides -A (as the order matters)
             .overrides_with_all([dir_flags::files::DIR_ALL, dir_flags::files::DIR_ALMOST_ALL])
             .help("Do not ignore hidden files (files with names that start with '.').")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::files::DIR_ALMOST_ALL)
             .short('A')
             .long(dir_flags::files::DIR_ALMOST_ALL)
             // Overrides -a (as the order matters)
             .overrides_with_all([dir_flags::files::DIR_ALL, dir_flags::files::DIR_ALMOST_ALL])
             .help(
                 "In a directory, do not ignore all file names that start with '.', \
                     only ignore '.' and '..'.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_DIRECTORY)
             .short('d')
             .long(dir_flags::DIR_DIRECTORY)
             .help(
                 "Only list the names of directories, rather than listing directory contents. \
                     This will not follow symbolic links unless one of `--dereference-command-line \
                     (-H)`, `--dereference (-L)`, or `--dereference-command-line-symlink-to-dir` is \
                     specified.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::size::DIR_HUMAN_READABLE)
             .short('h')
             .long(dir_flags::size::DIR_HUMAN_READABLE)
             .help("Print human readable file sizes (e.g. 1K 234M 56G).")
             .overrides_with_all([dir_flags::size::DIR_BLOCK_SIZE, dir_flags::size::DIR_SI])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::size::DIR_KIBIBYTES)
             .short('k')
             .long(dir_flags::size::DIR_KIBIBYTES)
             .help(
                 "default to 1024-byte blocks for file system usage; used only with -s and per \
                     directory totals",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::size::DIR_SI)
             .long(dir_flags::size::DIR_SI)
             .help("Print human readable file sizes using powers of 1000 instead of 1024.")
             .overrides_with_all([dir_flags::size::DIR_BLOCK_SIZE, dir_flags::size::DIR_HUMAN_READABLE])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::size::DIR_BLOCK_SIZE)
             .long(dir_flags::size::DIR_BLOCK_SIZE)
             .require_equals(true)
             .value_name("CT_BLOCK_SIZE")
             .help("scale sizes by CT_BLOCK_SIZE when printing them")
             .overrides_with_all([dir_flags::size::DIR_SI, dir_flags::size::DIR_HUMAN_READABLE]),
         Arg::new(dir_flags::DIR_INODE)
             .short('i')
             .long(dir_flags::DIR_INODE)
             .help("print the index number of each file")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_REVERSE)
             .short('r')
             .long(dir_flags::DIR_REVERSE)
             .help(
                 "Reverse whatever the sorting method is e.g., list files in reverse \
             alphabetical order, youngest first, smallest first, or whatever.",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_RECURSIVE)
             .short('R')
             .long(dir_flags::DIR_RECURSIVE)
             .help("List the contents of all directories recursively.")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_WIDTH)
             .long(dir_flags::DIR_WIDTH)
             .short('w')
             .help("Assume that the terminal is COLS columns wide.")
             .value_name("COLS"),
         Arg::new(dir_flags::size::DIR_ALLOCATION_SIZE)
             .short('s')
             .long(dir_flags::size::DIR_ALLOCATION_SIZE)
             .help("print the allocated size of each file, in blocks")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_COLOR)
             .long(dir_flags::DIR_COLOR)
             .help("Color output based on file type.")
             .value_parser([
                 "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
             ])
             .require_equals(true)
             .num_args(0..=1),
         Arg::new(dir_flags::DIR_INDICATOR_STYLE)
             .long(dir_flags::DIR_INDICATOR_STYLE)
             .help(
                 "Append indicator with style WORD to entry names: \
                 none (default),  slash (-p), file-type (--file-type), classify (-F)",
             )
             .value_parser(["none", "slash", "file-type", "classify"])
             .overrides_with_all([
                 dir_flags::indicator_style::DIR_FILE_TYPE,
                 dir_flags::indicator_style::DIR_SLASH,
                 dir_flags::indicator_style::DIR_CLASSIFY,
                 dir_flags::DIR_INDICATOR_STYLE,
             ]),
         Arg::new(dir_flags::indicator_style::DIR_CLASSIFY)
             .short('F')
             .long(dir_flags::indicator_style::DIR_CLASSIFY)
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
                 dir_flags::indicator_style::DIR_FILE_TYPE,
                 dir_flags::indicator_style::DIR_SLASH,
                 dir_flags::indicator_style::DIR_CLASSIFY,
                 dir_flags::DIR_INDICATOR_STYLE,
             ]),
         Arg::new(dir_flags::indicator_style::DIR_FILE_TYPE)
             .long(dir_flags::indicator_style::DIR_FILE_TYPE)
             .help("Same as --classify, but do not append '*'")
             .overrides_with_all([
                 dir_flags::indicator_style::DIR_FILE_TYPE,
                 dir_flags::indicator_style::DIR_SLASH,
                 dir_flags::indicator_style::DIR_CLASSIFY,
                 dir_flags::DIR_INDICATOR_STYLE,
             ])
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::indicator_style::DIR_SLASH)
             .short('p')
             .help("Append / indicator to directories.")
             .overrides_with_all([
                 dir_flags::indicator_style::DIR_FILE_TYPE,
                 dir_flags::indicator_style::DIR_SLASH,
                 dir_flags::indicator_style::DIR_CLASSIFY,
                 dir_flags::DIR_INDICATOR_STYLE,
             ])
             .action(ArgAction::SetTrue),
         //This still needs support for posix-*
         Arg::new(dir_flags::DIR_TIME_STYLE)
             .long(dir_flags::DIR_TIME_STYLE)
             .help("time/date format with -l; see CT_TIME_STYLE below")
             .value_name("CT_TIME_STYLE")
             .env("CT_TIME_STYLE")
             .value_parser(NonEmptyStringValueParser::new())
             .overrides_with_all([dir_flags::DIR_TIME_STYLE]),
         Arg::new(dir_flags::DIR_FULL_TIME)
             .long(dir_flags::DIR_FULL_TIME)
             .overrides_with(dir_flags::DIR_FULL_TIME)
             .help("like -l --time-style=full-iso")
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_CONTEXT)
             .short('Z')
             .long(dir_flags::DIR_CONTEXT)
             .help(DIR_CONTEXT_HELP_TEXT)
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_GROUP_DIRECTORIES_FIRST)
             .long(dir_flags::DIR_GROUP_DIRECTORIES_FIRST)
             .help(
                 "group directories before files; can be augmented with \
                     a --sort option, but any use of --sort=none (-U) disables grouping",
             )
             .action(ArgAction::SetTrue),
         Arg::new(dir_flags::DIR_PATHS)
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
        .after_help(DIR_AFTER_HELP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::File;
        use std::io::Write;

        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_ctmain_input_err_no_app_name_v() {
            let args = ["--version", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 2);
        }

        #[test]
        fn test_ctmain_input_err_no_app_name_uppercase_v() {
            let args = ["-V", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            //println!("{}", result);
            assert_eq!(result, 2);
        }

        #[test]
        fn test_ctmain_return() {
            let args = vec![ctcore::ct_util_name()];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ctmain_dir_dir_return() {
            let args = vec![ctcore::ct_util_name(), "./"];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        // vdir 文件测试
        #[test]
        fn test_ct_main_with_dir_file() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), test_file_path.to_str().unwrap()];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_a() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-a", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_all() {
            // 创建临时目录结构
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-all", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_block_size() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--block-size=1", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--block-size=1", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_format_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--format=long", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_columns_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-C", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-l", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_across_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-x", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-T", "4", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--tabsize=8", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_commas_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-m", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_one_line_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-1", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_group_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-o", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_owner_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-g", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-n", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_quoting_style_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=literal", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_literal_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-N", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_escape_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-b", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_c_quoting_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-Q", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_control_chars_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-q", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_show_control_chars_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--show-control-chars", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--time=access", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--hide=*", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-I", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_backups_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-B", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_change_short_c() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-c", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_access_short_u() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-u", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short_uppercase_i() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-I", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_sort_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--sort=size", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_size_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-S", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_sort_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-t", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_extension_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-X", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_none_sort_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-U", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_all_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-L", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_dir_args_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
                &dir_name,
            ];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_args_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-H", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_no_group_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-G", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_all_files_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-a", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_almost_all_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-A", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_directory_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-d", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                    assert_eq!(0, dir_vec.len());
                    assert_eq!(1, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_kibibytes_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-k", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_si_long() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--si", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_inode_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-i", dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_reverse_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-r", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_recursive_short() {
            let file_name = "test_dir_file";
            let tmp_dir = TempDir::with_prefix("test_dir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-R", &dir_name];
            let result = dir_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // vdir 接口: vdir [OPTION]... [FILE]...
        // List information about the FILEs (the current directory by default).
        // Sort entries alphabetically if none of -cftuvSUX nor --sort is specified.
        //
        // Mandatory arguments to long options are mandatory for short options too.
        //   -a, --all                  do not ignore entries starting with .
        //   -A, --almost-all           do not list implied . and ..
        //       --author               with -l, print the author of each file
        //   -b, --escape               print C-style escapes for nongraphic characters
        //       --block-size=SIZE      with -l, scale sizes by SIZE when printing them;
        //                                e.g., '--block-size=M'; see SIZE format below
        //   -B, --ignore-backups       do not list implied entries ending with ~
        //   -c                         with -lt: sort by, and show, ctime (time of last
        //                                modification of file status information);
        //                                with -l: show ctime and sort by name;
        //                                otherwise: sort by ctime, newest first
        //   -C                         list entries by columns
        //       --color[=WHEN]         colorize the output; WHEN can be 'always' (default
        //                                if omitted), 'auto', or 'never'; more info below
        //   -d, --directory            list directories themselves, not their contents
        //   -D, --dired                generate output designed for Emacs' dired mode
        //   -f                         list all entries in directory order
        //   -F, --classify[=WHEN]      append indicator (one of */=>@|) to entries;
        //                                WHEN can be 'always' (default if omitted),
        //                                'auto', or 'never'
        //       --file-type            likewise, except do not append '*'
        //       --format=WORD          across -x, commas -m, horizontal -x, long -l,
        //                                single-column -1, verbose -l, vertical -C
        //       --full-time            like -l --time-style=full-iso
        //   -g                         like -l, but do not list owner
        //       --group-directories-first
        //                              group directories before files;
        //                                can be augmented with a --sort option, but any
        //                                use of --sort=none (-U) disables grouping
        //   -G, --no-group             in a long listing, don't print group names
        //   -h, --human-readable       with -l and -s, print sizes like 1K 234M 2G etc.
        //       --si                   likewise, but use powers of 1000 not 1024
        //   -H, --dereference-command-line
        //                              follow symbolic links listed on the command line
        //       --dereference-command-line-symlink-to-dir
        //                              follow each command line symbolic link
        //                                that points to a directory
        //       --hide=PATTERN         do not list implied entries matching shell PATTERN
        //                                (overridden by -a or -A)
        //       --hyperlink[=WHEN]     hyperlink file names; WHEN can be 'always'
        //                                (default if omitted), 'auto', or 'never'
        //       --indicator-style=WORD  append indicator with style WORD to entry names:
        //                                none (default), slash (-p),
        //                                file-type (--file-type), classify (-F)
        //   -i, --inode                print the index number of each file
        //   -I, --ignore=PATTERN       do not list implied entries matching shell PATTERN
        //   -k, --kibibytes            default to 1024-byte blocks for file system usage;
        //                                used only with -s and per directory totals
        //   -l                         use a long listing format
        //   -L, --dereference          when showing file information for a symbolic
        //                                link, show information for the file the link
        //                                references rather than for the link itself
        //   -m                         fill width with a comma separated list of entries
        //   -n, --numeric-uid-gid      like -l, but list numeric user and group IDs
        //   -N, --literal              print entry names without quoting
        //   -o                         like -l, but do not list group information
        //   -p, --indicator-style=slash
        //                              append / indicator to directories
        //   -q, --hide-control-chars   print ? instead of nongraphic characters
        //       --show-control-chars   show nongraphic characters as-is (the default,
        //                                unless program is 'ls' and output is a terminal)
        //   -Q, --quote-name           enclose entry names in double quotes
        //       --quoting-style=WORD   use quoting style WORD for entry names:
        //                                literal, locale, shell, shell-always,
        //                                shell-escape, shell-escape-always, c, escape
        //                                (overrides QUOTING_STYLE environment variable)
        //   -r, --reverse              reverse order while sorting
        //   -R, --recursive            list subdirectories recursively
        //   -s, --size                 print the allocated size of each file, in blocks
        //   -S                         sort by file size, largest first
        //       --sort=WORD            sort by WORD instead of name: none (-U), size (-S),
        //                                time (-t), version (-v), extension (-X), width
        //       --time=WORD            change the default of using modification times;
        //                                access time (-u): atime, access, use;
        //                                change time (-c): ctime, status;
        //                                birth time: birth, creation;
        //                              with -l, WORD determines which time to show;
        //                              with --sort=time, sort by WORD (newest first)
        //       --time-style=TIME_STYLE  time/date format with -l; see TIME_STYLE below
        //   -t                         sort by time, newest first; see --time
        //   -T, --tabsize=COLS         assume tab stops at each COLS instead of 8
        //   -u                         with -lt: sort by, and show, access time;
        //                                with -l: show access time and sort by name;
        //                                otherwise: sort by access time, newest first
        //   -U                         do not sort; list entries in directory order
        //   -v                         natural sort of (version) numbers within text
        //   -w, --width=COLS           set output width to COLS.  0 means no limit
        //   -x                         list entries by lines instead of by columns
        //   -X                         sort alphabetically by entry extension
        //   -Z, --context              print any security context of each file
        //       --zero                 end each output line with NUL, not newline
        //   -1                         list one file per line
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_not_support_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_help_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_ct_app_format_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=long"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::CT_FORMAT));
            assert!(matches.unwrap().contains_id(dir_flags::DIR_FORMAT));
        }

        #[test]
        fn test_ct_app_columns_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-C"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(dir_flags::format::DIR_COLUMNS));
        }

        #[test]
        fn test_ct_app_long_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(dir_flags::format::DIR_LONG));
        }

            #[test]
        fn test_ct_app_across_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-x"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(dir_flags::format::DIR_ACROSS));
        }

        #[test]
        fn test_ct_app_tab_size_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-T", "4"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(dir_flags::format::DIR_TAB_SIZE)
                    .unwrap(),
                "4"
            );
        }

        #[test]
        fn test_ct_app_tab_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tabsize=8"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(dir_flags::format::DIR_TAB_SIZE)
                    .unwrap(),
                "8"
            );
        }

        #[test]
        fn test_ct_app_commas_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-m"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(dir_flags::format::DIR_COMMAS));
        }

        #[test]
        fn test_ct_app_one_line_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-1"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(dir_flags::format::DIR_ONE_LINE));
        }

        #[test]
        fn test_ct_app_long_no_group_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-o"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(dir_flags::format::DIR_LONG_NO_GROUP));
        }

        #[test]
        fn test_ct_app_long_no_owner_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(dir_flags::format::DIR_LONG_NO_OWNER));
        }

        #[test]
        fn test_ct_app_long_no_owner_short_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g", "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-n"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(dir_flags::format::DIR_LONG_NUMERIC_UID_GID));
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(dir_flags::format::DIR_LONG_NUMERIC_UID_GID));
        }

    }
}
