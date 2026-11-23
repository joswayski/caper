/** Fenced code block parsing, automatic language detection, and lightweight
 * syntax highlighting, aligned with `shared/messages/code.rs` and verified by
 * `shared/messages/code-cases.json`.
 */
import { fencedBlockEnd, singleLineFence } from "./formatting.ts";

export type TokenKind =
  | "plain"
  | "keyword"
  | "string"
  | "comment"
  | "number"
  | "type"
  | "function"
  | "attribute"
  | "addition"
  | "deletion";

export type CodeLanguage =
  | "plain"
  | "rust"
  | "typescript"
  | "javascript"
  | "python"
  | "swift"
  | "kotlin"
  | "go"
  | "c"
  | "cpp"
  | "java"
  | "csharp"
  | "sql"
  | "json"
  | "html"
  | "css"
  | "bash"
  | "yaml"
  | "toml"
  | "ruby"
  | "php"
  | "diff";

export const languageDisplayNames: Record<CodeLanguage, string> = {
  plain: "Plain text",
  rust: "Rust",
  typescript: "TypeScript",
  javascript: "JavaScript",
  python: "Python",
  swift: "Swift",
  kotlin: "Kotlin",
  go: "Go",
  c: "C",
  cpp: "C++",
  java: "Java",
  csharp: "C#",
  sql: "SQL",
  json: "JSON",
  html: "HTML",
  css: "CSS",
  bash: "Shell",
  yaml: "YAML",
  toml: "TOML",
  ruby: "Ruby",
  php: "PHP",
  diff: "Diff",
};

const tagAliases: Record<string, CodeLanguage> = {
  rs: "rust",
  rust: "rust",
  ts: "typescript",
  tsx: "typescript",
  typescript: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  javascript: "javascript",
  py: "python",
  python: "python",
  py3: "python",
  swift: "swift",
  kt: "kotlin",
  kts: "kotlin",
  kotlin: "kotlin",
  go: "go",
  golang: "go",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  "c++": "cpp",
  java: "java",
  cs: "csharp",
  csharp: "csharp",
  "c#": "csharp",
  sql: "sql",
  postgres: "sql",
  postgresql: "sql",
  mysql: "sql",
  sqlite: "sql",
  json: "json",
  jsonc: "json",
  html: "html",
  htm: "html",
  xml: "html",
  svg: "html",
  css: "css",
  scss: "css",
  less: "css",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  shell: "bash",
  console: "bash",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  rb: "ruby",
  ruby: "ruby",
  php: "php",
  diff: "diff",
  patch: "diff",
  text: "plain",
  txt: "plain",
  plain: "plain",
  plaintext: "plain",
};

export function languageFromTag(tag: string): CodeLanguage | undefined {
  return tagAliases[tag.trim().toLowerCase()];
}

export type MessageBlock =
  | { kind: "prose"; text: string; start: number; end: number }
  | {
      kind: "code";
      declared?: string;
      language: CodeLanguage;
      detected: boolean;
      code: string;
      start: number;
      end: number;
    };

/** Splits `text` into alternating prose and fenced code block segments. */
export function parseMessageBlocks(text: string): MessageBlock[] {
  const lines: [number, number][] = [];
  for (let start = 0; ;) {
    const newline = text.indexOf("\n", start);
    const end = newline < 0 ? text.length : newline;
    lines.push([start, end]);
    if (newline < 0) break;
    start = end + 1;
  }

  const out: MessageBlock[] = [];
  let proseStart: number | undefined;
  let proseEnd = 0;
  for (let idx = 0; idx < lines.length;) {
    const close = fencedBlockEnd(text, lines, idx);
    if (close !== undefined) {
      if (proseStart !== undefined && proseStart < proseEnd) {
        out.push({ kind: "prose", text: text.slice(proseStart, proseEnd), start: proseStart, end: proseEnd });
      }
      proseStart = undefined;
      const [openStart, openEnd] = lines[idx];
      let declared: string | undefined;
      let codeStart: number;
      let codeEnd: number;
      if (close === idx) {
        const single = singleLineFence(text, openStart, openEnd)!;
        codeStart = single[0];
        codeEnd = single[1];
      } else {
        const tag = text.slice(openStart + 3, openEnd).trim();
        declared = tag || undefined;
        codeStart = openEnd + 1;
        codeEnd = close === idx + 1 ? codeStart : lines[close - 1][1];
      }
      const code = text.slice(codeStart, codeEnd);
      const fromTag = declared ? languageFromTag(declared) : undefined;
      const language = fromTag ?? detectLanguage(code);
      const detected = !fromTag && language !== "plain";
      out.push({ kind: "code", declared, language, detected, code, start: codeStart, end: codeEnd });
      idx = close + 1;
    } else {
      const [s, e] = lines[idx];
      if (proseStart === undefined) proseStart = s;
      proseEnd = e;
      idx++;
    }
  }
  if (proseStart !== undefined && proseStart < proseEnd) {
    out.push({ kind: "prose", text: text.slice(proseStart, proseEnd), start: proseStart, end: proseEnd });
  }
  return out;
}

