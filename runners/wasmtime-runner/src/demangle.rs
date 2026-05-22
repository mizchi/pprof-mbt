//! Decode moonbit's symbol mangling into a readable `a::b::c` path.
//!
//! Mirrors `runners/lib/demangle.mjs` and
//! `runners/wzprof-runner/internal/demangle/demangle.go`. Each segment is
//! `<decimal length><identifier of that length>`; segments are concatenated
//! with optional structural markers (P/B/C, a count digit, etc). We parse
//! from the right end so the leading count digit of `26mizchi5bench…`
//! doesn't masquerade as part of the next length.

const MANGLE_PREFIX_BYTES: &[u8] = b"M0";

pub fn symbol(name: &str) -> String {
    let inner = strip_prefix(name).unwrap_or("");
    if inner.is_empty() {
        return name.to_string();
    }
    let inner = strip_generic_suffix(inner);
    let bytes = inner.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut i = bytes.len();
    for _ in 0..50 {
        if i == 0 {
            break;
        }
        let mut hit: Option<(usize, usize)> = None;
        let max_n = (i - 1).min(64);
        for n in (1..=max_n).rev() {
            let chars = &bytes[i - n..i];
            if !is_ident(chars) {
                continue;
            }
            let d_end = i - n;
            let mut d_start = d_end;
            while d_start > 0 && is_digit(bytes[d_start - 1]) {
                d_start -= 1;
            }
            if d_start == d_end {
                continue;
            }
            // Try each possible split of the preceding digit run; pick the
            // leftmost suffix that parses as exactly `n`.
            let target = n.to_string();
            let target_bytes = target.as_bytes();
            for ds in d_start..d_end {
                if &bytes[ds..d_end] == target_bytes {
                    hit = Some((i - n, ds));
                    break;
                }
            }
            if hit.is_some() {
                break;
            }
        }
        match hit {
            Some((chars_start, new_i)) => {
                // SAFETY: is_ident already ensured the slice is ASCII.
                let s = std::str::from_utf8(&bytes[chars_start..chars_start + (i - chars_start)])
                    .expect("ascii");
                parts.insert(0, s);
                i = new_i;
            }
            None => break,
        }
    }
    if parts.is_empty() {
        name.to_string()
    } else {
        parts.join("::")
    }
}

/// Strip the `_M0X` (or `__M0X` / `M0X`) prefix and return what follows.
fn strip_prefix(name: &str) -> Option<&str> {
    let trimmed = name.trim_start_matches('_');
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 || &bytes[..2] != MANGLE_PREFIX_BYTES {
        return None;
    }
    if !bytes[2].is_ascii_uppercase() {
        return None;
    }
    // Keep the whole "M0X…" body — backward scan handles the structural prefix.
    Some(trimmed)
}

/// Drop a trailing `G<letters>E` generic instantiation marker.
fn strip_generic_suffix(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    if end < 3 || bytes[end - 1] != b'E' {
        return s;
    }
    let mut i = end - 1;
    while i > 0 && bytes[i - 1].is_ascii_alphabetic() && bytes[i - 1] != b'G' {
        i -= 1;
    }
    if i > 0 && bytes[i - 1] == b'G' && i - 1 < end - 1 {
        end = i - 1;
        &s[..end]
    } else {
        s
    }
}

fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_function() {
        assert_eq!(
            symbol("_M0FP26mizchi5bench9ackermann"),
            "mizchi::bench::ackermann"
        );
    }

    #[test]
    fn double_underscore_in_name() {
        assert_eq!(
            symbol("_M0FP26mizchi5bench11mandel__sum"),
            "mizchi::bench::mandel__sum"
        );
    }

    #[test]
    fn macho_underscore() {
        assert_eq!(
            symbol("__M0FP26mizchi5bench3fib"),
            "mizchi::bench::fib"
        );
    }

    #[test]
    fn samply_inline_form() {
        assert_eq!(
            symbol("M0FP26mizchi5bench9ackermann"),
            "mizchi::bench::ackermann"
        );
    }

    #[test]
    fn generic_suffix_dropped() {
        assert_eq!(symbol("_M0FPB7printlnGsE"), "println");
    }

    #[test]
    fn non_mangled_passthrough() {
        assert_eq!(symbol("main"), "main");
        assert_eq!(symbol(""), "");
        assert_eq!(symbol("printc"), "printc");
    }
}
