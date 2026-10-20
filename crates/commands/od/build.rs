fn main() {
    println!("cargo:rerun-if-changed=src/long_double.c");
    cc::Build::new()
        .file("src/long_double.c")
        .compile("ct_od_long_double");
}