/** Automatically detects the most likely language of an untagged code snippet. */
export function detectLanguage(code: string): CodeLanguage {
  const trimmed = code.trim();
  if (!trimmed) return "plain";

  const splitLines = trimmed.split("\n");
  const firstLine = splitLines[0] ?? "";
  if (firstLine.startsWith("#!")) {
    const lower = firstLine.toLowerCase();
    if (lower.includes("python")) return "python";
    if (lower.includes("node") || lower.includes("deno") || lower.includes("bun")) return "javascript";
    if (lower.includes("ruby")) return "ruby";
    if (lower.includes("php")) return "php";
    if (lower.includes("sh") || lower.includes("bash") || lower.includes("zsh")) return "bash";
  }

  if (trimmed.includes("<?php")) return "php";

  const diffLines = splitLines.filter(
    (l) => l.startsWith("diff --git ") || l.startsWith("@@ ") || l.startsWith("--- ") || l.startsWith("+++ "),
  ).length;
  if (diffLines >= 2) return "diff";

  if (
    ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) &&
    trimmed.includes('":') &&
    !trimmed.includes("=>") &&
    !trimmed.includes("function ") &&
    !trimmed.includes("fn ") &&
    !trimmed.includes("def ") &&
    !trimmed.includes("const ") &&
    !trimmed.includes("let ") &&
    !trimmed.includes("var ")
  ) {
    return "json";
  }

  const lower = trimmed.toLowerCase();
  if (
    (lower.startsWith("<!doctype html") ||
      lower.includes("<html") ||
      lower.includes("</div>") ||
      lower.includes("</span>") ||
      lower.includes("</section>") ||
      lower.includes("</button>") ||
      lower.includes("</svg>") ||
      (trimmed.startsWith("<") && trimmed.endsWith(">") && trimmed.includes("</"))) &&
    !trimmed.includes("export ") &&
    !trimmed.includes("import ") &&
    !trimmed.includes("const ") &&
    !trimmed.includes("function ")
  ) {
    return "html";
  }

  const scores = new Map<CodeLanguage, number>([
    ["rust", 0],
    ["typescript", 0],
    ["javascript", 0],
    ["python", 0],
    ["swift", 0],
    ["kotlin", 0],
    ["go", 0],
    ["cpp", 0],
    ["c", 0],
    ["java", 0],
    ["csharp", 0],
    ["sql", 0],
    ["css", 0],
    ["bash", 0],
    ["yaml", 0],
    ["toml", 0],
    ["ruby", 0],
  ]);
  const add = (lang: CodeLanguage, pts: number) => scores.set(lang, (scores.get(lang) ?? 0) + pts);
  const score = (lang: CodeLanguage, rules: readonly (readonly [string, number])[], haystack = trimmed) => {
    for (const [pat, pts] of rules) {
      if (haystack.includes(pat)) add(lang, pts);
    }
  };

  score("rust", [
    ["fn ", 3],
    ["let mut ", 4],
    ["pub fn ", 4],
    ["pub struct ", 4],
    ["impl ", 3],
    ["match ", 2],
    ["::", 2],
    ["-> ", 1],
    ["&str", 4],
    ["&mut ", 4],
    ["println!(", 4],
    ["format!(", 3],
    ["vec![", 4],
    ["#[derive(", 5],
    ["use crate::", 5],
    ["use std::", 5],
    ["Result<", 3],
    ["Option<", 3],
    ["Some(", 2],
    ["Ok(", 2],
    ["Err(", 2],
  ]);
  score("typescript", [
    ["interface ", 4],
    [": string", 4],
    [": number", 4],
    [": boolean", 4],
    [": void", 3],
    ["Promise<", 4],
    ["export type ", 5],
    ["export interface ", 5],
    ["as const", 4],
    ["readonly ", 3],
    ["Record<", 4],
    ["type ", 2],
    ["async function ", 3],
    ["const ", 2],
    ["=> ", 2],
    ["import ", 1],
    ["export ", 1],
  ]);
  score("javascript", [
    ["const ", 2],
    ["let ", 1],
    ["function ", 3],
    ["async function ", 3],
    ["=> ", 2],
    ["console.log(", 4],
    ["document.", 3],
    ["window.", 3],
    ["module.exports", 5],
    ["require(", 3],
    ["export default ", 3],
    ["===", 2],
    ["!==", 2],
    ["fetch(", 2],
    ["await ", 1],
  ]);
  score("python", [
    ["def ", 4],
    ["elif ", 5],
    ["self.", 4],
    ["__init__", 5],
    ["__name__", 5],
    ["print(", 3],
    ["lambda ", 3],
    ["None", 2],
    ["True", 1],
    ["False", 1],
    ["async def ", 5],
    ["from ", 1],
    ["import ", 1],
    ['f"', 3],
    ["f'", 3],
    ["raise ", 2],
    ["except ", 4],
  ]);
  if (
    splitLines.some((l) => {
      const t = l.trimEnd();
      return (
        (t.startsWith("def ") ||
          t.startsWith("class ") ||
          t.startsWith("if ") ||
          t.startsWith("for ") ||
          t.startsWith("while ")) &&
        t.endsWith(":")
      );
    })
  ) {
    add("python", 4);
  }
  score("swift", [
    ["func ", 3],
    ["guard let ", 5],
    ["guard ", 3],
    ["if let ", 4],
    ["import SwiftUI", 6],
    ["import Foundation", 5],
    ["import AppKit", 6],
    ["import UIKit", 6],
    ["@MainActor", 5],
    ["@State ", 5],
    ["@Published ", 5],
    ["protocol ", 3],
    ["struct ", 2],
    ["var ", 1],
    ["let ", 1],
    ["NSView", 4],
    ["UIView", 4],
    ["-> ", 1],
  ]);
  score("kotlin", [
    ["fun ", 4],
    ["val ", 3],
    ["data class ", 5],
    ["suspend fun ", 5],
    ["companion object", 5],
    ["@Composable", 6],
    ["when (", 4],
    ["println(", 3],
    ["lateinit var ", 5],
    ["object ", 2],
  ]);
  score("go", [
    ["package main", 6],
    ["func (", 4],
    ["func main()", 5],
    [":=", 4],
    ["fmt.", 4],
    ["if err != nil", 6],
    ["defer ", 3],
    ["go func", 4],
    ["chan ", 3],
    ["interface{}", 4],
  ]);
  score("cpp", [
    ["#include <", 5],
    ['#include "', 4],
    ["#define ", 3],
    ["std::", 5],
    ["nullptr", 5],
    ["cout <<", 5],
    ["template<", 5],
    ["template <", 5],
    ["namespace ", 2],
  ]);
  score("c", [
    ["#include <stdio.h>", 6],
    ["#include <stdlib.h>", 6],
    ["printf(", 4],
    ["malloc(", 4],
    ["sizeof(", 3],
    ["int main(", 4],
  ]);
  score("java", [
    ["public class ", 5],
    ["public static void main", 6],
    ["System.out.println", 6],
    ["private final ", 4],
    ["@Override", 4],
    ["implements ", 3],
    ["extends ", 2],
    ["ArrayList<", 4],
  ]);
  score("csharp", [
    ["using System", 6],
    ["Console.WriteLine", 6],
    ["public async Task", 5],
    ["get; set;", 5],
    ["namespace ", 3],
    ["string[]", 3],
  ]);
  score(
    "sql",
    [
      ["SELECT ", 3],
      ["FROM ", 2],
      ["WHERE ", 2],
      ["INSERT INTO ", 5],
      ["CREATE TABLE ", 5],
      ["ALTER TABLE ", 5],
      ["UPDATE ", 2],
      ["LEFT JOIN ", 4],
      ["INNER JOIN ", 4],
      ["GROUP BY ", 4],
      ["ORDER BY ", 4],
      ["VALUES (", 3],
      ["PRIMARY KEY", 4],
    ],
    trimmed.toUpperCase(),
  );
  if (trimmed.includes("{") && trimmed.includes("}") && trimmed.includes(":")) {
    score("css", [
      ["display:", 3],
      ["color:", 2],
      ["background:", 3],
      ["background-color:", 3],
      ["padding:", 3],
      ["margin:", 3],
      ["border-radius:", 4],
      ["font-size:", 3],
      ["@media ", 4],
      ["var(--", 4],
      ["px;", 2],
      ["rem;", 2],
      [":hover", 3],
    ]);
  }
  score("bash", [
    ["export ", 2],
    ["echo ", 3],
    ["curl ", 3],
    ["sudo ", 3],
    ["grep ", 3],
    ["chmod ", 4],
    ["mkdir ", 3],
    ["rm -rf ", 4],
    ["git ", 2],
    ["cargo ", 2],
    ["npm ", 2],
    ["if [ ", 4],
    ["if [[ ", 5],
    ["\nfi", 4],
    ["\ndone", 4],
    ["$(", 2],
    [" | ", 2],
    [" && ", 2],
  ]);
  if (
    splitLines.some((l) => l.startsWith("[") && l.endsWith("]") && !l.includes(" ")) &&
    splitLines.some((l) => l.includes(" = ")) &&
    !trimmed.includes("{") &&
    !trimmed.includes(";")
  ) {
    add("toml", 6);
  }
  const yamlKv = splitLines.filter((l) => {
    const t = l.trim();
    if (!t || t.startsWith("#") || t.endsWith("{") || t.endsWith(";")) return false;
    const colon = t.indexOf(": ");
    return colon > 0 && /^[A-Za-z0-9_.-]+$/.test(t.slice(0, colon));
  }).length;
  if (yamlKv >= 2 && !trimmed.includes("{") && !trimmed.includes(";")) {
    add("yaml", 4 + Math.min(4, yamlKv));
  }
  score("ruby", [
    ["puts ", 4],
    ["elsif ", 5],
    ["attr_accessor", 5],
    [".each do", 5],
    [" do |", 4],
    ["require '", 4],
    ["\nend", 2],
    ["def ", 2],
  ]);

  let bestLang: CodeLanguage = "plain";
  let bestScore = 0;
  for (const [lang, pts] of scores) {
    if (pts > bestScore) {
      bestScore = pts;
      bestLang = lang;
    }
  }
  return bestScore >= 4 ? bestLang : "plain";
}

