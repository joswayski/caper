//! JNI functions for `chat.caper.android.data.CaperMobile`. Java strings are
//! UTF-16; a lone surrogate (mid-edit) becomes U+FFFD, one unit as well, so
//! every offset still lines up with the Kotlin string.
#![allow(unsafe_code)]

use jni_sys::{
    JNI_FALSE, JNI_TRUE, JNIEnv, jboolean, jbyte, jbyteArray, jclass, jint, jintArray, jsize,
    jstring,
};

/// Streams cleaned bytes into a new file; status codes match the C ABI.
///
/// # Safety
/// Called by the JVM with its environment and three live strings.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_cleanMetadata(
    env: *mut JNIEnv,
    _class: jclass,
    source: jstring,
    destination: jstring,
    mime: jstring,
) -> jint {
    // SAFETY: the JVM supplies live string references for this call.
    let values = unsafe {
        (
            string(env, source),
            string(env, destination),
            string(env, mime),
        )
    };
    match values {
        (Some(source), Some(destination), Some(mime)) => {
            crate::media::clean_metadata(&source, &destination, &mime)
        }
        _ => 3,
    }
}

/// # Safety
/// `env` must be the calling thread's JNI environment and `text` a live string.
unsafe fn string(env: *mut JNIEnv, text: jstring) -> Option<String> {
    // SAFETY: a JNI environment always carries its function table.
    let functions = unsafe { &**env };
    let (Some(length), Some(region)) = (functions.GetStringLength, functions.GetStringRegion)
    else {
        return None;
    };
    // SAFETY: `text` is a live string reference for this native call.
    let len = unsafe { length(env, text) };
    let mut units = vec![0u16; usize::try_from(len).unwrap_or_default()];
    // SAFETY: `units` holds exactly `len` UTF-16 units.
    unsafe { region(env, text, 0, len, units.as_mut_ptr()) };
    Some(String::from_utf16_lossy(&units))
}

/// # Safety
/// `env` must be the calling thread's JNI environment.
unsafe fn int_array(env: *mut JNIEnv, values: &[i32]) -> jintArray {
    // SAFETY: a JNI environment always carries its function table.
    let functions = unsafe { &**env };
    let (Some(new_array), Some(set_region)) = (functions.NewIntArray, functions.SetIntArrayRegion)
    else {
        return std::ptr::null_mut();
    };
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

/// # Safety
/// `env` must be the calling thread's JNI environment and `text` a live string.
unsafe fn spans(env: *mut JNIEnv, text: jstring, parse: fn(&str, bool) -> Vec<i32>) -> jintArray {
    // SAFETY: forwarded from this function's contract.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from this function's contract.
        Some(text) => unsafe { int_array(env, &parse(&text, false)) },
        None => std::ptr::null_mut(),
    }
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

/// `CaperMobile.mentionSpans(text)`: `crate::mention_spans(text, false)`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_mentionSpans(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    unsafe { spans(env, text, crate::mention_spans) }
}

/// `CaperMobile.historyDiffSpans(before, after)`:
/// `crate::history_diff_spans(before, after, false)`.
///
/// # Safety
/// Called by the JVM with its environment and two live strings.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_historyDiffSpans(
    env: *mut JNIEnv,
    _class: jclass,
    before: jstring,
    after: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { (string(env, before), string(env, after)) } {
        // SAFETY: forwarded from the JVM's call.
        (Some(before), Some(after)) => unsafe {
            int_array(env, &crate::history_diff_spans(&before, &after, false))
        },
        _ => std::ptr::null_mut(),
    }
}

/// `CaperMobile.composerToggle(text, start, end, style)`:
/// `crate::composer_toggle`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_composerToggle(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
    start: jint,
    end: jint,
    style: jint,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::composer_toggle(&text, start, end, style)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.selectionStyles(text, start, end)`: `crate::selection_styles`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_selectionStyles(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
    start: jint,
    end: jint,
) -> jint {
    // SAFETY: forwarded from the JVM's call.
    unsafe { string(env, text) }.map_or(0, |text| crate::selection_styles(&text, start, end))
}

/// `CaperMobile.composerEdit(previous, next, start, end, continuing)`:
/// `crate::composer_edit`.
///
/// # Safety
/// Called by the JVM with its environment and two live strings.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_composerEdit(
    env: *mut JNIEnv,
    _class: jclass,
    previous: jstring,
    next: jstring,
    start: jint,
    end: jint,
    continuing: jint,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { (string(env, previous), string(env, next)) } {
        // SAFETY: forwarded from the JVM's call.
        (Some(previous), Some(next)) => unsafe {
            int_array(
                env,
                &crate::composer_edit(&previous, &next, start, end, continuing),
            )
        },
        _ => std::ptr::null_mut(),
    }
}

/// `CaperMobile.emojiCatalog(json)`: `crate::emoji_catalog`, from the bytes
/// of the catalog asset the app ships.
///
/// # Safety
/// Called by the JVM with its environment and a live byte array.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_emojiCatalog(
    env: *mut JNIEnv,
    _class: jclass,
    json: jbyteArray,
) -> jboolean {
    // SAFETY: a JNI environment always carries its function table.
    let functions = unsafe { &**env };
    let (Some(length), Some(region)) = (functions.GetArrayLength, functions.GetByteArrayRegion)
    else {
        return JNI_FALSE;
    };
    // SAFETY: `json` is a live array reference for this native call.
    let len = unsafe { length(env, json) };
    let mut bytes = vec![0u8; usize::try_from(len).unwrap_or_default()];
    // SAFETY: `bytes` holds exactly `len` bytes.
    unsafe { region(env, json, 0, len, bytes.as_mut_ptr().cast::<jbyte>()) };
    if crate::emoji_catalog(&bytes) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// `CaperMobile.emojiToken(text, caret)`: `crate::emoji_token`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_emojiToken(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
    caret: jint,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::emoji_token(&text, caret)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.emojiSuggestions(query)`: `crate::emoji_suggestions`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_emojiSuggestions(
    env: *mut JNIEnv,
    _class: jclass,
    query: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, query) } {
        // SAFETY: forwarded from the JVM's call.
        Some(query) => unsafe { int_array(env, &crate::emoji_suggestions(&query)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.nameHasHidden(text)`: `crate::names::has_hidden(text)`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_nameHasHidden(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jboolean {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        Some(text) if crate::names::has_hidden(&text) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

/// `CaperMobile.channelName(text)`: `crate::channel_name`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_channelName(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::channel_name(&text)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.nameError(text, channel)`: `crate::name_error`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_nameError(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
    channel: jboolean,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::name_error(&text, channel != JNI_FALSE)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.spaceName(text)`: `crate::space_name_span(text, false)`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_spaceName(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::space_name_span(&text, false)) },
        None => std::ptr::null_mut(),
    }
}

/// `CaperMobile.inviteCode(text)`: `crate::invite_code`.
///
/// # Safety
/// Called by the JVM with its environment and a live string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_chat_caper_android_data_CaperMobile_inviteCode(
    env: *mut JNIEnv,
    _class: jclass,
    text: jstring,
) -> jintArray {
    // SAFETY: forwarded from the JVM's call.
    match unsafe { string(env, text) } {
        // SAFETY: forwarded from the JVM's call.
        Some(text) => unsafe { int_array(env, &crate::invite_code(&text)) },
        None => std::ptr::null_mut(),
    }
}
