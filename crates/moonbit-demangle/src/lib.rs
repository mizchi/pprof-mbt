//! Decode MoonBit's symbol mangling into a readable path.
//!
//! MoonBit's compiler emits symbols like `_M0FP26mizchi5bench9ackermann`
//! into every backend (wasm `name` section, JS function names, Mach-O / ELF
//! exports). Each segment is `<decimal length><identifier of that length>`,
//! separated by single-character structural markers (`P`, `B`, `C`, …) and
//! occasional namespace-count digits. There's no published specification,
//! so this crate uses a heuristic: scan the suffix backwards, greedily
//! match `<digits><chars-of-that-length>` segments, and stop when the run
//! is no longer well-formed. It recovers the user-visible package + name
//! on the symbols you typically see in profiles. Impl / trait / generic
//! decorations get partially decoded (the `core::` prefix on stdlib
//! methods, for example, is dropped) — readable, not lossless.
//!
//! # Examples
//!
//! ```
//! use moonbit_demangle::demangle;
//!
//! assert_eq!(
//!     demangle("_M0FP26mizchi5bench9ackermann"),
//!     "mizchi::bench::ackermann",
//! );
//! // Mach-O prepends an underscore; samply's inline-frame `function`
//! // strings strip it. Both forms are accepted:
//! assert_eq!(demangle("__M0FP26mizchi5bench3fib"), "mizchi::bench::fib");
//! assert_eq!(demangle("M0FP26mizchi5bench3fib"), "mizchi::bench::fib");
//! // Non-MoonBit symbols pass through unchanged:
//! assert_eq!(demangle("main"), "main");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

const MANGLE_INNER_PREFIX: &[u8] = b"M0";

/// Decode a MoonBit mangled symbol. Returns the original `name` if it does
/// not look mangled.
///
/// Allocates the return string. For repeated calls, intern through your
/// own cache: the algorithm is `O(n²)` in the symbol length.
pub fn demangle(name: &str) -> String {
    let Some(inner) = strip_prefix(name) else {
        return name.to_string();
    };
    let inner = strip_generic_suffix(inner);
    let bytes = inner.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut i = bytes.len();
    // Hard cap so a pathological input can't make us loop forever.
    for _ in 0..MAX_SEGMENTS {
        if i == 0 {
            break;
        }
        let Some((chars_start, new_i)) = find_segment_end(bytes, i) else {
            break;
        };
        // ASCII-only by construction (is_ident enforced it).
        parts.insert(0, &inner[chars_start..i]);
        i = new_i;
    }
    if parts.is_empty() {
        name.to_string()
    } else {
        parts.join("::")
    }
}

const MAX_SEGMENTS: usize = 50;
const MAX_SEGMENT_LEN: usize = 64;

/// Pin the segment's end to `i` and find the largest `n` such that
/// `bytes[i-n..i]` is an identifier preceded by a digit run whose
/// rightmost suffix parses as exactly `n`.
fn find_segment_end(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let max_n = (i - 1).min(MAX_SEGMENT_LEN);
    for n in (1..=max_n).rev() {
        let chars = &bytes[i - n..i];
        if !is_ident(chars) {
            continue;
        }
        let d_end = i - n;
        let mut d_start = d_end;
        while d_start > 0 && bytes[d_start - 1].is_ascii_digit() {
            d_start -= 1;
        }
        if d_start == d_end {
            continue;
        }
        // The digit prefix may include a leading namespace-count digit
        // (e.g. `26mizchi` = count `2` + length `6`). Try every suffix of
        // the digit run, leftmost first, and accept the one whose decimal
        // value equals `n`.
        let mut buf = itoa_buf();
        let target = itoa(n, &mut buf);
        for ds in d_start..d_end {
            if &bytes[ds..d_end] == target {
                return Some((i - n, ds));
            }
        }
    }
    None
}

/// Accept `_*M0[A-Z]…` and return everything from `M0` onward.
fn strip_prefix(name: &str) -> Option<&str> {
    let trimmed = name.trim_start_matches('_');
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 || &bytes[..2] != MANGLE_INNER_PREFIX {
        return None;
    }
    if !bytes[2].is_ascii_uppercase() {
        return None;
    }
    Some(trimmed)
}

/// Drop a trailing `G<letters>E` generic-instantiation marker.
fn strip_generic_suffix(s: &str) -> &str {
    let bytes = s.as_bytes();
    let end = bytes.len();
    if end < 3 || bytes[end - 1] != b'E' {
        return s;
    }
    let mut i = end - 1;
    while i > 0 && bytes[i - 1].is_ascii_alphabetic() && bytes[i - 1] != b'G' {
        i -= 1;
    }
    if i > 0 && bytes[i - 1] == b'G' && i - 1 < end - 1 {
        &s[..i - 1]
    } else {
        s
    }
}

fn is_ident(s: &[u8]) -> bool {
    if s.is_empty() {
        return false;
    }
    let c = s[0];
    if !(c == b'_' || c.is_ascii_alphabetic()) {
        return false;
    }
    s[1..]
        .iter()
        .all(|&c| c == b'_' || c.is_ascii_alphanumeric())
}

// Small stack-only integer formatter so demangle() doesn't allocate the
// digit-suffix target on every iteration.
type ItoaBuf = [u8; 3]; // u8 max = 255, but MAX_SEGMENT_LEN = 64 so 2 digits suffice
fn itoa_buf() -> ItoaBuf {
    [0; 3]
}
fn itoa(n: usize, buf: &mut ItoaBuf) -> &[u8] {
    debug_assert!(n <= MAX_SEGMENT_LEN);
    if n < 10 {
        buf[0] = b'0' + n as u8;
        &buf[..1]
    } else {
        buf[0] = b'0' + (n / 10) as u8;
        buf[1] = b'0' + (n % 10) as u8;
        &buf[..2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_function() {
        assert_eq!(
            demangle("_M0FP26mizchi5bench9ackermann"),
            "mizchi::bench::ackermann"
        );
    }

    #[test]
    fn double_underscore_in_name() {
        assert_eq!(
            demangle("_M0FP26mizchi5bench11mandel__sum"),
            "mizchi::bench::mandel__sum"
        );
    }

    #[test]
    fn macho_underscore() {
        assert_eq!(demangle("__M0FP26mizchi5bench3fib"), "mizchi::bench::fib");
    }

    #[test]
    fn samply_inline_form() {
        assert_eq!(
            demangle("M0FP26mizchi5bench9ackermann"),
            "mizchi::bench::ackermann"
        );
    }

    #[test]
    fn generic_suffix_dropped() {
        assert_eq!(demangle("_M0FPB7printlnGsE"), "println");
    }

    #[test]
    fn non_mangled_passthrough() {
        assert_eq!(demangle("main"), "main");
        assert_eq!(demangle(""), "");
        assert_eq!(demangle("printc"), "printc");
    }

    #[test]
    fn malformed_does_not_loop() {
        // No length-prefixed segments → returns original.
        assert_eq!(demangle("_M0FPXXXXX"), "_M0FPXXXXX");
    }
}