export interface TokenSpan {
  start: number;
  end: number;
  kind: TokenKind;
}

const isDigit = (c: string | undefined) => !!c && c >= "0" && c <= "9";
const isHex = (c: string | undefined) => !!c && /^[0-9a-fA-F]$/.test(c);
const isAlpha = (c: string | undefined) => !!c && /^[A-Za-z]$/.test(c);
const isAlnum = (c: string | undefined) => !!c && /^[A-Za-z0-9]$/.test(c);
const isIdent = (c: string | undefined) => !!c && /^[A-Za-z0-9_]$/.test(c);

function pushSpan(out: TokenSpan[], start: number, end: number, kind: TokenKind) {
  if (start >= end) return;
  const last = out.at(-1);
  if (last && last.kind === kind && last.end === start) {
    last.end = end;
    return;
  }
  out.push({ start, end, kind });
}

function scanQuoted(code: string, start: number, quote: string): number {
  let i = start + 1;
  while (i < code.length) {
    if (code[i] === "\\" && i + 1 < code.length) {
      i += 2;
      continue;
    }
    if (code[i] === quote) return i + 1;
    if (code[i] === "\n" && quote !== "`") return i;
    i++;
  }
  return code.length;
}

function scanNumber(code: string, start: number): number {
  let i = start;
  if (code[i] === "-") i++;
  if (code[i] === "0" && /[xXbBoO]/.test(code[i + 1] ?? "")) {
    i += 2;
    while (i < code.length && (isHex(code[i]) || code[i] === "_")) i++;
    return i;
  }
  while (i < code.length && (isDigit(code[i]) || code[i] === "_")) i++;
  if (code[i] === "." && isDigit(code[i + 1])) {
    i++;
    while (i < code.length && (isDigit(code[i]) || code[i] === "_")) i++;
  }
  if (code[i] === "e" || code[i] === "E") {
    i++;
    if (code[i] === "+" || code[i] === "-") i++;
    while (i < code.length && (isDigit(code[i]) || code[i] === "_")) i++;
  }
  return i;
}

