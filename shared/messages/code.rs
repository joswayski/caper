//! Fenced code block parsing, automatic language detection, and lightweight
//! syntax highlighting shared across all native clients (and mirrored in
//! `apps/web/src/chat/code.ts`, verified by `shared/messages/code-cases.json`).
//!
//! The includer provides the formatting grammar as `super::parser`.

use super::parser::{fenced_block_end, single_line_fence};
use std::ops::Range;

/// Semantic syntax token kinds (passed as `i32` across the mobile C ABI / JNI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum TokenKind {
    Plain = 0,
    Keyword = 1,
    String = 2,
    Comment = 3,
    Number = 4,
    Type = 5,
    Function = 6,
    Attribute = 7,
    Addition = 8,
    Deletion = 9,
}

impl TokenKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::Comment => "comment",
            Self::Number => "number",
            Self::Type => "type",
            Self::Function => "function",
            Self::Attribute => "attribute",
            Self::Addition => "addition",
            Self::Deletion => "deletion",
        }
    }
}

/// Supported programming and markup languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Language {
    Plain = 0,
    Rust = 1,
    TypeScript = 2,
    JavaScript = 3,
    Python = 4,
    Swift = 5,
    Kotlin = 6,
    Go = 7,
    C = 8,
    Cpp = 9,
    Java = 10,
    CSharp = 11,
    Sql = 12,
    Json = 13,
    Html = 14,
    Css = 15,
    Bash = 16,
    Yaml = 17,
    Toml = 18,
    Ruby = 19,
    Php = 20,
    Diff = 21,
}

impl Language {
    pub fn id(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Go => "go",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Sql => "sql",
            Self::Json => "json",
            Self::Html => "html",
            Self::Css => "css",
            Self::Bash => "bash",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Diff => "diff",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Plain => "Plain text",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Swift => "Swift",
            Self::Kotlin => "Kotlin",
            Self::Go => "Go",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Java => "Java",
            Self::CSharp => "C#",
            Self::Sql => "SQL",
            Self::Json => "JSON",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Bash => "Shell",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Ruby => "Ruby",
            Self::Php => "PHP",
            Self::Diff => "Diff",
        }
    }

    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Rust,
            2 => Self::TypeScript,
            3 => Self::JavaScript,
            4 => Self::Python,
            5 => Self::Swift,
            6 => Self::Kotlin,
            7 => Self::Go,
            8 => Self::C,
            9 => Self::Cpp,
            10 => Self::Java,
            11 => Self::CSharp,
            12 => Self::Sql,
            13 => Self::Json,
            14 => Self::Html,
            15 => Self::Css,
            16 => Self::Bash,
            17 => Self::Yaml,
            18 => Self::Toml,
            19 => Self::Ruby,
            20 => Self::Php,
            21 => Self::Diff,
            _ => Self::Plain,
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "rs" | "rust" => Some(Self::Rust),
            "ts" | "tsx" | "typescript" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" | "javascript" => Some(Self::JavaScript),
            "py" | "python" | "py3" => Some(Self::Python),
            "swift" => Some(Self::Swift),
            "kt" | "kts" | "kotlin" => Some(Self::Kotlin),
            "go" | "golang" => Some(Self::Go),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "hpp" | "c++" => Some(Self::Cpp),
            "java" => Some(Self::Java),
            "cs" | "csharp" | "c#" => Some(Self::CSharp),
            "sql" | "postgres" | "postgresql" | "mysql" | "sqlite" => Some(Self::Sql),
            "json" | "jsonc" => Some(Self::Json),
            "html" | "htm" | "xml" | "svg" => Some(Self::Html),
            "css" | "scss" | "less" => Some(Self::Css),
            "sh" | "bash" | "zsh" | "shell" | "console" => Some(Self::Bash),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "rb" | "ruby" => Some(Self::Ruby),
            "php" => Some(Self::Php),
            "diff" | "patch" => Some(Self::Diff),
            "text" | "txt" | "plain" | "plaintext" => Some(Self::Plain),
            _ => None,
        }
    }
}

/// A parsed fenced code block inside a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlock<'a> {
    /// Explicit language tag after opening ```` ``` ````, if any.
    pub declared: Option<&'a str>,
    /// Resolved language (from explicit tag or auto-detection).
    pub language: Language,
    /// Whether `language` came from automatic detection rather than a tag.
    pub detected: bool,
    /// The inner code body without the surrounding fences.
    pub code: &'a str,
    /// Byte range of `code` in the original message.
    pub code_range: Range<usize>,
    /// Byte range of the full fenced block (including fences) in the original message.
    pub full_range: Range<usize>,
}

