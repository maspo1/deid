//! 맥락 기반 탐지 — 패스워드, 쿠키, 세션/인증 값.
//!
//! 구성
//! 1. 형식이 확정적인 자격증명(AWS/GitHub/Slack/JWT/PEM 개인키, URI 내 비밀번호 등) → 규칙으로 바로 검출
//! 2. `키 = 값` 형태 후보 → 특징(feature) 추출 → `ContextModel` 이 확률 p 산출 → p ≥ 임계값(0.7)이면 채택
//!
//! `ContextModel` 은 trait 이다. 현재는 수작업 가중치의 로지스틱 기준모델(`LogisticBaseline`)이며,
//! 사용자 정답셋이 쌓이면 같은 특징 벡터로 학습한 XGBoost 트리 모델로 교체한다.

use super::dict;
use crate::common::{char_class_count, context_window, shannon_entropy, Hit, InfoType, MaskMode};
use regex::Regex;

// ───────────────────────────── 특징 / 모델 ─────────────────────────────

/// 맥락 모델 입력 특징
#[derive(Clone, Debug, Default)]
pub struct Features {
    /// 키 이름의 강도 (0 = 무관, 1.5~2.5)
    pub key_strength: f32,
    /// 주변에 HTTP 관련 맥락이 있는가
    pub http_ctx: bool,
    /// HTTP 맥락에서만 의미가 있는 키인가 (쿠키/세션)
    pub needs_http: bool,
    /// 키에 부정 접미어 (timeout, policy, hint ...)
    pub negative_key: bool,
    pub value_len: usize,
    pub entropy: f32,
    pub char_classes: usize,
    pub quoted: bool,
    pub placeholder: bool,
    pub code_expr: bool,
    /// 따옴표 없는 짧은 영문 식별자 (변수명일 가능성)
    pub bare_identifier: bool,
}

impl Features {
    /// XGBoost 학습/추론용 수치 벡터 (열 순서 고정)
    pub fn to_vec(&self) -> [f32; 11] {
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        [
            self.key_strength,
            b(self.http_ctx),
            b(self.needs_http),
            b(self.negative_key),
            self.value_len as f32,
            self.entropy,
            self.char_classes as f32,
            b(self.quoted),
            b(self.placeholder),
            b(self.code_expr),
            b(self.bare_identifier),
        ]
    }
}

pub trait ContextModel: Send + Sync {
    /// 민감정보일 확률 (0~1)
    fn predict(&self, f: &Features) -> f32;
}

/// 기준 모델: 로지스틱 회귀 형태의 수작업 가중치
pub struct LogisticBaseline;

impl ContextModel for LogisticBaseline {
    fn predict(&self, f: &Features) -> f32 {
        if f.key_strength <= 0.0 {
            return 0.0;
        }
        let mut z = -1.0 + f.key_strength;
        if f.http_ctx {
            z += 1.0;
        }
        if f.needs_http && !f.http_ctx {
            z -= 2.0;
        }
        if f.value_len >= 8 {
            z += 0.6;
        }
        if f.value_len < 4 {
            z -= 2.5;
        }
        if f.char_classes >= 2 {
            z += 0.6;
        }
        if f.char_classes >= 3 {
            z += 0.4;
        }
        if f.entropy >= 3.5 {
            z += 0.4;
        }
        if f.quoted {
            z += 0.3;
        }
        if f.placeholder {
            z -= 6.0;
        }
        if f.code_expr {
            z -= 3.5;
        }
        if f.negative_key {
            z -= 3.0;
        }
        if f.bare_identifier {
            z -= 0.8;
        }
        1.0 / (1.0 + (-z).exp())
    }
}

// ───────────────────────────── 키 분류 ─────────────────────────────

#[derive(Clone, Debug)]
pub struct KeyClass {
    pub kind: InfoType,
    pub strength: f32,
    pub needs_http: bool,
    pub negative: bool,
}