const KEYWORDS: Partial<Record<CodeLanguage, Set<string>>> = {
  rust: new Set([
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
  ]),
  typescript: new Set([
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "keyof",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
  ]),
  python: new Set([
    "and",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "class",
    "continue",
    "def",
    "del",
    "elif",
    "else",
    "except",
    "False",
    "finally",
    "for",
    "from",
    "global",
    "if",
    "import",
    "in",
    "is",
    "lambda",
    "match",
    "None",
    "nonlocal",
    "not",
    "or",
    "pass",
    "raise",
    "return",
    "self",
    "True",
    "try",
    "while",
    "with",
    "yield",
  ]),
  swift: new Set([
    "actor",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "enum",
    "extension",
    "false",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "internal",
    "is",
    "lazy",
    "let",
    "mutating",
    "nil",
    "nonisolated",
    "open",
    "override",
    "private",
    "protocol",
    "public",
    "return",
    "self",
    "Self",
    "some",
    "static",
    "struct",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "weak",
    "where",
    "while",
  ]),
  kotlin: new Set([
    "abstract",
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "const",
    "continue",
    "data",
    "do",
    "else",
    "enum",
    "false",
    "finally",
    "for",
    "fun",
    "if",
    "import",
    "in",
    "init",
    "inline",
    "interface",
    "internal",
    "is",
    "lateinit",
    "null",
    "object",
    "open",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "sealed",
    "super",
    "suspend",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "while",
  ]),
  go: new Set([
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
  ]),
  c: new Set([
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "constexpr",
    "continue",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "false",
    "float",
    "for",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "new",
    "nullptr",
    "NULL",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
  ]),
  java: new Set([
    "abstract",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "for",
    "foreach",
    "if",
    "implements",
    "import",
    "in",
    "interface",
    "namespace",
    "new",
    "null",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "return",
    "sealed",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "throws",
    "true",
    "try",
    "using",
    "var",
    "virtual",
    "void",
    "while",
  ]),
  sql: new Set([
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "ALTER",
    "DROP",
    "INDEX",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "ORDER",
    "GROUP",
    "BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "DISTINCT",
    "UNION",
    "ALL",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "DEFAULT",
    "UNIQUE",
    "TRUE",
    "FALSE",
    "RETURNING",
    "WITH",
    "ASC",
    "DESC",
  ]),
  bash: new Set([
    "case",
    "do",
    "done",
    "elif",
    "else",
    "esac",
    "export",
    "fi",
    "for",
    "function",
    "if",
    "in",
    "local",
    "readonly",
    "return",
    "set",
    "then",
    "unset",
    "until",
    "while",
  ]),
  ruby: new Set([
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "raise",
    "rescue",
    "return",
    "self",
    "super",
    "then",
    "true",
    "unless",
    "until",
    "when",
    "while",
    "yield",
  ]),
  php: new Set([
    "abstract",
    "array",
    "as",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "echo",
    "else",
    "elseif",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "if",
    "implements",
    "interface",
    "match",
    "namespace",
    "new",
    "null",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "true",
    "try",
    "use",
    "while",
  ]),
};
KEYWORDS.javascript = KEYWORDS.typescript;
KEYWORDS.cpp = KEYWORDS.c;
KEYWORDS.csharp = KEYWORDS.java;