/// Top-level message block: normal formatted prose or a fenced code block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBlock<'a> {
    Prose { text: &'a str, range: Range<usize> },
    Code(CodeBlock<'a>),
}

/// Splits `text` into alternating prose and fenced code block segments using
/// the same fence rules as `shared/messages/formatting.rs`.
pub fn parse_blocks(text: &str) -> Vec<MessageBlock<'_>> {
    let chars: Vec<char> = text.chars().collect();
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let n = chars.len();
    let mut lines = Vec::new();
    let mut start = 0;
    loop {
        let end = (start..n).find(|i| chars[*i] == '\n').unwrap_or(n);
        lines.push((start, end));
        if end == n {
            break;
        }
        start = end + 1;
    }

    let mut out = Vec::new();
    let mut prose_start: Option<usize> = None;
    let mut prose_end = 0usize;
    let mut idx = 0;
    while idx < lines.len() {
        if let Some(close) = fenced_block_end(&chars, &lines, idx) {
            if let Some(ps) = prose_start.take()
                && ps < prose_end
            {
                let range = offsets[ps]..offsets[prose_end];
                out.push(MessageBlock::Prose {
                    text: &text[range.clone()],
                    range,
                });
            }
            let (open_s, open_e) = lines[idx];
            let (_, close_e) = lines[close];
            let full_range = offsets[open_s]..offsets[close_e];
            let (declared, code_char_start, code_char_end) = if close == idx {
                let (a, b) = single_line_fence(&chars, open_s, open_e).unwrap();
                (None, a, b)
            } else {
                let tag_slice = text[offsets[open_s + 3]..offsets[open_e]].trim();
                let declared = (!tag_slice.is_empty()).then_some(tag_slice);
                let body_start = open_e + 1; // right after opening line's '\n'
                let body_end = if close == idx + 1 {
                    body_start
                } else {
                    lines[close - 1].1
                };
                (declared, body_start, body_end)
            };
            let code_range = offsets[code_char_start]..offsets[code_char_end];
            let code = &text[code_range.clone()];
            let (language, detected) = match declared {
                Some(tag) => match Language::from_tag(tag) {
                    Some(lang) => (lang, false),
                    None => (detect_language(code), true),
                },
                None => (detect_language(code), true),
            };
            out.push(MessageBlock::Code(CodeBlock {
                declared,
                language,
                detected: detected && language != Language::Plain,
                code,
                code_range,
                full_range,
            }));
            idx = close + 1;
        } else {
            let (s, e) = lines[idx];
            if prose_start.is_none() {
                prose_start = Some(s);
            }
            prose_end = e;
            idx += 1;
        }
    }
    if let Some(ps) = prose_start
        && ps < prose_end
    {
        let range = offsets[ps]..offsets[prose_end];
        out.push(MessageBlock::Prose {
            text: &text[range.clone()],
            range,
        });
    }
    out
}