/// 카멜/스네이크/케밥 표기를 소문자 토큰으로 분해
fn key_tokens(key: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in key.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            toks.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

const NEGATIVE_TOKENS: &[&str] = &[
    "timeout", "expire", "expires", "expiry", "expiration", "length", "len", "count", "name", "type", "path",
    "domain", "max", "min", "size", "mode", "enabled", "enable", "disabled", "policy", "url", "uri", "file", "dir",
    "field", "label", "hint", "prompt", "required", "reset", "change", "format", "header", "endpoint", "storage",
    "store", "manager", "class", "error", "message", "msg", "text", "placeholder", "regex", "pattern",
    "validation", "valid", "visible", "show", "toggle", "input", "age", "lifetime", "ttl", "id_type", "strength",
    "algorithm", "algo", "method", "duration", "interval", "limit", "version", "scope", "provider",
];

pub fn classify_key(key: &str) -> Option<KeyClass> {
    let k = key.to_lowercase();
    let toks = key_tokens(key);
    let has = |t: &str| toks.iter().any(|x| x == t);
    let negative = toks.iter().any(|t| NEGATIVE_TOKENS.contains(&t.as_str()));
    let mk = |kind, strength, needs_http| Some(KeyClass { kind, strength, needs_http, negative });

    if ["password", "passwd", "passphrase", "passcode", "비밀번호", "패스워드"].iter().any(|w| k.contains(w)) {
        return mk(InfoType::Password, 2.5, false);
    }
    if has("pwd") || has("pass") || has("pw") || has("암호") {
        return mk(InfoType::Password, 2.0, false);
    }
    if ["jsessionid", "phpsessid", "asp.net_sessionid", "sessionid", "session_id", "sessiontoken", "session_token", "sessionkey", "session_key"]
        .iter()
        .any(|w| k.contains(w))
    {
        return mk(InfoType::Session, 2.5, true);
    }
    if has("session") || has("sess") || has("sessid") || has("sid") || has("세션") {
        return mk(InfoType::Session, 1.8, true);
    }
    if has("cookie") || has("쿠키") {
        return mk(InfoType::Cookie, 1.8, true);
    }
    if ["secret", "apikey", "api_key", "api-key", "accesskey", "access_key", "privatekey", "private_key", "authkey", "auth_key", "credential"]
        .iter()
        .any(|w| k.contains(w))
        || has("token")
        || k.ends_with("token")
    {
        return mk(InfoType::Session, 2.5, false);
    }
    if has("auth") || has("authorization") || has("인증키") {
        return mk(InfoType::Session, 1.5, false);
    }
    None
}

// ───────────────────────────── HTTP 맥락 ─────────────────────────────

const HTTP_MARKERS: &[&str] = &[
    "http/1", "http/2", "cookie", "set-cookie", "authorization", "user-agent", "host:", "content-type", "get /",
    "post /", "put /", "http://", "https://", "header", "curl ", "request", "response", "jsessionid", "phpsessid",
    "document.cookie", "xmlhttprequest", "fetch(", "axios", "requests.", "session.get", "referer", "x-csrf",
    "csrf", "bearer",
];

pub fn has_http_context(text: &str, start: usize, end: usize) -> bool {
    // 표식 29개를 하나씩 찾지 않고 아호-코라식으로 창을 한 번만 훑는다
    static AC: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    let ac = AC.get_or_init(|| aho_corasick::AhoCorasick::new(HTTP_MARKERS).expect("HTTP 표식 오토마타"));
    ac.is_match(context_window(text, start, end, 500, 500).as_str())
}

// ───────────────────────────── 스캐너 ─────────────────────────────

pub struct CredentialScanner {
    /// (정규식, 치환할 그룹 번호)
    tokens: Vec<(Regex, usize)>,
    pem: Regex,
    uri_cred: Regex,
    cookie_header: Regex,
    auth_header: Regex,
    bearer: Regex,
    aws_secret: Regex,
    kv: Regex,
    model: Box<dyn ContextModel>,
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("내장 정규식 오류")
}

const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "token", "digest", "negotiate", "ntlm", "oauth", "apikey"];

const SET_COOKIE_ATTRS: &[&str] = &["path", "domain", "expires", "max-age", "secure", "httponly", "samesite", "priority", "partitioned", "version", "comment"];

impl CredentialScanner {
    pub fn new() -> Self {
        Self::with_model(Box::new(LogisticBaseline))
    }

