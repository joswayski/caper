//! Web's `inviteCode` (apps/web/src/spaces/client.ts): the invite code in what
//! someone pasted or opened. It accepts a full or scheme-less
//! `<host>/invite/<code>` link with an optional trailing slash, query or
//! fragment, `caper://invite/<code>`, or the bare code. A code is a generated
//! one (ten letters and digits) or a space's vanity name (3 to 32 letters,
//! digits and inner hyphens). It returns only the code: the link's host and
//! scheme are never used, so clients
//! always join through their configured API origin. Shared by the desktop
//! clients (`apps/native/client`) and Android and Apple (`apps/native/mobile`).
//! Every client runs `shared/invites/invite-cases.json`, generated from web;
//! change them together.

/// Web's `[A-Za-z0-9][A-Za-z0-9-]{1,30}[A-Za-z0-9]`.
fn code_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Whether `code` could name an invite: a generated code or a vanity name.
pub fn valid_code(code: &str) -> bool {
    (3..=32).contains(&code.len())
        && code.bytes().all(code_byte)
        && !code.starts_with('-')
        && !code.ends_with('-')
}

/// White space as JavaScript's `trim` and `\s` see it.
fn space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// The line terminators JavaScript's `.` does not match.
fn line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `code` plus at most one `/`, then the end, or a query or fragment that
/// (like web's `[?#].*$`) holds no line terminator.
fn code_then(rest: &str, suffix: bool) -> Option<&str> {
    let end = rest
        .bytes()
        .position(|byte| !code_byte(byte))
        .unwrap_or(rest.len());
    let code = Some(&rest[..end]).filter(|code| valid_code(code))?;
    let rest = &rest[end..];
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let ends = rest.is_empty()
        || (suffix && rest.starts_with(['?', '#']) && !rest.chars().any(line_terminator));
    ends.then_some(code)
}

/// The leading `scheme://` web strips (`^[a-z][a-z0-9+.-]*:\/\/`, any case).
fn without_scheme(value: &str) -> &str {
    let mut chars = value.char_indices();
    if !chars.next().is_some_and(|(_, c)| c.is_ascii_alphabetic()) {
        return value;
    }
    let end = chars
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
        .map_or(value.len(), |(index, _)| index);
    value[end..].strip_prefix("://").unwrap_or(value)
}

/// The code in a pasted or opened invite, or `None`.
pub fn invite_code(input: &str) -> Option<&str> {
    let value = input.trim_matches(space);
    if valid_code(value) {
        return Some(value);
    }
    if value
        .get(..15)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("caper://invite/"))
        && let Some(code) = code_then(&value[15..], true)
    {
        return Some(code);
    }
    let path = without_scheme(value);
    // Web's `[?#].*$`: from the first `?` or `#` after the last line terminator.
    let after = path
        .char_indices()
        .rfind(|(_, c)| line_terminator(*c))
        .map_or(0, |(index, c)| index + c.len_utf8());
    let path = path[after..]
        .find(['?', '#'])
        .map_or(path, |index| &path[..after + index]);
    let slash = path.find('/')?;
    if path[..slash].chars().any(space) {
        return None;
    }
    code_then(path[slash..].strip_prefix("/invite/")?, false)
}