/// Automatically detects the most likely language of an untagged code snippet.
/// Returns `Language::Plain` when confidence is below threshold.
pub fn detect_language(code: &str) -> Language {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Language::Plain;
    }

    // 1. Shebang detection.
    if let Some(first_line) = trimmed.lines().next()
        && first_line.starts_with("#!")
    {
        let lower = first_line.to_ascii_lowercase();
        if lower.contains("python") {
            return Language::Python;
        }
        if lower.contains("node") || lower.contains("deno") || lower.contains("bun") {
            return Language::JavaScript;
        }
        if lower.contains("ruby") {
            return Language::Ruby;
        }
        if lower.contains("php") {
            return Language::Php;
        }
        if lower.contains("sh") || lower.contains("bash") || lower.contains("zsh") {
            return Language::Bash;
        }
    }

    // 2. PHP opening tag.
    if trimmed.starts_with("<?php") || trimmed.contains("<?php") {
        return Language::Php;
    }

    // 3. Diff detection.
    let diff_lines = trimmed.lines().filter(|l| {
        l.starts_with("diff --git ")
            || l.starts_with("@@ ")
            || l.starts_with("--- ")
            || l.starts_with("+++ ")
    });
    if diff_lines.count() >= 2 {
        return Language::Diff;
    }

    // 4. JSON detection: strict structural shape.
    if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && trimmed.contains("\":")
        && !trimmed.contains("=>")
        && !trimmed.contains("function ")
        && !trimmed.contains("fn ")
        && !trimmed.contains("def ")
        && !trimmed.contains("const ")
        && !trimmed.contains("let ")
        && !trimmed.contains("var ")
    {
        return Language::Json;
    }

    // 5. HTML / XML detection.
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<!doctype html")
        || lower.contains("<html")
        || lower.contains("</div>")
        || lower.contains("</span>")
        || lower.contains("</section>")
        || lower.contains("</button>")
        || lower.contains("</svg>")
        || (trimmed.starts_with('<') && trimmed.ends_with('>') && trimmed.contains("</"))
    {
        // Exclude TSX/JSX that has TS/JS module syntax.
        if !trimmed.contains("export ")
            && !trimmed.contains("import ")
            && !trimmed.contains("const ")
            && !trimmed.contains("function ")
        {
            return Language::Html;
        }
    }

    // 6. Weighted feature scoring across languages.
    let mut scores: [(Language, i32); 17] = [
        (Language::Rust, 0),
        (Language::TypeScript, 0),
        (Language::JavaScript, 0),
        (Language::Python, 0),
        (Language::Swift, 0),
        (Language::Kotlin, 0),
        (Language::Go, 0),
        (Language::Cpp, 0),
        (Language::C, 0),
        (Language::Java, 0),
        (Language::CSharp, 0),
        (Language::Sql, 0),
        (Language::Css, 0),
        (Language::Bash, 0),
        (Language::Yaml, 0),
        (Language::Toml, 0),
        (Language::Ruby, 0),
    ];

    let mut add = |lang: Language, pts: i32| {
        if let Some(entry) = scores.iter_mut().find(|(l, _)| *l == lang) {
            entry.1 += pts;
        }
    };

    // Rust
    for (pat, pts) in [
        ("fn ", 3),
        ("let mut ", 4),
        ("pub fn ", 4),
        ("pub struct ", 4),
        ("impl ", 3),
        ("match ", 2),
        ("::", 2),
        ("-> ", 1),
        ("&str", 4),
        ("&mut ", 4),
        ("println!(", 4),
        ("format!(", 3),
        ("vec![", 4),
        ("#[derive(", 5),
        ("use crate::", 5),
        ("use std::", 5),
        ("Result<", 3),
        ("Option<", 3),
        ("Some(", 2),
        ("Ok(", 2),
        ("Err(", 2),
    ] {
        if trimmed.contains(pat) {
            add(Language::Rust, pts);
        }
    }

    // TypeScript
    for (pat, pts) in [
        ("interface ", 4),
        (": string", 4),
        (": number", 4),
        (": boolean", 4),
        (": void", 3),
        ("Promise<", 4),
        ("export type ", 5),
        ("export interface ", 5),
        ("as const", 4),
        ("readonly ", 3),
        ("Record<", 4),
        ("type ", 2),
        ("async function ", 3),
        ("const ", 2),
        ("=> ", 2),
        ("import ", 1),
        ("export ", 1),
    ] {
        if trimmed.contains(pat) {
            add(Language::TypeScript, pts);
        }
    }

    // JavaScript
    for (pat, pts) in [
        ("const ", 2),
        ("let ", 1),
        ("function ", 3),
        ("async function ", 3),
        ("=> ", 2),
        ("console.log(", 4),
        ("document.", 3),
        ("window.", 3),
        ("module.exports", 5),
        ("require(", 3),
        ("export default ", 3),
        ("===", 2),
        ("!==", 2),
        ("fetch(", 2),
        ("await ", 1),
    ] {
        if trimmed.contains(pat) {
            add(Language::JavaScript, pts);
        }
    }

    // Python
    for (pat, pts) in [
        ("def ", 4),
        ("elif ", 5),
        ("self.", 4),
        ("__init__", 5),
        ("__name__", 5),
        ("print(", 3),
        ("lambda ", 3),
        ("None", 2),
        ("True", 1),
        ("False", 1),
        ("async def ", 5),
        ("from ", 1),
        ("import ", 1),
        ("f\"", 3),
        ("f'", 3),
        ("raise ", 2),
        ("except ", 4),
    ] {
        if trimmed.contains(pat) {
            add(Language::Python, pts);
        }
    }
    if trimmed.lines().any(|l| {
        let t = l.trim_end();
        (t.starts_with("def ")
            || t.starts_with("class ")
            || t.starts_with("if ")
            || t.starts_with("for ")
            || t.starts_with("while "))
            && t.ends_with(':')
    }) {
        add(Language::Python, 4);
    }

    // Swift
    for (pat, pts) in [
        ("func ", 3),
        ("guard let ", 5),
        ("guard ", 3),
        ("if let ", 4),
        ("import SwiftUI", 6),
        ("import Foundation", 5),
        ("import AppKit", 6),
        ("import UIKit", 6),
        ("@MainActor", 5),
        ("@State ", 5),
        ("@Published ", 5),
        ("protocol ", 3),
        ("struct ", 2),
        ("var ", 1),
        ("let ", 1),
        ("NSView", 4),
        ("UIView", 4),
        ("-> ", 1),
    ] {
        if trimmed.contains(pat) {
            add(Language::Swift, pts);
        }
    }

    // Kotlin
    for (pat, pts) in [
        ("fun ", 4),
        ("val ", 3),
        ("data class ", 5),
        ("suspend fun ", 5),
        ("companion object", 5),
        ("@Composable", 6),
        ("when (", 4),
        ("println(", 3),
        ("lateinit var ", 5),
        ("object ", 2),
    ] {
        if trimmed.contains(pat) {
            add(Language::Kotlin, pts);
        }
    }

    // Go
    for (pat, pts) in [
        ("package main", 6),
        ("func (", 4),
        ("func main()", 5),
        (":=", 4),
        ("fmt.", 4),
        ("if err != nil", 6),
        ("defer ", 3),
        ("go func", 4),
        ("chan ", 3),
        ("interface{}", 4),
    ] {
        if trimmed.contains(pat) {
            add(Language::Go, pts);
        }
    }

    // C++ & C
    for (pat, pts) in [
        ("#include <", 5),
        ("#include \"", 4),
        ("#define ", 3),
        ("std::", 5),
        ("nullptr", 5),
        ("cout <<", 5),
        ("template<", 5),
        ("template <", 5),
        ("namespace ", 2),
    ] {
        if trimmed.contains(pat) {
            add(Language::Cpp, pts);
        }
    }
    for (pat, pts) in [
        ("#include <stdio.h>", 6),
        ("#include <stdlib.h>", 6),
        ("printf(", 4),
        ("malloc(", 4),
        ("sizeof(", 3),
        ("int main(", 4),
    ] {
        if trimmed.contains(pat) {
            add(Language::C, pts);
        }
    }

    // Java
    for (pat, pts) in [
        ("public class ", 5),
        ("public static void main", 6),
        ("System.out.println", 6),
        ("private final ", 4),
        ("@Override", 4),
        ("implements ", 3),
        ("extends ", 2),
        ("ArrayList<", 4),
    ] {
        if trimmed.contains(pat) {
            add(Language::Java, pts);
        }
    }

    // C#
    for (pat, pts) in [
        ("using System", 6),
        ("Console.WriteLine", 6),
        ("public async Task", 5),
        ("get; set;", 5),
        ("namespace ", 3),
        ("string[]", 3),
    ] {
        if trimmed.contains(pat) {
            add(Language::CSharp, pts);
        }
    }

    // SQL (case-insensitive)
    let upper = trimmed.to_ascii_uppercase();
    for (pat, pts) in [
        ("SELECT ", 3),
        ("FROM ", 2),
        ("WHERE ", 2),
        ("INSERT INTO ", 5),
        ("CREATE TABLE ", 5),
        ("ALTER TABLE ", 5),
        ("UPDATE ", 2),
        ("LEFT JOIN ", 4),
        ("INNER JOIN ", 4),
        ("GROUP BY ", 4),
        ("ORDER BY ", 4),
        ("VALUES (", 3),
        ("PRIMARY KEY", 4),
    ] {
        if upper.contains(pat) {
            add(Language::Sql, pts);
        }
    }

    // CSS
    if trimmed.contains('{') && trimmed.contains('}') && trimmed.contains(':') {
        for (pat, pts) in [
            ("display:", 3),
            ("color:", 2),
            ("background:", 3),
            ("background-color:", 3),
            ("padding:", 3),
            ("margin:", 3),
            ("border-radius:", 4),
            ("font-size:", 3),
            ("@media ", 4),
            ("var(--", 4),
            ("px;", 2),
            ("rem;", 2),
            (":hover", 3),
        ] {
            if trimmed.contains(pat) {
                add(Language::Css, pts);
            }
        }
    }

    // Bash / Shell
    for (pat, pts) in [
        ("export ", 2),
        ("echo ", 3),
        ("curl ", 3),
        ("sudo ", 3),
        ("grep ", 3),
        ("chmod ", 4),
        ("mkdir ", 3),
        ("rm -rf ", 4),
        ("git ", 2),
        ("cargo ", 2),
        ("npm ", 2),
        ("if [ ", 4),
        ("if [[ ", 5),
        ("\nfi", 4),
        ("\ndone", 4),
        ("$(", 2),
        (" | ", 2),
        (" && ", 2),
    ] {
        if trimmed.contains(pat) {
            add(Language::Bash, pts);
        }
    }

    // TOML
    if trimmed
        .lines()
        .any(|l| l.starts_with('[') && l.ends_with(']') && !l.contains(' '))
        && trimmed.lines().any(|l| l.contains(" = "))
        && !trimmed.contains('{')
        && !trimmed.contains(';')
    {
        add(Language::Toml, 6);
    }

    // YAML
    let yaml_kv = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('#')
                && !t.ends_with('{')
                && !t.ends_with(';')
                && t.find(": ").is_some_and(|i| {
                    i > 0
                        && t[..i]
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                })
        })
        .count();
    if yaml_kv >= 2 && !trimmed.contains('{') && !trimmed.contains(';') {
        add(Language::Yaml, 4 + yaml_kv.min(4) as i32);
    }

    // Ruby
    for (pat, pts) in [
        ("puts ", 4),
        ("elsif ", 5),
        ("attr_accessor", 5),
        (".each do", 5),
        (" do |", 4),
        ("require '", 4),
        ("\nend", 2),
        ("def ", 2),
    ] {
        if trimmed.contains(pat) {
            add(Language::Ruby, pts);
        }
    }

    let (best_lang, best_score) = scores
        .into_iter()
        .max_by_key(|&(_, score)| score)
        .unwrap_or((Language::Plain, 0));

    if best_score >= 4 {
        best_lang
    } else {
        Language::Plain
    }
}

