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

#[cfg(unix)]
#[test]
fn chgrp_reports_one_error_for_an_inaccessible_recursive_directory() {
    use std::fs::{self, Permissions};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    let inaccessible = tree.join("no-x");
    fs::create_dir_all(inaccessible.join("y")).expect("create tree");
    fs::write(inaccessible.join("y/file"), b"data").expect("create regular file");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    let ownership = Command::new("chown")
        .args(["-R", "65534:65534"])
        .arg(&tree)
        .status()
        .expect("make nobody own the tree");
    assert!(ownership.success());
    fs::set_permissions(&inaccessible, Permissions::from_mode(0o600))
        .expect("remove directory search permission");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-R", "65534", "d"])
        .output()
        .expect("run chgrp as nobody");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot access 'd/no-x/y': Permission denied\n"
    );
}

#[cfg(unix)]
#[test]
fn chgrp_reports_recursive_unreadable_directories_like_gnu() {
    use std::fs::{self, Permissions};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    let unreadable = tree.join("no-read");
    fs::create_dir_all(&unreadable).expect("create tree");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    let ownership = Command::new("chown")
        .args(["-R", "65534:65534"])
        .arg(&tree)
        .status()
        .expect("make nobody own the tree");
    assert!(ownership.success());
    fs::set_permissions(&unreadable, Permissions::from_mode(0o011))
        .expect("remove directory read permission");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-R", "65534", "d"])
        .output()
        .expect("run chgrp as nobody");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot read directory 'd/no-read': Permission denied\n"
    );
}

#[cfg(unix)]
#[test]
fn chgrp_reports_followed_unreadable_symlink_directories_like_gnu() {
    use std::fs::{self, Permissions};
    use std::os::unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    };
    use std::process::Command;
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    let target = tree.join("target");
    fs::create_dir_all(&target).expect("create tree");
    fs::write(target.join("file"), b"data").expect("create regular file");
    symlink("target", tree.join("link")).expect("create directory symlink");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    let ownership = Command::new("chown")
        .args(["-R", "65534:65534"])
        .arg(&tree)
        .status()
        .expect("make nobody own the tree");
    assert!(ownership.success());
    fs::set_permissions(&target, Permissions::from_mode(0o011))
        .expect("remove directory read permission");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-RL", "65534", "d"])
        .output()
        .expect("run chgrp as nobody");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("chgrp: cannot read directory 'd/link': Permission denied\n"));
    assert!(stderr.contains("chgrp: cannot read directory 'd/target': Permission denied\n"));
    assert!(!stderr.contains("os error"));

    let verbose = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-RLv", "65534", "d"])
        .output()
        .expect("run verbose chgrp as nobody");

    assert_eq!(verbose.status.code(), Some(1));
    let stdout = String::from_utf8(verbose.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("failed to change group of 'd/link' to 65534\n"));
    assert!(stdout.contains("failed to change group of 'd/target' to 65534\n"));
    assert!(stdout.contains("group of 'd' retained as nobody\n"));
    let stderr = String::from_utf8(verbose.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("chgrp: cannot read directory 'd/link': Permission denied\n"));
    assert!(stderr.contains("chgrp: cannot read directory 'd/target': Permission denied\n"));
}

#[cfg(unix)]
#[test]
fn chgrp_force_silent_does_not_reprocess_unreadable_directories() {
    use std::fs::{self, Permissions};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    let unreadable = tree.join("no-read");
    fs::create_dir_all(&unreadable).expect("create tree");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    let ownership = Command::new("chown")
        .args(["-R", "65534:65534"])
        .arg(&tree)
        .status()
        .expect("make nobody own the tree");
    assert!(ownership.success());
    fs::set_permissions(&unreadable, Permissions::from_mode(0o011))
        .expect("remove directory read permission");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-Rfv", "65534", "d"])
        .output()
        .expect("run chgrp as nobody");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "failed to change group of 'd/no-read' to 65534\n\
         group of 'd' retained as nobody\n"
    );
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn chgrp_does_not_process_a_recursive_unreadable_root_directory() {
    use std::fs::{self, Permissions};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    fs::create_dir(&tree).expect("create tree");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    let ownership = Command::new("chown")
        .args(["65534:65534"])
        .arg(&tree)
        .status()
        .expect("make nobody own the tree");
    assert!(ownership.success());
    fs::set_permissions(&tree, Permissions::from_mode(0o011))
        .expect("remove directory read permission");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .uid(65534)
        .gid(65534)
        .args(["-Rv", "65534", "d"])
        .output()
        .expect("run chgrp as nobody");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "failed to change group of 'd' to 65534\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot read directory 'd': Permission denied\n"
    );
}
