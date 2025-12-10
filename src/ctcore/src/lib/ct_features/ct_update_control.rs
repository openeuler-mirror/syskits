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
//! Implement GNU-style update functionality.
//!
//! - pre-defined [`clap`-Arguments][1] for inclusion in utilities that
//!   implement updates
//! - determination of the [update mode][2]
//!
//! Update-functionality is implemented by the following utilities:
//!
//! - `cp`
//! - `mv`
//!
//!
//! [1]: arguments
//! [2]: `ct_determine_update_mode()`
//!
//!
//! # Usage example
//!
//! ```
//! #[macro_use]
//! extern crate ctcore;
//!
//! use clap::{Command, Arg, ArgMatches};
//! use ctcore::ct_update_control::{self, CtUpdateMode};
//!
//! fn main() {
//!     let matches = Command::new("command")
//!         .arg(ct_update_control::arguments::update())
//!         .arg(ct_update_control::arguments::update_no_args())
//!         .get_matches_from(vec![
//!             "command", "--update=older"
//!         ]);
//!
//!     let update_mode = ct_update_control::ct_determine_update_mode(&matches);
//!
//!     // handle cases
//!     if update_mode == CtUpdateMode::ReplaceIfOlder {
//!         // do
//!     } else {
//!         unreachable!()
//!     }
//! }
//! ```
use clap::ArgMatches;

// Available update mode
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CtUpdateMode {
    // --update=`all`, ``
    ReplaceAll,
    // --update=`none`
    ReplaceNone,
    // --update=`older`
    // -u
    ReplaceIfOlder,
}

pub mod arguments {
    use clap::ArgAction;

    pub static OPT_UPDATE: &str = "update";
    pub static OPT_UPDATE_NO_ARG: &str = "u";

    // `--update` argument, defaults to `older` if no values are provided
    pub fn update() -> clap::Arg {
        clap::Arg::new(OPT_UPDATE)
            .long("update")
            .help("move only when the SOURCE file is newer than the destination file or when the destination file is missing")
            .value_parser(["none", "all", "older"])
            .num_args(0..=1)
            .default_missing_value("older")
            .require_equals(true)
            .overrides_with("update")
            .action(clap::ArgAction::Set)
    }

    // `-u` argument
    pub fn update_no_args() -> clap::Arg {
        clap::Arg::new(OPT_UPDATE_NO_ARG)
            .short('u')
            .help("like --update but does not accept an argument")
            .action(ArgAction::SetTrue)
    }
}

/// Determine the "mode" for the update operation to perform, if any.
///
/// Parses the backup options and converts them to an instance of
/// `CtUpdateMode` for further processing.
///
/// Takes [`clap::ArgMatches`] as argument which **must** contain the options
/// from [`arguments::update()`] or [`arguments::update_no_args()`]. Otherwise
/// the `ReplaceAll` mode is returned unconditionally.
///
/// # Examples
///
/// Here's how one would integrate the update mode determination into an
/// application.
///
/// ```
/// #[macro_use]
/// extern crate ctcore;
/// use ctcore::ct_update_control::{self, CtUpdateMode};
/// use clap::{Command, Arg, ArgMatches};
///
/// fn main() {
///     let matches = Command::new("command")
///         .arg(ct_update_control::arguments::update())
///         .arg(ct_update_control::arguments::update_no_args())
///         .get_matches_from(vec![
///             "command", "--update=all"
///         ]);
///
///     let update_mode = ct_update_control::ct_determine_update_mode(&matches);
///     assert_eq!(update_mode, CtUpdateMode::ReplaceAll)
/// }
pub fn ct_determine_update_mode(matches: &ArgMatches) -> CtUpdateMode {
    if let Some(mode) = matches.get_one::<String>(arguments::OPT_UPDATE) {
        match mode.as_str() {
            "all" => CtUpdateMode::ReplaceAll,
            "none" => CtUpdateMode::ReplaceNone,
            "older" => CtUpdateMode::ReplaceIfOlder,
            _ => unreachable!("other args restricted by clap"),
        }
    } else if matches.get_flag(arguments::OPT_UPDATE_NO_ARG) {
        // short form of this option is equivalent to using --update=older
        CtUpdateMode::ReplaceIfOlder
    } else {
        // no option was present
        CtUpdateMode::ReplaceAll
    }
}
