#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struct_sizes() {
        // Ensure C structs have reasonable sizes (not zero)
        assert!(std::mem::size_of::<lh_position_t>() > 0);
        assert!(std::mem::size_of::<lh_web_color_t>() > 0);
        assert!(std::mem::size_of::<lh_container_vtable_t>() > 0);
    }

    #[test]
    fn test_null_document() {
        unsafe {
            // Passing null should not crash
            lh_document_destroy(std::ptr::null_mut());
            assert_eq!(lh_document_width(std::ptr::null()), 0.0);
            assert_eq!(lh_document_height(std::ptr::null()), 0.0);
        }
    }
}

// Test-only, thread-scoped controls. Production binaries do not export these.
#[cfg(feature = "shep-test-support")]
pub mod table_layout_test {
    unsafe extern "C" {
        fn shep_table_layout_test_mode(enabled: bool);
        fn shep_table_layout_test_count(hits: bool) -> u64;
    }
    pub struct Mode(std::marker::PhantomData<std::rc::Rc<()>>);
    impl Mode {
        pub fn new(enabled: bool) -> Self {
            // No pointers, global environment, or state shared with other threads.
            unsafe { shep_table_layout_test_mode(enabled) };
            Self(std::marker::PhantomData)
        }
        pub fn counts(&self) -> (u64, u64) {
            unsafe { (shep_table_layout_test_count(false), shep_table_layout_test_count(true)) }
        }
    }
    impl Drop for Mode {
        fn drop(&mut self) {
            unsafe { shep_table_layout_test_mode(true) };
        }
    }
}