/// One highlighted token span (UTF-8 byte offsets into the code string).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Tokenizes `code` in `lang` into contiguous, non-overlapping semantic spans
/// covering `0..code.len()`.
pub fn highlight(code: &str, lang: Language) -> Vec<TokenSpan> {
    if code.is_empty() {
        return Vec::new();
    }
    if lang == Language::Plain {
        return vec![TokenSpan {
            start: 0,
            end: code.len(),
            kind: TokenKind::Plain,
        }];
    }
    if lang == Language::Diff {
        return highlight_diff(code);
    }
    if lang == Language::Html {
        return highlight_html(code);
    }
    if lang == Language::Json {
        return highlight_json(code);
    }
    if lang == Language::Yaml {
        return highlight_yaml(code);
    }
    if lang == Language::Toml {
        return highlight_toml(code);
    }
    if lang == Language::Css {
        return highlight_css(code);
    }
    highlight_code(code, lang)
}

fn push_span(out: &mut Vec<TokenSpan>, start: usize, end: usize, kind: TokenKind) {
    if start >= end {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.kind == kind
        && last.end == start
    {
        last.end = end;
        return;
    }
    out.push(TokenSpan { start, end, kind });
}

fn highlight_diff(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let mut offset = 0;
    for chunk in code.split_inclusive('\n') {
        let kind =
            if chunk.starts_with("@@") || chunk.starts_with("diff ") || chunk.starts_with("index ")
            {
                TokenKind::Attribute
            } else if chunk.starts_with("+++") || chunk.starts_with("---") {
                TokenKind::Keyword
            } else if chunk.starts_with('+') {
                TokenKind::Addition
            } else if chunk.starts_with('-') {
                TokenKind::Deletion
            } else {
                TokenKind::Plain
            };
        push_span(&mut out, offset, offset + chunk.len(), kind);
        offset += chunk.len();
    }
    out
}

fn highlight_html(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if code[i..].starts_with("<!--") {
            let end = code[i + 4..].find("-->").map_or(n, |pos| i + 4 + pos + 3);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if bytes[i] == b'<' {
            let start = i;
            i += 1;
            if i < n && (bytes[i] == b'/' || bytes[i] == b'!' || bytes[i] == b'?') {
                i += 1;
            }
            let name_start = i;
            while i < n
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_' | b':'))
            {
                i += 1;
            }
            push_span(&mut out, start, name_start, TokenKind::Plain);
            push_span(&mut out, name_start, i, TokenKind::Keyword);
            while i < n && bytes[i] != b'>' {
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let q = bytes[i];
                    let s = i;
                    i += 1;
                    while i < n && bytes[i] != q {
                        i += 1;
                    }
                    if i < n {
                        i += 1;
                    }
                    push_span(&mut out, s, i, TokenKind::String);
                } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                    let s = i;
                    while i < n
                        && (bytes[i].is_ascii_alphanumeric()
                            || matches!(bytes[i], b'-' | b'_' | b':'))
                    {
                        i += 1;
                    }
                    push_span(&mut out, s, i, TokenKind::Attribute);
                } else {
                    let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
                    push_span(&mut out, i, i + ch_len, TokenKind::Plain);
                    i += ch_len;
                }
            }
            if i < n && bytes[i] == b'>' {
                push_span(&mut out, i, i + 1, TokenKind::Plain);
                i += 1;
            }
        } else {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            push_span(&mut out, i, i + ch_len, TokenKind::Plain);
            i += ch_len;
        }
    }
    out
}

