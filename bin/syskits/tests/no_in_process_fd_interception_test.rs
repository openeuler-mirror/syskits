use std::fs;
use std::path::{Path, PathBuf};

fn forbidden_code_patterns() -> Vec<String> {
    [
        &[
            99, 97, 112, 116, 117, 114, 101, 95, 115, 116, 100, 111, 117, 116, 95, 98, 121, 116,
            101, 115,
        ][..],
        &[
            99, 97, 112, 116, 117, 114, 101, 95, 115, 116, 100, 111, 117, 116, 95, 98, 121, 116,
            101, 115, 95, 119, 105, 116, 104, 95, 115, 97, 118, 101, 100, 95, 115, 116, 100, 111,
            117, 116,
        ][..],
        &[
            99, 97, 112, 116, 117, 114, 101, 95, 115, 116, 100, 111, 117, 116, 95, 98, 121, 116,
            101, 115, 95, 119, 105, 116, 104, 95, 115, 116, 100, 105, 110, 95, 119, 114, 105, 116,
            101, 114,
        ][..],
        &[
            83, 79, 82, 84, 95, 67, 65, 80, 84, 85, 82, 69, 95, 83, 84, 68, 79, 85, 84,
        ][..],
        &[
            83, 80, 76, 73, 84, 95, 67, 65, 80, 84, 85, 82, 69, 95, 83, 84, 68, 79, 85, 84,
        ][..],
        &[
            83, 80, 76, 73, 84, 95, 67, 65, 80, 84, 85, 82, 69, 95, 70, 73, 76, 69, 83,
        ][..],
        &[
            115, 111, 114, 116, 95, 119, 105, 116, 104, 95, 99, 97, 112, 116, 117, 114, 101, 100,
            95, 115, 116, 100, 111, 117, 116,
        ][..],
        &[
            115, 112, 108, 105, 116, 95, 99, 97, 112, 116, 117, 114, 101, 95, 114, 117, 110, 116,
            105, 109, 101,
        ][..],
        &[
            100, 105, 114, 101, 99, 116, 95, 99, 97, 112, 116, 117, 114, 101,
        ][..],
        &[116, 97, 105, 108, 95, 99, 97, 112, 116, 117, 114, 101][..],
        &[
            99, 97, 112, 116, 117, 114, 101, 100, 95, 111, 117, 116, 112, 117, 116,
        ][..],
    ]
    .into_iter()
    .map(|bytes| String::from_utf8(bytes.to_vec()).expect("forbidden pattern is utf-8"))
    .collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("bin/syskits should be two levels below repo root")
        .to_path_buf()
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn repository_code_does_not_use_in_process_stdout_fd_interception() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_rs_files(&root.join("crates"), &mut files);
    collect_rs_files(&root.join("bin"), &mut files);

    let this_test = Path::new(file!());
    let forbidden_code_patterns = forbidden_code_patterns();
    let mut violations = Vec::new();

    for path in files {
        if path.ends_with(this_test) {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read source file");
        for pattern in &forbidden_code_patterns {
            if text.contains(pattern) {
                violations.push(format!(
                    "{} contains forbidden in-process stdout FD interception pattern `{}`",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    pattern
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "in-process stdout FD interception must not be used:\n{}",
        violations.join("\n")
    );
}
