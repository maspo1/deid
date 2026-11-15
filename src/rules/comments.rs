//! 소스코드 주석 탐지.
//! 문자열 리터럴 안의 `//`, `#` 등은 주석으로 보지 않도록 간단한 렉서로 처리한다.
//! 반환값은 주석 "본문"의 바이트 범위(주석 기호 제외)이다.

/// 주석 문법 계열
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// 주석 개념 없음 (txt, log, json, csv ...)
    None,
    /// `//`, `/* */` (C, C++, C#, Java, JS, TS, Go, Kotlin, Swift ...)
    CLike,
    /// CLike + 문자 리터럴 없음 (Rust: 'a 수명 표기)
    Rust,
    /// `/* */` 만 (CSS)
    Css,
    /// `//`, `#`, `/* */`
    Php,
    /// `#` (shell, ruby, perl, yaml, toml, conf ...)
    Hash,
    /// `#` + 독스트링
    Python,
    /// `#`, `<# #>`
    PowerShell,
    /// `--`, `/* */`
    Sql,
    /// `<!-- -->` + `<script>`/`<style>` 내부
    Markup,
    /// 줄 시작 `;`, `#`
    Ini,
    /// `--`, `--[[ ]]`
    Lua,
    /// 줄 시작 `REM`, `::`
    Batch,
    /// `'`
    Vb,
}

impl Lang {
    pub fn is_code(&self) -> bool {
        *self != Lang::None
    }
}

/// 파일 이름(확장자)과 내용으로 언어 판별
pub fn lang_from_name(name: &str, text: &str) -> Lang {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    match base.as_str() {
        "dockerfile" | "makefile" | "cmakelists.txt" | ".gitignore" | ".env" | ".bashrc" | ".profile" | ".htaccess" => {
            return Lang::Hash
        }
        _ => {}
    }
    let ext = base.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" | "cs" | "java" | "js" | "mjs" | "cjs" | "jsx" | "ts"
        | "tsx" | "go" | "swift" | "kt" | "kts" | "scala" | "dart" | "groovy" | "gradle" | "m" | "mm" | "proto"
        | "jsonc" | "json5" | "v" | "sv" | "glsl" | "hlsl" | "sol" | "zig" | "scss" | "less" | "ino" => Lang::CLike,
        "rs" => Lang::Rust,
        "css" => Lang::Css,
        "php" | "phtml" => Lang::Php,
        "py" | "pyw" | "pyi" => Lang::Python,
        "sh" | "bash" | "zsh" | "ksh" | "rb" | "pl" | "pm" | "r" | "yaml" | "yml" | "toml" | "conf" | "cfg"
        | "properties" | "mk" | "cmake" | "nim" | "tcl" | "awk" | "ex" | "exs" | "coffee" | "env" | "tf" => Lang::Hash,
        "ps1" | "psm1" | "psd1" => Lang::PowerShell,
        "sql" => Lang::Sql,
        "html" | "htm" | "xhtml" | "xml" | "svg" | "vue" | "jsp" | "asp" | "aspx" | "xaml" | "md" | "markdown"
        | "svelte" | "cshtml" | "razor" => Lang::Markup,
        "ini" | "inf" | "reg" => Lang::Ini,
        "lua" => Lang::Lua,
        "bat" | "cmd" => Lang::Batch,
        "vb" | "vbs" | "bas" | "cls" | "frm" => Lang::Vb,
        _ => {
            // 확장자가 없으면 shebang 으로 판단
            if text.starts_with("#!") {
                if text.lines().next().map_or(false, |l| l.contains("python")) {
                    Lang::Python
                } else {
                    Lang::Hash
                }
            } else {
                Lang::None
            }
        }
    }
}

struct Syntax {
    line: &'static [&'static str],
    block: &'static [(&'static str, &'static str)],
    /// 한 줄 문자열 따옴표
    quotes: &'static [u8],
    /// 여러 줄 허용 따옴표 (JS 템플릿 리터럴 등)
    multiline_quotes: &'static [u8],
    /// 파이썬 삼중따옴표 독스트링
    triple: bool,
    /// 줄 주석이 줄 시작에서만 인정됨 (ini, bat)
    line_start_only: bool,
    /// `#` 앞이 영숫자/`$`/`{`면 주석 아님 (shell, yaml)
    strict_hash: bool,
}

