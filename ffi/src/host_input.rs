//! Host data read in place through a table of C functions: the Perl side (Perldantic.xs) hands
//! the validator its own values, and the core reads arrays and hashes where they are instead
//! of receiving a copy in the binary wire format ([`crate::binary`]).
//!
//! Nodes are opaque pointers to host values (Perl SVs) that stay valid for the whole call.
//! Everything the core converts to a `Value` (scalars, objects, tied containers) is written by
//! the host in the binary wire format, so conversions are those of the binary transport.

use std::cell::RefCell;
use std::ffi::c_void;

use perldantic_core::{HostData, HostKind, Value};

use crate::binary;

/// The functions a host provides; `ctx` is passed back to each of them.
#[repr(C)]
pub struct PdHost {
    pub ctx: *mut c_void,
    /// 1 for an array read in place, 2 for a hash read in place, 0 for anything else.
    pub kind: unsafe extern "C" fn(ctx: *mut c_void, node: *mut c_void) -> i32,
    pub array_len: unsafe extern "C" fn(ctx: *mut c_void, node: *mut c_void) -> usize,
    pub array_item:
        unsafe extern "C" fn(ctx: *mut c_void, node: *mut c_void, index: usize) -> *mut c_void,
    /// The value under a key (UTF-8 bytes), or null.
    pub hash_get: unsafe extern "C" fn(
        ctx: *mut c_void,
        node: *mut c_void,
        key: *const u8,
        key_len: usize,
    ) -> *mut c_void,
    pub hash_len: unsafe extern "C" fn(ctx: *mut c_void, node: *mut c_void) -> usize,
    /// Fill up to `capacity` entries, in the order to visit them: key (UTF-8 bytes) and value.
    /// Returns how many were written.
    pub hash_entries: unsafe extern "C" fn(
        ctx: *mut c_void,
        node: *mut c_void,
        keys: *mut *const u8,
        key_lens: *mut usize,
        values: *mut *mut c_void,
        capacity: usize,
    ) -> usize,
    /// Describe a plain scalar (see [`PdScalar`]); returns 0 for anything else, which is then
    /// converted with `to_binary`.
    pub scalar:
        unsafe extern "C" fn(ctx: *mut c_void, node: *mut c_void, out: *mut PdScalar) -> i32,
    /// Write the node in the binary wire format; the bytes stay valid for the call. Returns 0
    /// when the host cannot convert the node (it keeps the error and raises it afterwards).
    pub to_binary: unsafe extern "C" fn(
        ctx: *mut c_void,
        node: *mut c_void,
        bytes: *mut *const u8,
        len: *mut usize,
    ) -> i32,
}

/// A node of the host: an opaque pointer to one of its values.
#[derive(Debug, Clone, Copy)]
pub struct HostNode(pub *mut c_void);

/// A plain scalar of the host, described without allocating: `tag` 0 for `None`, 1 and 2 for
/// `true` and `false`, 3 for the integer `int`, 4 for the float `float`, 5 for the UTF-8 string
/// at `ptr` (`len` bytes, valid for the call).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PdScalar {
    pub tag: i32,
    pub int: i64,
    pub float: f64,
    pub ptr: *const u8,
    pub len: usize,
}

/// [`HostData`] over a [`PdHost`].
pub struct PerlHost<'a> {
    host: &'a PdHost,
    /// Whether a conversion failed (the host holds the error).
    failed: RefCell<bool>,
}

impl<'a> PerlHost<'a> {
    /// # Safety
    /// The functions of `host` accept every node the host gives (the root and the nodes they
    /// return) for as long as this value lives.
    pub unsafe fn new(host: &'a PdHost) -> Self {
        Self {
            host,
            failed: RefCell::new(false),
        }
    }

    /// Whether a conversion failed during the call.
    pub fn failed(&self) -> bool {
        *self.failed.borrow()
    }
}