    pub fn with_model(model: Box<dyn ContextModel>) -> Self {
        let tokens = [
            (r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|ANPA)[0-9A-Z]{16}\b", 0),              // AWS Access Key ID
            (r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,255}\b", 0),                // GitHub PAT
            (r"\bgithub_pat_[A-Za-z0-9_]{22,255}\b", 0),                            // GitHub fine-grained
            (r"\bglpat-[A-Za-z0-9_\-]{20,}", 0),                                    // GitLab
            (r"\bxox[abposr]-[A-Za-z0-9\-]{10,}", 0),                               // Slack
            (r"\beyJ[A-Za-z0-9_\-]{5,}\.eyJ[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]{10,}", 0), // JWT
            (r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_\-]{20,}", 0),                        // OpenAI/Anthropic
            (r"\bAIza[0-9A-Za-z_\-]{35}\b", 0),                                     // Google API Key
            (r"\b(?:sk|rk|pk)_(?:live|test)_[0-9A-Za-z]{16,}\b", 0),                // Stripe
            (r"\bnpm_[A-Za-z0-9]{36}\b", 0),                                        // npm
        ];
        CredentialScanner {
            tokens: tokens.iter().map(|(p, g)| (re(p), *g)).collect(),
            pem: re(r"-----BEGIN ((?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?)-----((?s:.*?))-----END "),
            uri_cred: re(r"\b[A-Za-z][A-Za-z0-9+.\-]{1,15}://([^:/\s@'\x22]+):([^@\s/'\x22]+)@"),
            cookie_header: re(r"(?im)^[ \t]*(?:set-)?cookie[ \t]*:[ \t]*([^\r\n]+)"),
            auth_header: re(r"(?i)\b(?:proxy-)?authorization\x22?'?[ \t]*[:=][ \t]*\x22?'?(?:bearer|basic|token|digest|negotiate)[ \t]+([A-Za-z0-9\-._~+/]+=*)"),
            bearer: re(r"(?i)\bbearer[ \t]+([A-Za-z0-9\-._~+/]{16,}=*)"),
            aws_secret: re(r"[A-Za-z0-9/+]{40}"),
            kv: re(concat!(
                r#"(?i)(?P<key>[A-Za-z0-9_.\-]*(?:pass|pwd|pw|secret|token|api[_\-]?key|access[_\-]?key|auth|private[_\-]?key|credential|sess|sid|cookie)[A-Za-z0-9_.\-]*|비밀번호|패스워드|암호|세션|쿠키|인증키)"#,
                r#"["']?[ \t]*(?:=>|:=|->|=|:)[ \t]*"#,
                r#"(?:"(?P<dq>[^"\r\n]{1,512})"|'(?P<sq>[^'\r\n]{1,512})'|(?P<raw>[^\s"',;<>&)\]}]{1,512}))"#
            )),
            model,
        }
    }

    /// 형식이 확정적인 자격증명 (규칙 기반, p≈1)
    fn fixed_formats(&self, text: &str, out: &mut Vec<Hit>) {
        for (re, g) in &self.tokens {
            for c in re.captures_iter(text) {
                let m = c.get(*g).unwrap();
                out.push(Hit::new(m.start(), m.end(), InfoType::Session, MaskMode::NonSpace).with_score(0.99));
            }
        }
        // PEM 개인키 본문
        for c in self.pem.captures_iter(text) {
            let m = c.get(2).unwrap();
            out.push(Hit::new(m.start(), m.end(), InfoType::Session, MaskMode::NonSpace).with_score(0.99));
        }
        // scheme://user:password@host
        for c in self.uri_cred.captures_iter(text) {
            let m = c.get(2).unwrap();
            if !dict::is_placeholder(m.as_str()) {
                out.push(Hit::new(m.start(), m.end(), InfoType::Password, MaskMode::NonSpace).with_score(0.98));
            }
        }
        // Authorization 헤더 / Bearer 토큰
        for re in [&self.auth_header, &self.bearer] {
            for c in re.captures_iter(text) {
                let m = c.get(1).unwrap();
                if !dict::is_placeholder(m.as_str()) {
                    out.push(Hit::new(m.start(), m.end(), InfoType::Session, MaskMode::NonSpace).with_score(0.97));
                }
            }
        }
        // AWS Secret Access Key: 40자 + 앞쪽 맥락에 aws/secret
        for m in self.aws_secret.find_iter(text) {
            let s = m.as_str();
            let edge = |c: Option<char>| c.map_or(false, |c| c.is_ascii_alphanumeric() || "/+=".contains(c));
            if edge(crate::common::char_before(text, m.start())) || edge(crate::common::char_after(text, m.end())) {
                continue;
            }
            let classes = s.bytes().any(|b| b.is_ascii_lowercase()) as u8
                + s.bytes().any(|b| b.is_ascii_uppercase()) as u8
                + s.bytes().any(|b| b.is_ascii_digit()) as u8;
            if classes < 3 {
                continue;
            }
            let back = context_window(text, m.start(), m.start(), 300, 0);
            if back.contains("aws") && back.contains("secret") {
                out.push(Hit::new(m.start(), m.end(), InfoType::Session, MaskMode::NonSpace).with_score(0.9));
            }
        }
    }

