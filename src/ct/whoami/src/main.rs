// ctcore::bin!(ct_whoami);
pub fn main() {
    use std::io::Write;

    ctcore::ct_panic::ct_mute_set_panic_hook();
    let code = ct_whoami::ctmain(ctcore::ct_os_args());
    if let Err(e) = std::io::stdout().flush() {
        {
            eprintln!("Error flushing stdout: {}", e);
        };
    }
    std::process::exit(code);
}