impl HostData for PerlHost<'_> {
    type Node = HostNode;

    fn kind(&self, node: Self::Node) -> HostKind {
        // SAFETY: the host's functions take any node it gave.
        match unsafe { (self.host.kind)(self.host.ctx, node.0) } {
            1 => HostKind::Array,
            2 => HostKind::Hash,
            _ => HostKind::Value,
        }
    }

    fn to_value(&self, node: Self::Node) -> Value {
        let mut scalar = PdScalar {
            tag: -1,
            int: 0,
            float: 0.0,
            ptr: std::ptr::null(),
            len: 0,
        };
        // SAFETY: as above; the host fills `scalar`.
        if unsafe { (self.host.scalar)(self.host.ctx, node.0, &raw mut scalar) } != 0 {
            match scalar.tag {
                0 => return Value::None,
                1 => return Value::Bool(true),
                2 => return Value::Bool(false),
                3 => return Value::Int(scalar.int),
                4 => return Value::Float(scalar.float),
                5 if !scalar.ptr.is_null() => {
                    // SAFETY: the host keeps `len` bytes of UTF-8 at `ptr` for the call.
                    let bytes = unsafe { std::slice::from_raw_parts(scalar.ptr, scalar.len) };
                    if let Ok(text) = std::str::from_utf8(bytes) {
                        return Value::Str(text.to_owned());
                    }
                }
                _ => {}
            }
        }
        let mut bytes = std::ptr::null();
        let mut len = 0;
        // SAFETY: as above; the host writes a pointer and a length.
        let ok =
            unsafe { (self.host.to_binary)(self.host.ctx, node.0, &raw mut bytes, &raw mut len) };
        if ok == 0 || bytes.is_null() {
            *self.failed.borrow_mut() = true;
            return Value::None;
        }
        // SAFETY: the host keeps `len` bytes at `bytes` for the call.
        let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
        binary::decode(slice).unwrap_or_else(|_| {
            *self.failed.borrow_mut() = true;
            Value::None
        })
    }

    fn array_len(&self, node: Self::Node) -> usize {
        // SAFETY: as above.
        unsafe { (self.host.array_len)(self.host.ctx, node.0) }
    }

    fn array_item(&self, node: Self::Node, index: usize) -> Self::Node {
        // SAFETY: as above; the host gives a node for every index below the length.
        HostNode(unsafe { (self.host.array_item)(self.host.ctx, node.0, index) })
    }

    fn hash_get(&self, node: Self::Node, key: &str) -> Option<Self::Node> {
        // SAFETY: as above; the key is readable for the call.
        let found = unsafe { (self.host.hash_get)(self.host.ctx, node.0, key.as_ptr(), key.len()) };
        (!found.is_null()).then_some(HostNode(found))
    }

    fn hash_entries(&self, node: Self::Node) -> Vec<(String, Self::Node)> {
        // SAFETY: as above.
        let capacity = unsafe { (self.host.hash_len)(self.host.ctx, node.0) };
        let mut keys = vec![std::ptr::null(); capacity];
        let mut key_lens = vec![0usize; capacity];
        let mut values = vec![std::ptr::null_mut(); capacity];
        // SAFETY: the host writes at most `capacity` entries into the three buffers.
        let count = unsafe {
            (self.host.hash_entries)(
                self.host.ctx,
                node.0,
                keys.as_mut_ptr(),
                key_lens.as_mut_ptr(),
                values.as_mut_ptr(),
                capacity,
            )
        };
        (0..count.min(capacity))
            .map(|i| {
                // SAFETY: the host wrote a readable key of that length for the call.
                let key = unsafe { std::slice::from_raw_parts(keys[i], key_lens[i]) };
                (
                    String::from_utf8_lossy(key).into_owned(),
                    HostNode(values[i]),
                )
            })
            .collect()
    }

    fn identity(&self, node: Self::Node) -> usize {
        node.0 as usize
    }
}
