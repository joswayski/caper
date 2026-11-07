//! JNI functions for `chat.caper.android.data.CaperMobile`. Java strings are
//! UTF-16; a lone surrogate (mid-edit) becomes U+FFFD, one unit as well, so
//! every offset still lines up with the Kotlin string.
#![allow(unsafe_code)]

use jni_sys::{JNIEnv, jclass, jint, jintArray, jsize, jstring};

/// # Safety
/// `env` must be the calling thread's JNI environment and `text` a live string.
unsafe fn spans(env: *mut JNIEnv, text: jstring, parse: fn(&str, bool) -> Vec<i32>) -> jintArray {
    // SAFETY: a JNI environment always carries its function table.
    let functions = unsafe { &**env };
    let (Some(length), Some(region), Some(new_array), Some(set_region)) = (
        functions.GetStringLength,
        functions.GetStringRegion,
        functions.NewIntArray,
        functions.SetIntArrayRegion,
    ) else {
        return std::ptr::null_mut();
    };
    // SAFETY: `text` is a live string reference for this native call.
    let len = unsafe { length(env, text) };
    let mut units = vec![0u16; usize::try_from(len).unwrap_or_default()];
    // SAFETY: `units` holds exactly `len` UTF-16 units.
    unsafe { region(env, text, 0, len, units.as_mut_ptr()) };
    let values = parse(&String::from_utf16_lossy(&units), false);
    let Ok(count) = jsize::try_from(values.len()) else {
        return std::ptr::null_mut();
    };
    // SAFETY: on failure NewIntArray returns null with OutOfMemoryError pending,
    // which Kotlin throws when this call returns.
    let array = unsafe { new_array(env, count) };
    if !array.is_null() {
        // SAFETY: `array` has room for `count` ints from `values`.
        unsafe { set_region(env, array, 0, count, values.as_ptr().cast::<jint>()) };
    }
    array
}

/// `CaperMobile.formatSpans(text)`: `crate::format_spans(text, false)`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_formatSpans(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    unsafe { spans(env, text, crate::format_spans) }
}

/// `CaperMobile.linkSpans(text)`: `crate::link_spans(text, false)`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_linkSpans(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    unsafe { spans(env, text, crate::link_spans) }
}