fn highlight_json(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let end = code[i..].find('\n').map_or(n, |p| i + p);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if bytes[i] == b'"' {
            let start = i;
            i = scan_quoted(bytes, i, b'"');
            // Check whether followed by ':' -> object key.
            let mut look = i;
            while look < n && bytes[look].is_ascii_whitespace() {
                look += 1;
            }
            let kind = if look < n && bytes[look] == b':' {
                TokenKind::Attribute
            } else {
                TokenKind::String
            };
            push_span(&mut out, start, i, kind);
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'-' && i + 1 < n && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i = scan_number(bytes, i);
            push_span(&mut out, start, i, TokenKind::Number);
        } else if bytes[i].is_ascii_alphabetic() {
            let start = i;
            while i < n && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let word = &code[start..i];
            let kind = if matches!(word, "true" | "false" | "null") {
                TokenKind::Keyword
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, start, i, kind);
        } else {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            push_span(&mut out, i, i + ch_len, TokenKind::Plain);
            i += ch_len;
        }
    }
    out
}

fn highlight_yaml(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let mut offset = 0;
    for chunk in code.split_inclusive('\n') {
        let trimmed = chunk.trim_start();
        let lead = chunk.len() - trimmed.len();
        if trimmed.starts_with('#') {
            push_span(&mut out, offset, offset + lead, TokenKind::Plain);
            push_span(
                &mut out,
                offset + lead,
                offset + chunk.len(),
                TokenKind::Comment,
            );
        } else if let Some(colon) = trimmed.find(": ")
            && colon > 0
            && !trimmed.starts_with('-')
        {
            push_span(&mut out, offset, offset + lead, TokenKind::Plain);
            push_span(
                &mut out,
                offset + lead,
                offset + lead + colon,
                TokenKind::Attribute,
            );
            let rest_start = offset + lead + colon;
            highlight_yaml_value(&mut out, code, rest_start, offset + chunk.len());
        } else {
            highlight_yaml_value(&mut out, code, offset, offset + chunk.len());
        }
        offset += chunk.len();
    }
    out
}

fn highlight_yaml_value(out: &mut Vec<TokenSpan>, code: &str, start: usize, end: usize) {
    let bytes = code.as_bytes();
    let mut i = start;
    while i < end {
        if bytes[i] == b'#' && (i == start || bytes[i - 1].is_ascii_whitespace()) {
            push_span(out, i, end, TokenKind::Comment);
            return;
        } else if bytes[i] == b'"' || bytes[i] == b'\'' {
            let s = i;
            i = scan_quoted(bytes, i, bytes[i]).min(end);
            push_span(out, s, i, TokenKind::String);
        } else if bytes[i].is_ascii_digit() && (i == start || !bytes[i - 1].is_ascii_alphanumeric())
        {
            let s = i;
            i = scan_number(bytes, i).min(end);
            push_span(out, s, i, TokenKind::Number);
        } else if bytes[i].is_ascii_alphabetic() {
            let s = i;
            while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &code[s..i];
            let kind = if matches!(word, "true" | "false" | "null" | "yes" | "no") {
                TokenKind::Keyword
            } else {
                TokenKind::Plain
            };
            push_span(out, s, i, kind);
        } else {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            push_span(out, i, (i + ch_len).min(end), TokenKind::Plain);
            i += ch_len;
        }
    }
}

fn highlight_toml(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let mut offset = 0;
    for chunk in code.split_inclusive('\n') {
        let trimmed = chunk.trim_start();
        let lead = chunk.len() - trimmed.len();
        if trimmed.starts_with('#') {
            push_span(&mut out, offset, offset + lead, TokenKind::Plain);
            push_span(
                &mut out,
                offset + lead,
                offset + chunk.len(),
                TokenKind::Comment,
            );
        } else if trimmed.starts_with('[') {
            push_span(&mut out, offset, offset + lead, TokenKind::Plain);
            push_span(
                &mut out,
                offset + lead,
                offset + chunk.len(),
                TokenKind::Keyword,
            );
        } else if let Some(eq) = trimmed.find('=') {
            push_span(&mut out, offset, offset + lead, TokenKind::Plain);
            push_span(
                &mut out,
                offset + lead,
                offset + lead + eq,
                TokenKind::Attribute,
            );
            highlight_yaml_value(&mut out, code, offset + lead + eq, offset + chunk.len());
        } else {
            highlight_yaml_value(&mut out, code, offset, offset + chunk.len());
        }
        offset += chunk.len();
    }
    out
}