const BUILTIN_TYPES: Partial<Record<CodeLanguage, Set<string>>> = {
  rust: new Set([
    "bool",
    "char",
    "str",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "f32",
    "f64",
    "String",
    "Vec",
    "Option",
    "Result",
    "Box",
    "Arc",
    "Rc",
  ]),
  typescript: new Set([
    "string",
    "number",
    "boolean",
    "bigint",
    "symbol",
    "unknown",
    "never",
    "any",
    "object",
    "Promise",
    "Record",
    "Array",
    "Map",
    "Set",
  ]),
  go: new Set([
    "bool",
    "byte",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "any",
  ]),
  python: new Set(["int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes"]),
  sql: new Set([
    "INT",
    "INTEGER",
    "BIGINT",
    "SMALLINT",
    "TEXT",
    "VARCHAR",
    "CHAR",
    "BOOLEAN",
    "BOOL",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "DATE",
    "UUID",
    "JSON",
    "JSONB",
    "NUMERIC",
    "FLOAT",
    "REAL",
    "SERIAL",
    "BYTEA",
  ]),
  java: new Set(["int", "long", "short", "byte", "float", "double", "char", "bool", "string", "decimal", "object"]),
};
BUILTIN_TYPES.csharp = BUILTIN_TYPES.java;

function splitLinesWithNewline(code: string): string[] {
  const out: string[] = [];
  let start = 0;
  for (let i = 0; i < code.length; i++) {
    if (code[i] === "\n") {
      out.push(code.slice(start, i + 1));
      start = i + 1;
    }
  }
  if (start < code.length) out.push(code.slice(start));
  return out;
}