    /// Cookie / Set-Cookie 헤더
    fn cookie_headers(&self, text: &str, out: &mut Vec<Hit>) {
        for c in self.cookie_header.captures_iter(text) {
            let line = c.get(1).unwrap();
            let mut off = line.start();
            for part in line.as_str().split(';') {
                let part_start = off;
                off += part.len() + 1;
                let Some((name, value)) = part.split_once('=') else { continue };
                let name_t = name.trim();
                if SET_COOKIE_ATTRS.contains(&name_t.to_ascii_lowercase().as_str()) {
                    continue;
                }
                let v = value.trim();
                if v.is_empty() || dict::is_placeholder(v) {
                    continue;
                }
                let vs = part_start + name.len() + 1 + (value.len() - value.trim_start().len());
                let kind = match classify_key(name_t) {
                    Some(k) if k.kind == InfoType::Session => InfoType::Session,
                    _ => InfoType::Cookie,
                };
                out.push(Hit::new(vs, vs + v.len(), kind, MaskMode::NonSpace).with_score(0.95));
            }
        }
    }

    /// 키=값 후보 + 맥락 모델
    fn key_values(&self, text: &str, threshold: f32, out: &mut Vec<Hit>) {
        for c in self.kv.captures_iter(text) {
            let key = c.name("key").unwrap();
            let (val, quoted) = match (c.name("dq"), c.name("sq"), c.name("raw")) {
                (Some(v), _, _) | (_, Some(v), _) => (v, true),
                (_, _, Some(v)) => (v, false),
                _ => continue,
            };
            let kl = key.as_str().to_ascii_lowercase();
            // 헤더 형식 쿠키는 cookie_headers 에서 처리
            if (kl == "cookie" || kl == "set-cookie") && text[key.end()..val.start()].contains(':') {
                continue;
            }
            let Some(kc) = classify_key(key.as_str()) else { continue };
            let v = val.as_str();
            // "Authorization: Bearer xxx" 의 인증 방식 단어는 값이 아님 (토큰은 auth_header 가 처리)
            // PEM 키 블록은 pem 규칙이 본문만 정확히 처리
            if AUTH_SCHEMES.contains(&v.to_ascii_lowercase().as_str()) || v.contains("-----BEGIN ") {
                continue;
            }
            let f = Features {
                key_strength: kc.strength,
                http_ctx: has_http_context(text, key.start(), val.end()),
                needs_http: kc.needs_http,
                negative_key: kc.negative,
                value_len: v.chars().count(),
                entropy: shannon_entropy(v),
                char_classes: char_class_count(v),
                quoted,
                placeholder: dict::is_placeholder(v),
                code_expr: dict::is_code_expr(v, quoted),
                bare_identifier: !quoted && v.len() < 12 && v.chars().all(|c| c.is_ascii_alphabetic() || c == '_'),
            };
            let p = self.model.predict(&f);
            if p >= threshold {
                out.push(Hit::new(val.start(), val.end(), kc.kind, MaskMode::NonSpace).with_score(p));
            }
        }
    }

    pub fn scan(&self, text: &str, threshold: f32, out: &mut Vec<Hit>) {
        if !may_contain_credential(text) {
            return;
        }
        self.fixed_formats(text, out);
        self.cookie_headers(text, out);
        self.key_values(text, threshold, out);
    }
}

/// 민감정보 서명 — 위 정규식들이 일치하려면 **반드시** 들어 있어야 하는 글자열.
/// 키=값 키워드(pass·pw·secret·token·key·auth·credential·sess·sid·cookie·한글 키), 토큰 접두어, PEM, `://`, bearer.
/// (api_key·access-key·private_key 는 모두 `key`, authorization 은 `auth`, Stripe 는 `_live_`/`_test_` 를 포함)
const CREDENTIAL_SIGNATURES: &[&str] = &[
    "pass", "pw", "secret", "token", "key", "auth", "credential", "sess", "sid", "cookie", "bearer", "비밀번호", "패스워드", "암호",
    "세션", "쿠키", "인증키", "akia", "asia", "agpa", "aida", "aroa", "anpa", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_",
    "glpat-", "xox", "eyj", "sk-", "aiza", "_live_", "_test_", "npm_", "-----begin ", "://",
];