fn highlight_css(code: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut depth = 0usize;
    while i < n {
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let end = code[i + 2..].find("*/").map_or(n, |p| i + 2 + p + 2);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if bytes[i] == b'"' || bytes[i] == b'\'' {
            let s = i;
            i = scan_quoted(bytes, i, bytes[i]);
            push_span(&mut out, s, i, TokenKind::String);
        } else if bytes[i] == b'@' {
            let s = i;
            i += 1;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
                i += 1;
            }
            push_span(&mut out, s, i, TokenKind::Keyword);
        } else if bytes[i] == b'{' {
            depth += 1;
            push_span(&mut out, i, i + 1, TokenKind::Plain);
            i += 1;
        } else if bytes[i] == b'}' {
            depth = depth.saturating_sub(1);
            push_span(&mut out, i, i + 1, TokenKind::Plain);
            i += 1;
        } else if bytes[i] == b'#' && depth > 0 {
            let s = i;
            i += 1;
            while i < n && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            push_span(&mut out, s, i, TokenKind::Number);
        } else if bytes[i].is_ascii_digit() {
            let s = i;
            i = scan_number(bytes, i);
            while i < n && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'%') {
                i += 1;
            }
            push_span(&mut out, s, i, TokenKind::Number);
        } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'-' || bytes[i] == b'.' {
            let s = i;
            while i < n
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_' | b'.'))
            {
                i += 1;
            }
            let mut look = i;
            while look < n && bytes[look] == b' ' {
                look += 1;
            }
            let kind = if depth > 0 && look < n && bytes[look] == b':' {
                TokenKind::Attribute
            } else if look < n && bytes[look] == b'(' {
                TokenKind::Function
            } else if depth == 0 {
                TokenKind::Type
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, s, i, kind);
        } else {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            push_span(&mut out, i, i + ch_len, TokenKind::Plain);
            i += ch_len;
        }
    }
    out
}

fn highlight_code(code: &str, lang: Language) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let n = bytes.len();
    let hash_comment = matches!(
        lang,
        Language::Python | Language::Bash | Language::Ruby | Language::Php
    );
    let dash_comment = lang == Language::Sql;
    let slash_comment = !matches!(lang, Language::Python | Language::Bash | Language::Ruby);
    let backtick_string = matches!(
        lang,
        Language::JavaScript | Language::TypeScript | Language::Go | Language::Bash
    );
    let single_quote_string = lang != Language::Rust;
    let mut i = 0;

    while i < n {
        // Line comments
        if slash_comment && bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let end = code[i..].find('\n').map_or(n, |p| i + p);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if slash_comment && bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let end = code[i + 2..].find("*/").map_or(n, |p| i + 2 + p + 2);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if dash_comment && bytes[i] == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            let end = code[i..].find('\n').map_or(n, |p| i + p);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        } else if hash_comment && bytes[i] == b'#' {
            let end = code[i..].find('\n').map_or(n, |p| i + p);
            push_span(&mut out, i, end, TokenKind::Comment);
            i = end;
        }
        // Rust attributes `#[...]` / `#![...]` and C/C++ preprocessor `#include`
        else if bytes[i] == b'#' && matches!(lang, Language::Rust | Language::C | Language::Cpp) {
            if lang == Language::Rust
                && ((i + 1 < n && bytes[i + 1] == b'[')
                    || (i + 2 < n && bytes[i + 1] == b'!' && bytes[i + 2] == b'['))
            {
                let start = i;
                let mut depth = 0usize;
                while i < n {
                    if bytes[i] == b'[' {
                        depth += 1;
                    } else if bytes[i] == b']' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
                push_span(&mut out, start, i, TokenKind::Attribute);
            } else {
                let start = i;
                i += 1;
                while i < n && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                push_span(&mut out, start, i, TokenKind::Attribute);
            }
        }
        // Decorators / attributes `@MainActor`, `@Composable`, `@Override`, `@decorator`
        else if bytes[i] == b'@'
            && matches!(
                lang,
                Language::Swift
                    | Language::Kotlin
                    | Language::Java
                    | Language::Python
                    | Language::TypeScript
            )
            && i + 1 < n
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let start = i;
            i += 1;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            push_span(&mut out, start, i, TokenKind::Attribute);
        }
        // Strings
        else if bytes[i] == b'"'
            || (single_quote_string && bytes[i] == b'\'')
            || (backtick_string && bytes[i] == b'`')
        {
            let q = bytes[i];
            let start = i;
            // Python triple-quoted strings.
            if lang == Language::Python
                && (q == b'"' || q == b'\'')
                && i + 2 < n
                && bytes[i + 1] == q
                && bytes[i + 2] == q
            {
                let delim = if q == b'"' { "\"\"\"" } else { "'''" };
                i = code[i + 3..].find(delim).map_or(n, |p| i + 3 + p + 3);
            } else {
                i = scan_quoted(bytes, i, q);
            }
            push_span(&mut out, start, i, TokenKind::String);
        }
        // Rust char literal `'a'` or lifetime `'a`
        else if lang == Language::Rust && bytes[i] == b'\'' {
            let start = i;
            if i + 2 < n && bytes[i + 1] == b'\\' {
                let end = (i + 3..n)
                    .find(|&k| bytes[k] == b'\'')
                    .map_or(i + 1, |k| k + 1);
                push_span(&mut out, start, end, TokenKind::String);
                i = end;
            } else if i + 2 < n && bytes[i + 2] == b'\'' {
                push_span(&mut out, start, i + 3, TokenKind::String);
                i += 3;
            } else if i + 1 < n && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') {
                i += 1;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                push_span(&mut out, start, i, TokenKind::Attribute);
            } else {
                push_span(&mut out, i, i + 1, TokenKind::Plain);
                i += 1;
            }
        }
        // Numbers
        else if bytes[i].is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let start = i;
            i = scan_number(bytes, i);
            push_span(&mut out, start, i, TokenKind::Number);
        }
        // Identifiers / keywords / types / functions
        else if bytes[i].is_ascii_alphabetic()
            || bytes[i] == b'_'
            || (bytes[i] == b'$' && matches!(lang, Language::Php | Language::Bash))
        {
            let start = i;
            i += 1;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &code[start..i];
            let kind = if bytes[start] == b'$' {
                TokenKind::Attribute
            } else if is_keyword(lang, word) {
                TokenKind::Keyword
            } else if is_builtin_type(lang, word) {
                TokenKind::Type
            } else if lang == Language::Rust && i < n && bytes[i] == b'!' {
                i += 1;
                TokenKind::Function
            } else if i < n && bytes[i] == b'(' {
                TokenKind::Function
            } else if word.len() > 1
                && word.as_bytes()[0].is_ascii_uppercase()
                && word.as_bytes()[1..].iter().any(u8::is_ascii_lowercase)
            {
                TokenKind::Type
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, start, i, kind);
        } else {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            push_span(&mut out, i, i + ch_len, TokenKind::Plain);
            i += ch_len;
        }
    }
    out
}

