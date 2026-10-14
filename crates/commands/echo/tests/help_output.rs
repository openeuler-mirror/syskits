#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn help_output_uses_syskits_information() {
    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0("echo")
        .arg("--help")
        .env("LC_ALL", "C")
        .output()
        .expect("run echo --help");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        br#"Usage: echo [SHORT-OPTION]... [STRING]...
  or:  echo LONG-OPTION
Echo the STRING(s) to standard output.

  -n             do not output the trailing newline
  -e             enable interpretation of backslash escapes
  -E             disable interpretation of backslash escapes (default)
      --help        display this help and exit
      --version     output version information and exit

If -e is in effect, the following sequences are recognized:

  \\      backslash
  \a      alert (BEL)
  \b      backspace
  \c      produce no further output
  \e      escape
  \f      form feed
  \n      new line
  \r      carriage return
  \t      horizontal tab
  \v      vertical tab
  \0NNN   byte with octal value NNN (1 to 3 digits)
  \xHH    byte with hexadecimal value HH (1 to 2 digits)

NOTE: your shell may have its own version of echo, which usually supersedes
the version described here.  Please refer to your shell's documentation
for details about the options it supports.

NOTE: printf(1) is a preferred alternative,
which does not have issues outputting option-like strings.

syskits online help: <https://www.ctyun.cn/developer>
"#
    );
    assert!(!output.stdout.windows(3).any(|window| window == b"GNU"));
}
