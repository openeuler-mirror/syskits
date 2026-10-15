#[cfg(unix)]
#[test]
fn invalid_modes_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for mode in ["invalid", "999", "", "0755,u+s"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .env("LC_ALL", "C")
            .args(["-m", mode, "directory"])
            .output()
            .expect("run mkdir with an invalid mode");

        assert_eq!(output.status.code(), Some(1), "{mode:?}");
        assert!(output.stdout.is_empty(), "{mode:?}");
        assert_eq!(
            output.stderr,
            format!("mkdir: invalid mode '{mode}'\n").as_bytes(),
            "{mode:?}"
        );
    }
}