const CLIKE: Syntax = Syntax { line: &["//"], block: &[("/*", "*/")], quotes: b"\"'`", multiline_quotes: b"`", triple: false, line_start_only: false, strict_hash: false };
const RUST: Syntax = Syntax { line: &["//"], block: &[("/*", "*/")], quotes: b"\"", multiline_quotes: b"\"", triple: false, line_start_only: false, strict_hash: false };
const CSS: Syntax = Syntax { line: &[], block: &[("/*", "*/")], quotes: b"\"'", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };
const PHP: Syntax = Syntax { line: &["//", "#"], block: &[("/*", "*/")], quotes: b"\"'`", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };
const HASH: Syntax = Syntax { line: &["#"], block: &[], quotes: b"\"'", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: true };
const PYTHON: Syntax = Syntax { line: &["#"], block: &[], quotes: b"\"'", multiline_quotes: b"", triple: true, line_start_only: false, strict_hash: false };
const POWERSHELL: Syntax = Syntax { line: &["#"], block: &[("<#", "#>")], quotes: b"\"'", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: true };
const SQL: Syntax = Syntax { line: &["--"], block: &[("/*", "*/")], quotes: b"\"'`", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };
const MARKUP: Syntax = Syntax { line: &[], block: &[("<!--", "-->")], quotes: b"", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };
const INI: Syntax = Syntax { line: &[";", "#"], block: &[], quotes: b"", multiline_quotes: b"", triple: false, line_start_only: true, strict_hash: false };
const LUA: Syntax = Syntax { line: &["--"], block: &[("--[[", "]]")], quotes: b"\"'", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };
const BATCH: Syntax = Syntax { line: &["REM ", "rem ", "Rem ", "@REM ", "@rem ", "::"], block: &[], quotes: b"", multiline_quotes: b"", triple: false, line_start_only: true, strict_hash: false };
const VB: Syntax = Syntax { line: &["'", "REM ", "Rem "], block: &[], quotes: b"\"", multiline_quotes: b"", triple: false, line_start_only: false, strict_hash: false };

fn syntax(lang: Lang) -> Option<&'static Syntax> {
    Some(match lang {
        Lang::None => return None,
        Lang::CLike => &CLIKE,
        Lang::Rust => &RUST,
        Lang::Css => &CSS,
        Lang::Php => &PHP,
        Lang::Hash => &HASH,
        Lang::Python => &PYTHON,
        Lang::PowerShell => &POWERSHELL,
        Lang::Sql => &SQL,
        Lang::Markup => &MARKUP,
        Lang::Ini => &INI,
        Lang::Lua => &LUA,
        Lang::Batch => &BATCH,
        Lang::Vb => &VB,
    })
}

fn find_from(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    crate::common::find_bytes(&b[from.min(b.len())..], pat).map(|p| p + from)
}

fn eol(b: &[u8], from: usize) -> usize {
    b[from..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| p + from)
}

/// 따옴표 문자열 건너뛰기 → 닫는 따옴표 다음 위치
fn skip_string(b: &[u8], i: usize, q: u8, multiline: bool) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => return j + 1,
            b'\n' if !multiline => return j,
            _ => j += 1,
        }
    }
    b.len()
}

