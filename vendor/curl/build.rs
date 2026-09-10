use std::env;

fn main() {
    println!(
        "cargo:rustc-check-cfg=cfg(\
            need_openssl_init,\
            need_openssl_lifetime_init,\
            need_openssl_probe,\
        )"
    );
    // Shep: modern OpenSSL still needs Rust's NO_ATEXIT lifetime policy before
    // curl_global_init. Keep old-version environment probing/locking unchanged.
    let use_openssl = match env::var("DEP_OPENSSL_VERSION_NUMBER") {
        Ok(version) => {
            let version = u64::from_str_radix(&version, 16).unwrap();
            if version < 0x1_01_00_00_0 {
                println!("cargo:rustc-cfg=need_openssl_init");
            } else {
                println!("cargo:rustc-cfg=need_openssl_lifetime_init");
            }
            true
        }
        Err(_) => false,
    };

    if use_openssl {
        // The system libcurl should have the default certificate paths configured.
        if env::var_os("DEP_CURL_STATIC").is_some() {
            println!("cargo:rustc-cfg=need_openssl_probe");
        }
    }
}
