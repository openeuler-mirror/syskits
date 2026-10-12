#[cfg(unix)]
#[test]
fn chgrp_distinguishes_missing_paths_from_dangling_symlinks() {
    use std::os::unix::{fs::symlink, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let missing = temp_dir.path().join("missing");
    let dangling = temp_dir.path().join("dangling");
    symlink("missing", &dangling).expect("create dangling symlink");

    for (path, operation) in [(&missing, "access"), (&dangling, "dereference")] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args(["0"])
            .arg(path)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chgrp: cannot {operation} '{}': No such file or directory\n",
                path.display()
            )
        );
    }
}
