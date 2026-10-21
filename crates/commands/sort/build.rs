fn main() {
    println!("cargo:rerun-if-changed=src/general_numeric.c");

    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        cc::Build::new()
            .file("src/general_numeric.c")
            .compile("ct_sort_general_numeric");
    }
}