fn scan_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let n = bytes.len();
    let mut i = start + 1;
    while i < n {
        if bytes[i] == b'\\' && i + 1 < n {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        if bytes[i] == b'\n' && quote != b'`' {
            return i;
        }
        i += 1;
    }
    n
}

fn scan_number(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut i = start;
    if i < n && bytes[i] == b'-' {
        i += 1;
    }
    if i + 1 < n
        && bytes[i] == b'0'
        && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        i += 2;
        while i < n && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < n && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if i + 1 < n && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < n && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if i < n && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < n && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        while i < n && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    i
}

fn is_keyword(lang: Language, word: &str) -> bool {
    if lang == Language::Sql {
        return matches!(
            word.to_ascii_uppercase().as_str(),
            "SELECT"
                | "FROM"
                | "WHERE"
                | "INSERT"
                | "INTO"
                | "VALUES"
                | "UPDATE"
                | "SET"
                | "DELETE"
                | "CREATE"
                | "TABLE"
                | "ALTER"
                | "DROP"
                | "INDEX"
                | "JOIN"
                | "LEFT"
                | "RIGHT"
                | "INNER"
                | "OUTER"
                | "FULL"
                | "ON"
                | "AS"
                | "AND"
                | "OR"
                | "NOT"
                | "IN"
                | "IS"
                | "NULL"
                | "ORDER"
                | "GROUP"
                | "BY"
                | "HAVING"
                | "LIMIT"
                | "OFFSET"
                | "DISTINCT"
                | "UNION"
                | "ALL"
                | "CASE"
                | "WHEN"
                | "THEN"
                | "ELSE"
                | "END"
                | "PRIMARY"
                | "KEY"
                | "FOREIGN"
                | "REFERENCES"
                | "DEFAULT"
                | "UNIQUE"
                | "TRUE"
                | "FALSE"
                | "RETURNING"
                | "WITH"
                | "ASC"
                | "DESC"
        );
    }
    match lang {
        Language::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        Language::TypeScript | Language::JavaScript => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "keyof"
                | "let"
                | "new"
                | "null"
                | "of"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "satisfies"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "yield"
        ),
        Language::Python => matches!(
            word,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "match"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "self"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        Language::Swift => matches!(
            word,
            "actor"
                | "as"
                | "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "continue"
                | "default"
                | "defer"
                | "do"
                | "else"
                | "enum"
                | "extension"
                | "false"
                | "fileprivate"
                | "final"
                | "for"
                | "func"
                | "guard"
                | "if"
                | "import"
                | "in"
                | "init"
                | "internal"
                | "is"
                | "lazy"
                | "let"
                | "mutating"
                | "nil"
                | "nonisolated"
                | "open"
                | "override"
                | "private"
                | "protocol"
                | "public"
                | "return"
                | "self"
                | "Self"
                | "some"
                | "static"
                | "struct"
                | "super"
                | "switch"
                | "throw"
                | "throws"
                | "true"
                | "try"
                | "typealias"
                | "var"
                | "weak"
                | "where"
                | "while"
        ),
        Language::Kotlin => matches!(
            word,
            "abstract"
                | "as"
                | "break"
                | "by"
                | "catch"
                | "class"
                | "companion"
                | "const"
                | "continue"
                | "data"
                | "do"
                | "else"
                | "enum"
                | "false"
                | "finally"
                | "for"
                | "fun"
                | "if"
                | "import"
                | "in"
                | "init"
                | "inline"
                | "interface"
                | "internal"
                | "is"
                | "lateinit"
                | "null"
                | "object"
                | "open"
                | "override"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "sealed"
                | "super"
                | "suspend"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typealias"
                | "val"
                | "var"
                | "when"
                | "while"
        ),
        Language::Go => matches!(
            word,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "false"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "nil"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "true"
                | "type"
                | "var"
        ),
        Language::C | Language::Cpp => matches!(
            word,
            "auto"
                | "bool"
                | "break"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "constexpr"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "float"
                | "for"
                | "if"
                | "inline"
                | "int"
                | "long"
                | "namespace"
                | "new"
                | "nullptr"
                | "NULL"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "short"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "template"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "typename"
                | "unsigned"
                | "using"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
        ),
        Language::Java | Language::CSharp => matches!(
            word,
            "abstract"
                | "async"
                | "await"
                | "boolean"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "else"
                | "enum"
                | "extends"
                | "false"
                | "final"
                | "finally"
                | "for"
                | "foreach"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "interface"
                | "namespace"
                | "new"
                | "null"
                | "override"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "record"
                | "return"
                | "sealed"
                | "static"
                | "struct"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "throws"
                | "true"
                | "try"
                | "using"
                | "var"
                | "virtual"
                | "void"
                | "while"
        ),
        Language::Bash => matches!(
            word,
            "case"
                | "do"
                | "done"
                | "elif"
                | "else"
                | "esac"
                | "export"
                | "fi"
                | "for"
                | "function"
                | "if"
                | "in"
                | "local"
                | "readonly"
                | "return"
                | "set"
                | "then"
                | "unset"
                | "until"
                | "while"
        ),
        Language::Ruby => matches!(
            word,
            "alias"
                | "and"
                | "begin"
                | "break"
                | "case"
                | "class"
                | "def"
                | "do"
                | "else"
                | "elsif"
                | "end"
                | "ensure"
                | "false"
                | "for"
                | "if"
                | "in"
                | "module"
                | "next"
                | "nil"
                | "not"
                | "or"
                | "raise"
                | "rescue"
                | "return"
                | "self"
                | "super"
                | "then"
                | "true"
                | "unless"
                | "until"
                | "when"
                | "while"
                | "yield"
        ),
        Language::Php => matches!(
            word,
            "abstract"
                | "array"
                | "as"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "echo"
                | "else"
                | "elseif"
                | "extends"
                | "false"
                | "final"
                | "finally"
                | "fn"
                | "for"
                | "foreach"
                | "function"
                | "if"
                | "implements"
                | "interface"
                | "match"
                | "namespace"
                | "new"
                | "null"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "static"
                | "switch"
                | "throw"
                | "trait"
                | "true"
                | "try"
                | "use"
                | "while"
        ),
        _ => false,
    }
}

