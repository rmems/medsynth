//! Locator-convention syntax for registry sources.
//!
//! Each entry in `docs/sources.json` publishes a `locator_convention.format`
//! string; this module maps those strings to the regexes that check an
//! anchor's `locator` conforms. This is syntax only — resolving a
//! chapter/section and checking that the cited passage supports the claim is
//! the verification pipeline's job (#9), not this crate's.
//!
//! The patterns end with `(?![\s\S])` — a true end-of-input assertion —
//! because `$` in ECMA-style regexes can match before a final newline. The
//! lookahead needs an engine that supports it (ECMA, or fancy-regex as used
//! by the `jsonschema` crate).

/// The regex for a registry `locator_convention.format` string, or `None`
/// when the convention is unrecognized — an unknown convention is a registry
/// defect, and callers should treat it as one rather than skipping the check.
pub fn pattern(convention: &str) -> Option<&'static str> {
    Some(match convention {
        "ch<N>:<section-slug> or p.<page>" => {
            r"^(ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*|p\.[1-9][0-9]*)(?![\s\S])"
        }
        "ch<N>:<section-slug> or sec<N>.<M>:<section-slug>" => {
            r"^(ch[1-9][0-9]*|sec[1-9][0-9]*\.[1-9][0-9]*):[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])"
        }
        "ch<N>:<section-slug>" => r"^ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])",
        "vol<N>:ch<N>:<section-slug>" => {
            r"^vol[1-3]:ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])"
        }
        "sec<N>.<M>:<section-slug> or rec<N>" => {
            r"^(sec[1-9][0-9]*\.[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*|rec[1-9][0-9]*)(?![\s\S])"
        }
        "sec<N>[.<M>][A-Z]:<section-slug>" => {
            r"^sec[1-9][0-9]*(\.[1-9][0-9]*[A-Z]?)?:[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])"
        }
        "sec<N>:<title-slug>@YYYY-MM-DD" => {
            r"^sec[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*@[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])(?![\s\S])"
        }
        "rec<N> or sec<N>:<title-slug>" => {
            r"^(rec[1-9][0-9]*|sec[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*)(?![\s\S])"
        }
        _ => return None,
    })
}
