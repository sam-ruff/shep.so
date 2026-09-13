#[test]
fn stylesheet_bridge_links_and_accepts_a_null_document() {
    // Calling the bridge retains its parser dependency in the native linker.
    unsafe {
        litehtml_sys::lh_document_add_stylesheet(
            std::ptr::null_mut(),
            c"body { color: red; }".as_ptr(),
            c"".as_ptr(),
            c"".as_ptr(),
        );
    }
}
