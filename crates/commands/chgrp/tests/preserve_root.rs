#[cfg(unix)]
#[test]
fn chgrp_preserve_root_checks_followed_nested_links_before_recursing() {
    use std::fs::{self, Permissions};
    use std::os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        process::CommandExt,
    };
    use std::path::Path;
    use std::process::{Command, Output};
    use tempfile::tempdir;

    if unsafe { ctcore::libc::geteuid() } != 0 {
        return;
    }

    fn run_as_nobody(temp: &Path, args: &[&str]) -> Output {
        Command::new("/usr/bin/timeout")
            .current_dir(temp)
            .env("LC_ALL", "C")
            .uid(65534)
            .gid(65534)
            .args([
                "--kill-after=1s",
                "3s",
                "bash",
                "-c",
                "exec -a chgrp \"$0\" \"$@\"",
                env!("CARGO_BIN_EXE_chgrp"),
            ])
            .args(args)
            .output()
            .expect("run chgrp as nobody with a timeout")
    }

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("d");
    let root_link = tree.join("slink-to-root");
    fs::create_dir(&tree).expect("create tree");
    symlink("/", &root_link).expect("create root symlink");
    fs::set_permissions(temp.path(), Permissions::from_mode(0o755))
        .expect("make temporary directory searchable");

    for (args, path) in [
        (["65534:65534"].as_slice(), tree.as_path()),
        (["-h", "65534:65534"].as_slice(), root_link.as_path()),
    ] {
        let status = Command::new("chown")
            .args(args)
            .arg(path)
            .status()
            .expect("make nobody own the test path");
        assert!(status.success());
    }

    let no_follow = run_as_nobody(temp.path(), &["-RHh", "--preserve-root", "65534", "d"]);
    assert_eq!(no_follow.status.code(), Some(0));
    assert!(no_follow.stdout.is_empty());
    assert!(no_follow.stderr.is_empty());

    let follow = run_as_nobody(temp.path(), &["-RLh", "--preserve-root", "65534", "d"]);
    assert_eq!(follow.status.code(), Some(1));
    assert!(follow.stdout.is_empty());
    assert_eq!(
        String::from_utf8(follow.stderr).expect("stderr is UTF-8"),
        "chgrp: it is dangerous to operate recursively on 'd/slink-to-root' (same as '/')\n\
         chgrp: use --no-preserve-root to override this failsafe\n"
    );

    fs::remove_file(&root_link).expect("remove direct root symlink");
    let nested = tree.join("a");
    let nested_root_link = nested.join("slink-to-root");
    fs::create_dir(&nested).expect("create nested directory");
    symlink("/", &nested_root_link).expect("create nested root symlink");
    for path in [&tree, &nested] {
        let status = Command::new("chown")
            .args(["65534:0"])
            .arg(path)
            .status()
            .expect("reset nested directory ownership");
        assert!(status.success());
    }
    let status = Command::new("chown")
        .args(["-h", "65534:0"])
        .arg(&nested_root_link)
        .status()
        .expect("reset nested root symlink ownership");
    assert!(status.success());

    let nested_follow = run_as_nobody(temp.path(), &["-RLh", "--preserve-root", "65534", "d"]);
    assert_eq!(nested_follow.status.code(), Some(1));
    assert!(nested_follow.stdout.is_empty());
    assert_eq!(
        String::from_utf8(nested_follow.stderr).expect("stderr is UTF-8"),
        "chgrp: it is dangerous to operate recursively on 'd/a/slink-to-root' (same as '/')\n\
         chgrp: use --no-preserve-root to override this failsafe\n"
    );
    assert_eq!(
        fs::metadata(&tree).expect("read tree metadata").gid(),
        65534
    );
    assert_eq!(
        fs::metadata(&nested)
            .expect("read nested directory metadata")
            .gid(),
        65534
    );
    assert_eq!(
        fs::symlink_metadata(&nested_root_link)
            .expect("read nested root symlink metadata")
            .gid(),
        0
    );
}
