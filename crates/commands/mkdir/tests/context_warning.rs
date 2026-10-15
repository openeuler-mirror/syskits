#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn explicit_context_warns_once_before_operand_diagnostic() {
    if selinux::kernel_support() != selinux::KernelSupport::Unsupported {
        return;
    }

    for option in ["--context=label", "--context="] {
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .env("LC_ALL", "C")
            .arg(option)
            .output()
            .expect("run mkdir with an explicit context and no operands");

        assert_eq!(output.status.code(), Some(1), "{option}");
        assert!(output.stdout.is_empty(), "{option}");
        assert_eq!(
            output.stderr,
            b"mkdir: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel\nmkdir: missing operand\nTry 'mkdir --help' for more information.\n",
            "{option}"
        );
    }
}

#[test]
fn explicit_context_warns_once_for_multiple_directories() {
    if selinux::kernel_support() != selinux::KernelSupport::Unsupported {
        return;
    }

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["--context=label", "first", "second"])
        .output()
        .expect("run mkdir with an explicit context and multiple operands");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel\n"
    );
    assert!(temp_dir.path().join("first").is_dir());
    assert!(temp_dir.path().join("second").is_dir());
}
