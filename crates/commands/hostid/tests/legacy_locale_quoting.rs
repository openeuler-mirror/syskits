#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, ctype_locale: &str, operand: &OsString) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .arg("--")
        .arg(operand)
        .stdin(Stdio::null())
        .env_remove("LC_ALL")
        .env("LC_MESSAGES", "C")
        .env("LC_CTYPE", ctype_locale)
        .env("LANG", "C");
    command.output().expect("run hostid")
}

#[test]
fn legacy_locale_operand_quoting_matches_gnu() {
    let cases = [
        ("en_US.iso88591", vec![0xe4]),
        ("en_US.iso88591", vec![0xff]),
        ("en_US.iso88591", b"\xe4\xb8\xad\xe6\x96\x87".to_vec()),
        ("zh_CN.gbk", b"\xd6\xd0\xce\xc4".to_vec()),
        ("zh_CN.gbk", vec![0xff]),
        ("zh_CN.gb18030", b"plain".to_vec()),
        ("zh_CN.gb18030", b"\xd6\xd0\xce\xc4".to_vec()),
        ("zh_CN.gb18030", b"\xa1\xaf".to_vec()),
        ("zh_CN.gb18030", vec![0xff]),
        ("zh_TW.big5", b"\xa4\xa4\xa4\xe5".to_vec()),
    ];

    for (locale, operand) in cases {
        let operand = OsString::from_vec(operand);
        let gnu = run_hostid("/usr/bin/hostid", locale, &operand);
        let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, &operand);

        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "{locale}: {operand:?}"
        );
        assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {operand:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {operand:?}");
    }
}
