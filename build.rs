fn main() {
    println!("cargo:rerun-if-changed=assets/windows/shep.rc");
    println!("cargo:rerun-if-changed=assets/windows/shep.manifest");
    // Common Controls v6 exports the window subclassing API by name; without
    // this manifest no Windows binary, tests included, can start.
    let embedded =
        embed_resource::compile_for_everything("assets/windows/shep.rc", embed_resource::NONE);
    if let Err(error) = embedded.manifest_required() {
        eprintln!("Could not embed the Windows application manifest: {error}");
        std::process::exit(1);
    }
}
