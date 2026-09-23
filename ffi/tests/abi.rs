use std::ffi::CStr;

#[test]
fn pd_version_returns_core_version() {
    // SAFETY: pd_version returns a pointer to a static NUL-terminated string.
    let version = unsafe { CStr::from_ptr(perldantic_ffi::pd_version()) };
    assert_eq!(version.to_str().unwrap(), perldantic_core::VERSION);
}

#[test]
fn pd_version_is_stable_across_calls() {
    assert_eq!(perldantic_ffi::pd_version(), perldantic_ffi::pd_version());
}