function highlightYamlValue(out: TokenSpan[], code: string, start: number, end: number) {
  let i = start;
  while (i < end) {
    if (code[i] === "#" && (i === start || /\s/.test(code[i - 1]))) {
      pushSpan(out, i, end, "comment");
      return;
    }
    if (code[i] === '"' || code[i] === "'") {
      const s = i;
      i = Math.min(end, scanQuoted(code, i, code[i]));
      pushSpan(out, s, i, "string");
    } else if (isDigit(code[i]) && (i === start || !isAlnum(code[i - 1]))) {
      const s = i;
      i = Math.min(end, scanNumber(code, i));
      pushSpan(out, s, i, "number");
    } else if (isAlpha(code[i])) {
      const s = i;
      while (i < end && isIdent(code[i])) i++;
      const word = code.slice(s, i);
      pushSpan(out, s, i, /^(true|false|null|yes|no)$/.test(word) ? "keyword" : "plain");
    } else {
      pushSpan(out, i, i + 1, "plain");
      i++;
    }
  }
}

/** Tokenizes `code` in `language` into contiguous, non-overlapping spans covering `0..code.length`. */
export function highlightCode(code: string, language: CodeLanguage): TokenSpan[] {
  if (!code) return [];
  if (language === "plain") return [{ start: 0, end: code.length, kind: "plain" }];

  const out: TokenSpan[] = [];
  const n = code.length;

  if (language === "diff") {
    let offset = 0;
    for (const chunk of splitLinesWithNewline(code)) {
      const kind: TokenKind =
        chunk.startsWith("@@") || chunk.startsWith("diff ") || chunk.startsWith("index ")
          ? "attribute"
          : chunk.startsWith("+++") || chunk.startsWith("---")
            ? "keyword"
            : chunk.startsWith("+")
              ? "addition"
              : chunk.startsWith("-")
                ? "deletion"
                : "plain";
      pushSpan(out, offset, offset + chunk.length, kind);
      offset += chunk.length;
    }
    return out;
  }

  if (language === "html") {
    let i = 0;
    while (i < n) {
      if (code.startsWith("<!--", i)) {
        const close = code.indexOf("-->", i + 4);
        const end = close < 0 ? n : close + 3;
        pushSpan(out, i, end, "comment");
        i = end;
      } else if (code[i] === "<") {
        const start = i++;
        if (i < n && (code[i] === "/" || code[i] === "!" || code[i] === "?")) i++;
        const nameStart = i;
        while (i < n && /[A-Za-z0-9_:-]/.test(code[i])) i++;
        pushSpan(out, start, nameStart, "plain");
        pushSpan(out, nameStart, i, "keyword");
        while (i < n && code[i] !== ">") {
          if (code[i] === '"' || code[i] === "'") {
            const q = code[i];
            const s = i++;
            while (i < n && code[i] !== q) i++;
            if (i < n) i++;
            pushSpan(out, s, i, "string");
          } else if (isAlpha(code[i]) || code[i] === "_") {
            const s = i;
            while (i < n && /[A-Za-z0-9_:-]/.test(code[i])) i++;
            pushSpan(out, s, i, "attribute");
          } else {
            pushSpan(out, i, i + 1, "plain");
            i++;
          }
        }
        if (i < n && code[i] === ">") {
          pushSpan(out, i, i + 1, "plain");
          i++;
        }
      } else {
        pushSpan(out, i, i + 1, "plain");
        i++;
      }
    }
    return out;
  }

  if (language === "json") {
    let i = 0;
    while (i < n) {
      if (code[i] === "/" && code[i + 1] === "/") {
        const nl = code.indexOf("\n", i);
        const end = nl < 0 ? n : nl;
        pushSpan(out, i, end, "comment");
        i = end;
      } else if (code[i] === '"') {
        const s = i;
        i = scanQuoted(code, i, '"');
        let look = i;
        while (look < n && /\s/.test(code[look])) look++;
        pushSpan(out, s, i, code[look] === ":" ? "attribute" : "string");
      } else if (isDigit(code[i]) || (code[i] === "-" && isDigit(code[i + 1]))) {
        const s = i;
        i = scanNumber(code, i);
        pushSpan(out, s, i, "number");
      } else if (isAlpha(code[i])) {
        const s = i;
        while (i < n && isAlnum(code[i])) i++;
        const word = code.slice(s, i);
        pushSpan(out, s, i, word === "true" || word === "false" || word === "null" ? "keyword" : "plain");
      } else {
        pushSpan(out, i, i + 1, "plain");
        i++;
      }
    }
    return out;
  }

  if (language === "yaml" || language === "toml") {
    let offset = 0;
    for (const chunk of splitLinesWithNewline(code)) {
      const trimmed = chunk.trimStart();
      const lead = chunk.length - trimmed.length;
      if (trimmed.startsWith("#")) {
        pushSpan(out, offset, offset + lead, "plain");
        pushSpan(out, offset + lead, offset + chunk.length, "comment");
      } else if (language === "toml" && trimmed.startsWith("[")) {
        pushSpan(out, offset, offset + lead, "plain");
        pushSpan(out, offset + lead, offset + chunk.length, "keyword");
      } else if (language === "toml" && trimmed.includes("=")) {
        const eq = trimmed.indexOf("=");
        pushSpan(out, offset, offset + lead, "plain");
        pushSpan(out, offset + lead, offset + lead + eq, "attribute");
        highlightYamlValue(out, code, offset + lead + eq, offset + chunk.length);
      } else if (language === "yaml" && !trimmed.startsWith("-") && trimmed.indexOf(": ") > 0) {
        const colon = trimmed.indexOf(": ");
        pushSpan(out, offset, offset + lead, "plain");
        pushSpan(out, offset + lead, offset + lead + colon, "attribute");
        highlightYamlValue(out, code, offset + lead + colon, offset + chunk.length);
      } else {
        highlightYamlValue(out, code, offset, offset + chunk.length);
      }
      offset += chunk.length;
    }
    return out;
  }

  if (language === "css") {
    let i = 0;
    let depth = 0;
    while (i < n) {
      if (code[i] === "/" && code[i + 1] === "*") {
        const close = code.indexOf("*/", i + 2);
        const end = close < 0 ? n : close + 2;
        pushSpan(out, i, end, "comment");
        i = end;
      } else if (code[i] === '"' || code[i] === "'") {
        const s = i;
        i = scanQuoted(code, i, code[i]);
        pushSpan(out, s, i, "string");
      } else if (code[i] === "@") {
        const s = i++;
        while (i < n && /[A-Za-z0-9-]/.test(code[i])) i++;
        pushSpan(out, s, i, "keyword");
      } else if (code[i] === "{") {
        depth++;
        pushSpan(out, i, i + 1, "plain");
        i++;
      } else if (code[i] === "}") {
        depth = Math.max(0, depth - 1);
        pushSpan(out, i, i + 1, "plain");
        i++;
      } else if (code[i] === "#" && depth > 0) {
        const s = i++;
        while (i < n && isHex(code[i])) i++;
        pushSpan(out, s, i, "number");
      } else if (isDigit(code[i])) {
        const s = i;
        i = scanNumber(code, i);
        while (i < n && (isAlpha(code[i]) || code[i] === "%")) i++;
        pushSpan(out, s, i, "number");
      } else if (isAlpha(code[i]) || code[i] === "-" || code[i] === ".") {
        const s = i;
        while (i < n && /[A-Za-z0-9_.-]/.test(code[i])) i++;
        let look = i;
        while (look < n && code[look] === " ") look++;
        const kind: TokenKind =
          depth > 0 && code[look] === ":"
            ? "attribute"
            : code[look] === "("
              ? "function"
              : depth === 0
                ? "type"
                : "plain";
        pushSpan(out, s, i, kind);
      } else {
        pushSpan(out, i, i + 1, "plain");
        i++;
      }
    }
    return out;
  }

  const hashComment = language === "python" || language === "bash" || language === "ruby" || language === "php";
  const dashComment = language === "sql";
  const slashComment = language !== "python" && language !== "bash" && language !== "ruby";
  const backtickString =
    language === "javascript" || language === "typescript" || language === "go" || language === "bash";
  const singleQuoteString = language !== "rust";
  const kw = KEYWORDS[language];
  const types = BUILTIN_TYPES[language];

  let i = 0;
  while (i < n) {
    if (slashComment && code[i] === "/" && code[i + 1] === "/") {
      const nl = code.indexOf("\n", i);
      const end = nl < 0 ? n : nl;
      pushSpan(out, i, end, "comment");
      i = end;
    } else if (slashComment && code[i] === "/" && code[i + 1] === "*") {
      const close = code.indexOf("*/", i + 2);
      const end = close < 0 ? n : close + 2;
      pushSpan(out, i, end, "comment");
      i = end;
    } else if (dashComment && code[i] === "-" && code[i + 1] === "-") {
      const nl = code.indexOf("\n", i);
      const end = nl < 0 ? n : nl;
      pushSpan(out, i, end, "comment");
      i = end;
    } else if (hashComment && code[i] === "#") {
      const nl = code.indexOf("\n", i);
      const end = nl < 0 ? n : nl;
      pushSpan(out, i, end, "comment");
      i = end;
    } else if (code[i] === "#" && (language === "rust" || language === "c" || language === "cpp")) {
      if (language === "rust" && (code[i + 1] === "[" || (code[i + 1] === "!" && code[i + 2] === "["))) {
        const s = i;
        let depth = 0;
        while (i < n) {
          if (code[i] === "[") depth++;
          else if (code[i] === "]") {
            depth = Math.max(0, depth - 1);
            if (depth === 0) {
              i++;
              break;
            }
          }
          i++;
        }
        pushSpan(out, s, i, "attribute");
      } else {
        const s = i++;
        while (i < n && isAlpha(code[i])) i++;
        pushSpan(out, s, i, "attribute");
      }
    } else if (
      code[i] === "@" &&
      (language === "swift" ||
        language === "kotlin" ||
        language === "java" ||
        language === "python" ||
        language === "typescript") &&
      (isAlpha(code[i + 1]) || code[i + 1] === "_")
    ) {
      const s = i++;
      while (i < n && isIdent(code[i])) i++;
      pushSpan(out, s, i, "attribute");
    } else if (code[i] === '"' || (singleQuoteString && code[i] === "'") || (backtickString && code[i] === "`")) {
      const q = code[i];
      const s = i;
      if (language === "python" && (q === '"' || q === "'") && code[i + 1] === q && code[i + 2] === q) {
        const close = code.indexOf(q + q + q, i + 3);
        i = close < 0 ? n : close + 3;
      } else {
        i = scanQuoted(code, i, q);
      }
      pushSpan(out, s, i, "string");
    } else if (language === "rust" && code[i] === "'") {
      const s = i;
      if (code[i + 1] === "\\") {
        const close = code.indexOf("'", i + 3);
        const end = close < 0 ? i + 1 : close + 1;
        pushSpan(out, s, end, "string");
        i = end;
      } else if (code[i + 2] === "'") {
        pushSpan(out, s, i + 3, "string");
        i += 3;
      } else if (isAlpha(code[i + 1]) || code[i + 1] === "_") {
        i++;
        while (i < n && isIdent(code[i])) i++;
        pushSpan(out, s, i, "attribute");
      } else {
        pushSpan(out, i, i + 1, "plain");
        i++;
      }
    } else if (isDigit(code[i]) && (i === 0 || !isIdent(code[i - 1]))) {
      const s = i;
      i = scanNumber(code, i);
      pushSpan(out, s, i, "number");
    } else if (
      isAlpha(code[i]) ||
      code[i] === "_" ||
      (code[i] === "$" && (language === "php" || language === "bash"))
    ) {
      const s = i++;
      while (i < n && isIdent(code[i])) i++;
      const word = code.slice(s, i);
      const lookup = language === "sql" ? word.toUpperCase() : word;
      let kind: TokenKind = "plain";
      if (code[s] === "$") kind = "attribute";
      else if (kw?.has(lookup)) kind = "keyword";
      else if (types?.has(lookup)) kind = "type";
      else if (language === "rust" && code[i] === "!") {
        i++;
        kind = "function";
      } else if (code[i] === "(") kind = "function";
      else if (word.length > 1 && /[A-Z]/.test(word[0]) && /[a-z]/.test(word.slice(1))) kind = "type";
      pushSpan(out, s, i, kind);
    } else {
      pushSpan(out, i, i + 1, "plain");
      i++;
    }
  }
  return out;
}
