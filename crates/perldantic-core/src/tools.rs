//! Small helpers ported from upstream `tools.rs`.

use std::fmt;

/// Write `val` to `f`, eliding the middle with `...` when it is longer than `max_len` bytes.
///
/// Port of upstream `write_truncated_to_limited_bytes`; cuts only on char boundaries.
pub fn write_truncated_to_limited_bytes<F: fmt::Write>(
    f: &mut F,
    val: &str,
    max_len: usize,
) -> fmt::Result {
    if val.len() > max_len {
        let mid_point = max_len.div_ceil(2);
        write!(
            f,
            "{}...{}",
            &val[0..floor_char_boundary(val, mid_point)],
            &val[ceil_char_boundary(val, val.len() - (mid_point - 1))..]
        )
    } else {
        write!(f, "{val}")
    }
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        value.len()
    } else {
        (0..=index)
            .rev()
            .find(|&i| value.is_char_boundary(i))
            .unwrap_or(0)
    }
}

fn ceil_char_boundary(value: &str, index: usize) -> usize {
    (index..value.len())
        .find(|&i| value.is_char_boundary(i))
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use super::write_truncated_to_limited_bytes;

    fn truncate(val: &str, max_len: usize) -> String {
        let mut out = String::new();
        write_truncated_to_limited_bytes(&mut out, val, max_len).unwrap();
        out
    }

    #[test]
    fn short_values_are_unchanged() {
        assert_eq!(truncate("abc", 50), "abc");
        assert_eq!(truncate(&"x".repeat(50), 50), "x".repeat(50));
    }

    #[test]
    fn long_values_keep_head_and_tail() {
        assert_eq!(truncate("0123456789", 5), "012...89");
    }

    #[test]
    fn cuts_respect_multibyte_characters() {
        // "é" is two bytes; a naive byte cut would panic.
        let out = truncate(&"é".repeat(10), 5);
        assert_eq!(out, "é...é");
    }
}