fn lex(text: &str, syn: &Syntax, base: usize, out: &mut Vec<(usize, usize)>) {
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut at_line_start = true;

    'outer: while i < n {
        let c = b[i];
        if c == b'\n' {
            at_line_start = true;
            i += 1;
            continue;
        }
        // shebang 은 주석이 아니라 실행 지시문
        if i == 0 && b.starts_with(b"#!") {
            i = eol(b, 0);
            continue;
        }
        // 파이썬 독스트링 (줄 시작에 오는 삼중따옴표만 주석 취급)
        if syn.triple && (b[i..].starts_with(b"\"\"\"") || b[i..].starts_with(b"'''")) {
            let q = &b[i..i + 3];
            let end = find_from(b, i + 3, q).unwrap_or(n);
            if at_line_start {
                out.push((base + i + 3, base + end));
            }
            i = (end + 3).min(n);
            at_line_start = false;
            continue;
        }
        if syn.quotes.contains(&c) {
            i = skip_string(b, i, c, syn.multiline_quotes.contains(&c));
            at_line_start = false;
            continue;
        }
        for (open, close) in syn.block {
            if b[i..].starts_with(open.as_bytes()) {
                let s = i + open.len();
                let e = find_from(b, s, close.as_bytes()).unwrap_or(n);
                out.push((base + s, base + e));
                i = (e + close.len()).min(n);
                at_line_start = false;
                continue 'outer;
            }
        }
        for m in syn.line {
            if !b[i..].starts_with(m.as_bytes()) || (syn.line_start_only && !at_line_start) {
                continue;
            }
            if *m == "#" {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                if syn.strict_hash && (prev.is_ascii_alphanumeric() || prev == b'$' || prev == b'{' || prev == b'/') {
                    continue;
                }
                if b[i..].starts_with(b"#[") && std::ptr::eq(syn, &PHP) {
                    continue; // PHP 8 attribute
                }
            }
            let s = i + m.len();
            let mut e = eol(b, s);
            if e > s && b[e - 1] == b'\r' {
                e -= 1;
            }
            let body = &text[s..e];
            // 인코딩 선언(# -*- coding: utf-8 -*-)은 유지
            if !(m.starts_with('#') && body.contains("coding") && (body.contains("-*-") || body.trim_start().starts_with("coding") || body.contains("vim:"))) {
                out.push((base + s, base + e));
            }
            i = e;
            continue 'outer;
        }
        if !c.is_ascii_whitespace() {
            at_line_start = false;
        }
        i += 1;
    }
}

/// `<script>` / `<style>` 블록 내부 범위
fn embedded_blocks(text: &str, tag: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut v = Vec::new();
    let mut pos = 0;
    while let Some(p) = lower[pos..].find(&open) {
        let tag_start = pos + p;
        let Some(gt) = lower[tag_start..].find('>') else { break };
        let s = tag_start + gt + 1;
        let e = lower[s..].find(&close).map_or(text.len(), |q| s + q);
        v.push((s, e));
        pos = e;
    }
    v
}

/// 주석 본문 범위 목록 (영숫자가 하나도 없는 장식용 주석은 제외)
pub fn find_comments(text: &str, lang: Lang) -> Vec<(usize, usize)> {
    let Some(syn) = syntax(lang) else { return Vec::new() };
    let mut out = Vec::new();
    lex(text, syn, 0, &mut out);
    if lang == Lang::Markup {
        for (s, e) in embedded_blocks(text, "script") {
            lex(&text[s..e], &CLIKE, s, &mut out);
        }
        for (s, e) in embedded_blocks(text, "style") {
            lex(&text[s..e], &CSS, s, &mut out);
        }
    }
    out.retain(|&(s, e)| e > s && text[s..e].chars().any(|c| c.is_alphanumeric()));
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bodies(t: &str, l: Lang) -> Vec<String> {
        find_comments(t, l).iter().map(|&(s, e)| t[s..e].trim().to_string()).collect()
    }

    #[test]
    fn clike() {
        let t = "int a = 1; // 관리자 연락처\nchar *u = \"http://x.com\"; /* block\n note */";
        assert_eq!(bodies(t, Lang::CLike), vec!["관리자 연락처", "block\n note"]);
    }

    #[test]
    fn python() {
        let t = "#!/usr/bin/env python\n# -*- coding: utf-8 -*-\ndef f():\n    \"\"\"doc string\"\"\"\n    s = \"#notcomment\"  # real\n";
        assert_eq!(bodies(t, Lang::Python), vec!["doc string", "real"]);
    }

    #[test]
    fn shell_hash() {
        let t = "echo $# ${#arr} # yes\nURL=http://a/#frag";
        assert_eq!(bodies(t, Lang::Hash), vec!["yes"]);
    }

    #[test]
    fn markup() {
        let t = "<!-- 주석 --><script>var a=1; // js 주석\n</script><style>/* css */</style>";
        assert_eq!(bodies(t, Lang::Markup), vec!["주석", "js 주석", "css"]);
    }
}