fn is_builtin_type(lang: Language, word: &str) -> bool {
    match lang {
        Language::Rust => matches!(
            word,
            "bool"
                | "char"
                | "str"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
                | "Arc"
                | "Rc"
        ),
        Language::TypeScript => matches!(
            word,
            "string"
                | "number"
                | "boolean"
                | "bigint"
                | "symbol"
                | "unknown"
                | "never"
                | "any"
                | "object"
                | "Promise"
                | "Record"
                | "Array"
                | "Map"
                | "Set"
        ),
        Language::Go => matches!(
            word,
            "bool"
                | "byte"
                | "error"
                | "float32"
                | "float64"
                | "int"
                | "int8"
                | "int16"
                | "int32"
                | "int64"
                | "rune"
                | "string"
                | "uint"
                | "uint8"
                | "uint16"
                | "uint32"
                | "uint64"
                | "any"
        ),
        Language::Python => matches!(
            word,
            "int" | "float" | "str" | "bool" | "list" | "dict" | "set" | "tuple" | "bytes"
        ),
        Language::Sql => matches!(
            word.to_ascii_uppercase().as_str(),
            "INT"
                | "INTEGER"
                | "BIGINT"
                | "SMALLINT"
                | "TEXT"
                | "VARCHAR"
                | "CHAR"
                | "BOOLEAN"
                | "BOOL"
                | "TIMESTAMP"
                | "TIMESTAMPTZ"
                | "DATE"
                | "UUID"
                | "JSON"
                | "JSONB"
                | "NUMERIC"
                | "FLOAT"
                | "REAL"
                | "SERIAL"
                | "BYTEA"
        ),
        Language::Java | Language::CSharp => matches!(
            word,
            "int"
                | "long"
                | "short"
                | "byte"
                | "float"
                | "double"
                | "char"
                | "bool"
                | "string"
                | "decimal"
                | "object"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_code_contract() {
        #[derive(serde::Deserialize)]
        struct BlockExpect {
            kind: String,
            text: String,
            #[serde(default)]
            language: Option<String>,
            #[serde(default)]
            detected: Option<bool>,
        }
        #[derive(serde::Deserialize)]
        struct BlockCase {
            message: String,
            blocks: Vec<BlockExpect>,
        }
        #[derive(serde::Deserialize)]
        struct DetectCase {
            code: String,
            language: String,
        }
        #[derive(serde::Deserialize)]
        struct HighlightExpect {
            text: String,
            kind: String,
        }
        #[derive(serde::Deserialize)]
        struct HighlightCase {
            language: String,
            code: String,
            must_include: Vec<HighlightExpect>,
        }
        #[derive(serde::Deserialize)]
        struct Cases {
            blocks: Vec<BlockCase>,
            detect: Vec<DetectCase>,
            highlight: Vec<HighlightCase>,
        }
        let cases: Cases = serde_json::from_str(include_str!("code-cases.json")).unwrap();
        for case in cases.blocks {
            let parsed = parse_blocks(&case.message);
            assert_eq!(parsed.len(), case.blocks.len(), "{}", case.message);
            for (actual, expected) in parsed.iter().zip(&case.blocks) {
                match actual {
                    MessageBlock::Prose { text, .. } => {
                        assert_eq!(expected.kind, "prose", "{}", case.message);
                        assert_eq!(*text, expected.text, "{}", case.message);
                    }
                    MessageBlock::Code(block) => {
                        assert_eq!(expected.kind, "code", "{}", case.message);
                        assert_eq!(block.code, expected.text, "{}", case.message);
                        if let Some(lang) = &expected.language {
                            assert_eq!(block.language.id(), lang, "{}", case.message);
                        }
                        if let Some(det) = expected.detected {
                            assert_eq!(block.detected, det, "{}", case.message);
                        }
                    }
                }
            }
        }
        for case in cases.detect {
            assert_eq!(
                detect_language(&case.code).id(),
                case.language,
                "detect failed for:\n{}",
                case.code
            );
        }
        for case in cases.highlight {
            let lang = Language::from_tag(&case.language).unwrap_or(Language::Plain);
            let spans = highlight(&case.code, lang);
            // Verify full, contiguous coverage of code.
            let mut pos = 0;
            for span in &spans {
                assert_eq!(span.start, pos);
                assert!(span.end > span.start);
                pos = span.end;
            }
            assert_eq!(pos, case.code.len());
            for expected in &case.must_include {
                assert!(
                    spans.iter().any(|s| {
                        case.code[s.start..s.end] == expected.text && s.kind.name() == expected.kind
                    }),
                    "expected ({:?}, {:?}) in {:?}",
                    expected.text,
                    expected.kind,
                    spans
                        .iter()
                        .map(|s| (&case.code[s.start..s.end], s.kind.name()))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