/// 서명이 하나도 없으면 민감정보 정규식은 일치할 수 없다 → 통째로 건너뜀 (거짓 음성 없음).
/// 대소문자 무시 정규식은 `ſ`(긴 s)·`K`(켈빈 기호)도 s·k 로 보므로 그 글자가 있으면 그냥 검사한다.
fn may_contain_credential(text: &str) -> bool {
    static AC: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    let ac = AC.get_or_init(|| {
        aho_corasick::AhoCorasick::builder().ascii_case_insensitive(true).build(CREDENTIAL_SIGNATURES).expect("민감정보 서명 오토마타")
    });
    ac.is_match(text) || text.contains(['\u{17F}', '\u{212A}'])
}

impl Default for CredentialScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(t: &str) -> Vec<(String, InfoType)> {
        let s = CredentialScanner::new();
        let mut v = Vec::new();
        s.scan(t, 0.7, &mut v);
        let v = crate::common::resolve_overlaps(v);
        v.iter().map(|h| (t[h.start..h.end].to_string(), h.kind.clone())).collect()
    }

    #[test]
    fn password_kv() {
        assert_eq!(found(r#"password = "rootPass99!""#), vec![("rootPass99!".into(), InfoType::Password)]);
        assert_eq!(found("db_pwd: hunter22"), vec![("hunter22".into(), InfoType::Password)]);
        assert!(found("password = input()").is_empty());
        assert!(found("password_policy = \"strict-mode\"").is_empty());
        assert!(found("password: ********").is_empty());
        assert!(found("compass = north").is_empty());
    }

    #[test]
    fn session_needs_http() {
        assert!(found("session_timeout = 3600").is_empty());
        assert!(found("sessionid=8f3a9c2b77e1").is_empty());
        let r = found("GET /index HTTP/1.1\r\nHost: a.com\r\nX: sessionid=8f3a9c2b77e1d4");
        assert_eq!(r, vec![("8f3a9c2b77e1d4".into(), InfoType::Session)]);
    }

    #[test]
    fn cookie_header() {
        let r = found("Cookie: theme=dark2025; JSESSIONID=A1B2C3D4E5F6; Path=/");
        assert!(r.contains(&("dark2025".into(), InfoType::Cookie)));
        assert!(r.contains(&("A1B2C3D4E5F6".into(), InfoType::Session)));
        assert!(!r.iter().any(|(v, _)| v == "/"));
    }

    /// 서명 거르기: 각 검사 예문은 서명을 갖고, 숫자·일반 문장은 갖지 않는다
    #[test]
    fn signature_prefilter_is_superset() {
        for t in [
            r#"password = "rootPass99!""#, "db_pwd: hunter22", "Cookie: a=b", "AKIAIOSFODNN7EXAMPLE", "glpat-fX28szm91LkWqPoxZ71A",
            "postgresql://postgres:rootPass99!@localhost:5432/db", "Authorization: Bearer abcdefghijklmnop", "비밀번호: 1234abcd",
            "-----BEGIN RSA PRIVATE KEY-----", "eyJhbGciOi.eyJzdWIiOi.abcdefghijk", "sk_live_0123456789abcdef", "Paſſword=x",
        ] {
            assert!(may_contain_credential(t), "{t}");
        }
        for t in [r#"<hp:p id="12" width="3400"/>"#, "회의 일정은 10월 26일입니다", "1234-5678 x 99"] {
            assert!(!may_contain_credential(t), "{t}");
        }
    }

    #[test]
    fn fixed() {
        assert_eq!(found("AKIAIOSFODNN7EXAMPLE").len(), 1);
        assert_eq!(found("glpat-fX28szm91LkWqPoxZ71A").len(), 1);
        let r = found("postgresql://postgres:rootPass99!@localhost:5432/db");
        assert_eq!(r, vec![("rootPass99!".into(), InfoType::Password)]);
    }
}
