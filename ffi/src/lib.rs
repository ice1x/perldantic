//! C ABI over perldantic-core, consumed by the Perldantic Perl distribution.

use std::ffi::c_char;

/// NUL-terminated copy of [`perldantic_core::VERSION`], built at compile time.
const VERSION_CSTR: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

/// Returns the library version as a static NUL-terminated string.
///
/// The pointer is valid for the lifetime of the process and must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn pd_version() -> *const c_char {
    VERSION_CSTR.as_ptr().cast()
}
