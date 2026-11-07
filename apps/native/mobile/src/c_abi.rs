//! The C ABI the Apple apps call (`include/caper_mobile.h`). Text arrives as
//! UTF-8, Swift's own encoding, so passing it copies nothing.
#![allow(unsafe_code)]

/// An `i32` array the caller releases with `caper_mobile_ints_free`.
#[repr(C)]
pub struct CaperMobileInts {
    data: *mut i32,
    len: usize,
}

impl From<Vec<i32>> for CaperMobileInts {
    fn from(values: Vec<i32>) -> Self {
        let values = Box::into_raw(values.into_boxed_slice());
        Self {
            data: values.cast(),
            len: values.len(),
        }
    }
}

/// # Safety
/// `utf8` must point to `len` readable bytes, or may be null when `len` is 0.
unsafe fn text<'a>(utf8: *const u8, len: usize) -> &'a str {
    if len == 0 {
        return "";
    }
    // SAFETY: the caller passes `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(utf8, len) };
    // Swift strings are always valid UTF-8.
    std::str::from_utf8(bytes).unwrap_or_default()
}

/// `crate::format_spans(text, true)`: five values per visible span.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_format_spans(utf8: *const u8, len: usize) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::format_spans(unsafe { text(utf8, len) }, true).into()
}

/// `crate::link_spans(text, true)`: five values per link.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_link_spans(utf8: *const u8, len: usize) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::link_spans(unsafe { text(utf8, len) }, true).into()
}

/// `crate::mention_spans(text, true)`: four values per `@mention` token.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_mention_spans(
    utf8: *const u8,
    len: usize,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::mention_spans(unsafe { text(utf8, len) }, true).into()
}

/// `crate::history_diff_spans(before, after, true)`: the before span count,
/// then four values per highlighted span.
///
/// # Safety
/// `before` and `after` must point to `before_len` and `after_len` bytes of
/// UTF-8, or may be null when their length is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_history_diff_spans(
    before: *const u8,
    before_len: usize,
    after: *const u8,
    after_len: usize,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    let (before, after) = unsafe { (text(before, before_len), text(after, after_len)) };
    crate::history_diff_spans(before, after, true).into()
}

/// `crate::composer_edit(previous, next, start, end, continuing)`: the UTF-16
/// selection and pending style, then the edited text's UTF-16 code units.
///
/// # Safety
/// `previous` and `next` must point to `previous_len` and `next_len` bytes of
/// UTF-8, or may be null when their length is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_composer_edit(
    previous: *const u8,
    previous_len: usize,
    next: *const u8,
    next_len: usize,
    start: i32,
    end: i32,
    continuing: i32,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    let (previous, next) = unsafe { (text(previous, previous_len), text(next, next_len)) };
    crate::composer_edit(previous, next, start, end, continuing).into()
}

/// `crate::emoji_catalog(json)`: loads the catalog the app ships.
///
/// # Safety
/// `json` must point to `len` readable bytes, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_emoji_catalog(json: *const u8, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    // SAFETY: the caller passes `len` readable bytes.
    crate::emoji_catalog(unsafe { std::slice::from_raw_parts(json, len) })
}

/// `crate::emoji_token(text, caret)`: the UTF-16 token range, then its query.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_emoji_token(
    utf8: *const u8,
    len: usize,
    caret: i32,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::emoji_token(unsafe { text(utf8, len) }, caret).into()
}

/// `crate::emoji_suggestions(query)`: catalog positions, best first.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_emoji_suggestions(
    utf8: *const u8,
    len: usize,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::emoji_suggestions(unsafe { text(utf8, len) }).into()
}

/// `crate::names::has_hidden(text)`: whether a display or space name holds a
/// character names may not.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_name_has_hidden(utf8: *const u8, len: usize) -> bool {
    // SAFETY: forwarded from this function's contract.
    crate::names::has_hidden(unsafe { text(utf8, len) })
}

/// `crate::channel_name(text)`: web's normalized channel name, one ASCII
/// character per value.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_channel_name(utf8: *const u8, len: usize) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::channel_name(unsafe { text(utf8, len) }).into()
}

/// `crate::name_error(text, channel)`: the ASCII message, or nothing.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_name_error(
    utf8: *const u8,
    len: usize,
    channel: bool,
) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::name_error(unsafe { text(utf8, len) }, channel).into()
}

/// `crate::space_name_span(text, true)`: the trimmed name's four offsets.
///
/// # Safety
/// `utf8` must point to `len` bytes of UTF-8, or may be null when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_space_name(utf8: *const u8, len: usize) -> CaperMobileInts {
    // SAFETY: forwarded from this function's contract.
    crate::space_name_span(unsafe { text(utf8, len) }, true).into()
}

/// # Safety
/// `ints` must come from this library and be released only once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caper_mobile_ints_free(ints: CaperMobileInts) {
    // SAFETY: `ints` is a boxed slice this library leaked in `From<Vec<i32>>`.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ints.data, ints.len)) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_round_trip_through_the_c_abi() {
        let text = "a **b** www.c.d.e";
        // SAFETY: a valid UTF-8 buffer and its length; each result is freed once.
        unsafe {
            let spans = caper_mobile_format_spans(text.as_ptr(), text.len());
            assert_eq!(
                std::slice::from_raw_parts(spans.data, spans.len),
                crate::format_spans(text, true)
            );
            caper_mobile_ints_free(spans);
            let links = caper_mobile_link_spans(text.as_ptr(), text.len());
            assert_eq!(
                std::slice::from_raw_parts(links.data, links.len),
                [8, 17, 8, 17, 0]
            );
            caper_mobile_ints_free(links);
            let empty = caper_mobile_format_spans(std::ptr::null(), 0);
            assert_eq!(empty.len, 0);
            caper_mobile_ints_free(empty);
        }
    }
}
