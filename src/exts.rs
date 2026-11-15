//! 확장자 카탈로그 (고급 옵션 — 동영상 플레이어의 "지원 확장자 선택" 같은 목록).
//!
//! 실제 형식은 언제나 **매직바이트**로 판별한다. 확장자 선택은 "이 파일을 검사할지"만 정한다.
//! * 체크된 확장자 → 검사
//! * 체크 해제된 확장자 → 원본 복사 (단, 실제 형식이 확장자와 다르면 위장으로 보고 검사)
//! * 목록에 없는 확장자 → `scan_unknown` 이면 검사 (기본)

use crate::filetype::FileKind;
use std::collections::HashSet;
use std::sync::OnceLock;

/// 확장자가 기대하는 실제 형식 계열
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Text,
    Zip,
    Pdf,
    Image,
    Ole,
    Binary,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Text => "텍스트 검사",
            Family::Zip => "ZIP 내부 검사",
            Family::Pdf => "PDF 검사",
            Family::Image => "OCR + 흐림",
            Family::Ole => "문자열 검사",
            Family::Binary => "문자열 검사",
        }
    }

    pub fn of(kind: &FileKind) -> Option<Family> {
        Some(match kind {
            FileKind::Text(_) => Family::Text,
            FileKind::Zip(_) => Family::Zip,
            FileKind::Pdf => Family::Pdf,
            FileKind::Png | FileKind::Jpeg | FileKind::Bmp => Family::Image,
            FileKind::Binary(s) if s.starts_with("OLE") => Family::Ole,
            FileKind::Binary(_) => Family::Binary,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExtInfo {
    pub ext: &'static str,
    pub category: &'static str,
    pub desc: &'static str,
    pub family: Family,
    pub default_on: bool,
}

use Family::*;

/// (분류, 계열, [(확장자, 설명)])
const TABLE: &[(&str, Family, &[(&str, &str)])] = &[
    ("오피스 문서 (ZIP 기반)", Zip, &[
        ("docx", "Word 문서"), ("docm", "Word 매크로 문서"), ("dotx", "Word 서식"), ("dotm", "Word 매크로 서식"),
        ("xlsx", "Excel 통합 문서"), ("xlsm", "Excel 매크로 통합 문서"), ("xltx", "Excel 서식"), ("xltm", "Excel 매크로 서식"),
        ("xlsb", "Excel 바이너리 통합 문서"), ("xlam", "Excel 추가 기능"), ("pptx", "PowerPoint 프레젠테이션"),
        ("pptm", "PowerPoint 매크로 프레젠테이션"), ("potx", "PowerPoint 서식"), ("potm", "PowerPoint 매크로 서식"),
        ("ppsx", "PowerPoint 쇼"), ("ppsm", "PowerPoint 매크로 쇼"), ("ppam", "PowerPoint 추가 기능"), ("sldx", "PowerPoint 슬라이드"),
        ("vsdx", "Visio 드로잉"), ("vsdm", "Visio 매크로 드로잉"), ("vstx", "Visio 서식"), ("hwpx", "한글 문서 (HWPX)"),
        ("odt", "OpenDocument 텍스트"), ("ods", "OpenDocument 스프레드시트"), ("odp", "OpenDocument 프레젠테이션"),
        ("odg", "OpenDocument 그림"), ("odf", "OpenDocument 수식"), ("ott", "OpenDocument 텍스트 서식"),
        ("ots", "OpenDocument 스프레드시트 서식"), ("otp", "OpenDocument 프레젠테이션 서식"), ("epub", "전자책"),
        ("xps", "XPS 문서"), ("oxps", "OpenXPS 문서"), ("pages", "Pages 문서"), ("numbers", "Numbers 스프레드시트"),
        ("3mf", "3D 제조 형식"), ("kmz", "Google Earth 압축 지도"),
    ]),
    ("레거시 문서 (OLE)", Ole, &[
        ("doc", "Word 97-2003 문서"), ("dot", "Word 97-2003 서식"), ("xls", "Excel 97-2003 통합 문서"),
        ("xlt", "Excel 97-2003 서식"), ("ppt", "PowerPoint 97-2003"), ("pot", "PowerPoint 97-2003 서식"),
        ("pps", "PowerPoint 97-2003 쇼"), ("msg", "Outlook 메시지"), ("pub", "Publisher 문서"), ("vsd", "Visio 2003 드로잉"),
        ("wps", "Works 문서"), ("mpp", "Project 파일"), ("hwp", "한글 문서 (HWP, 문자열만)"),
    ]),
    ("PDF", Pdf, &[("pdf", "PDF 문서"), ("ai", "Illustrator (PDF 기반)")]),
    ("텍스트 / 데이터", Text, &[
        ("txt", "텍스트"), ("text", "텍스트"), ("csv", "쉼표 구분 값"), ("tsv", "탭 구분 값"), ("tab", "탭 구분 값"),
        ("json", "JSON"), ("jsonl", "JSON Lines"), ("ndjson", "줄 단위 JSON"), ("json5", "JSON5"), ("geojson", "GeoJSON"),
        ("xml", "XML"), ("xsd", "XML 스키마"), ("xsl", "XSL 스타일시트"), ("xslt", "XSLT 변환"), ("dtd", "문서 형식 정의"),
        ("yaml", "YAML"), ("yml", "YAML"), ("toml", "TOML"), ("ini", "INI 설정"), ("cfg", "설정"), ("conf", "설정"),
        ("config", "설정"), ("properties", "Java 속성"), ("env", "환경 변수"), ("log", "로그"), ("out", "출력 기록"),
        ("err", "오류 기록"), ("trace", "추적 기록"), ("md", "Markdown"), ("markdown", "Markdown"), ("rst", "reStructuredText"),
        ("adoc", "AsciiDoc"), ("tex", "LaTeX"), ("bib", "BibTeX"), ("srt", "자막 (SRT)"), ("smi", "자막 (SAMI)"),
        ("vtt", "자막 (WebVTT)"), ("ass", "자막 (ASS)"), ("nfo", "정보 파일"), ("diff", "차이 파일"), ("patch", "패치"),
        ("sql", "SQL 스크립트"), ("graphql", "GraphQL"), ("proto", "Protocol Buffers"), ("rtf", "서식 있는 텍스트"),
        ("ldif", "LDAP 데이터"), ("har", "HTTP 기록 (HAR)"), ("ps", "PostScript"), ("eps", "Encapsulated PostScript"),
        ("lst", "목록"),
    ]),
    ("소스코드 (주석 검사)", Text, &[
        ("c", "C"), ("h", "C 헤더"), ("cpp", "C++"), ("cc", "C++"), ("cxx", "C++"), ("hpp", "C++ 헤더"), ("hh", "C++ 헤더"),
        ("cs", "C#"), ("java", "Java"), ("kt", "Kotlin"), ("kts", "Kotlin 스크립트"), ("scala", "Scala"), ("groovy", "Groovy"),
        ("gradle", "Gradle"), ("go", "Go"), ("rs", "Rust"), ("swift", "Swift"), ("m", "Objective-C"), ("mm", "Objective-C++"),
        ("py", "Python"), ("pyw", "Python (창)"), ("pyi", "Python 스텁"), ("ipynb", "Jupyter 노트북"), ("rb", "Ruby"),
        ("php", "PHP"), ("phtml", "PHP 템플릿"), ("pl", "Perl"), ("pm", "Perl 모듈"), ("lua", "Lua"), ("r", "R"),
        ("jl", "Julia"), ("dart", "Dart"), ("js", "JavaScript"), ("mjs", "JavaScript 모듈"), ("cjs", "CommonJS"),
        ("jsx", "React JSX"), ("ts", "TypeScript"), ("tsx", "React TSX"), ("vue", "Vue"), ("svelte", "Svelte"),
        ("sh", "셸 스크립트"), ("bash", "Bash"), ("zsh", "Zsh"), ("ps1", "PowerShell"), ("psm1", "PowerShell 모듈"),
        ("psd1", "PowerShell 데이터"), ("bat", "배치 파일"), ("cmd", "명령 스크립트"), ("vbs", "VBScript"), ("vba", "VBA"),
        ("bas", "Visual Basic"), ("asm", "어셈블리"), ("s", "어셈블리"), ("f90", "Fortran"), ("pas", "Pascal"),
        ("ex", "Elixir"), ("exs", "Elixir 스크립트"), ("erl", "Erlang"), ("hs", "Haskell"), ("clj", "Clojure"),
        ("lisp", "Lisp"), ("el", "Emacs Lisp"), ("fs", "F#"), ("sol", "Solidity"), ("tf", "Terraform"), ("hcl", "HCL"),
        ("nix", "Nix"), ("cmake", "CMake"), ("mk", "Makefile"), ("mak", "Makefile"), ("coffee", "CoffeeScript"),
        ("awk", "AWK"), ("tcl", "Tcl"), ("vhd", "VHDL"), ("sv", "SystemVerilog"), ("yar", "YARA 규칙"), ("yara", "YARA 규칙"),
    ]),
    ("웹", Text, &[
        ("html", "HTML"), ("htm", "HTML"), ("xhtml", "XHTML"), ("shtml", "SSI HTML"), ("mhtml", "웹 보관 파일"), ("mht", "웹 보관 파일"),
        ("css", "스타일시트"), ("scss", "SCSS"), ("sass", "Sass"), ("less", "Less"), ("jsp", "JSP"), ("asp", "ASP"),
        ("aspx", "ASP.NET"), ("ascx", "ASP.NET 컨트롤"), ("cshtml", "Razor"), ("erb", "ERB 템플릿"), ("ejs", "EJS 템플릿"),
        ("hbs", "Handlebars"), ("mustache", "Mustache"), ("twig", "Twig"), ("j2", "Jinja2"), ("svg", "SVG 벡터 그림"),
        ("wsdl", "웹 서비스 정의"), ("rss", "RSS"), ("atom", "Atom 피드"),
    ]),
    ("설정 / 인증서 / 접속 정보", Text, &[
        ("reg", "레지스트리"), ("inf", "설치 정보"), ("plist", "속성 목록"), ("desktop", "바로 가기 (Linux)"),
        ("service", "systemd 서비스"), ("htaccess", "Apache 설정"), ("gitconfig", "Git 설정"), ("npmrc", "npm 설정"),
        ("editorconfig", "EditorConfig"), ("lock", "잠금 파일"), ("cnf", "MySQL 설정"), ("xaml", "XAML"),
        ("csproj", "C# 프로젝트"), ("vbproj", "VB 프로젝트"), ("vcxproj", "C++ 프로젝트"), ("sln", "솔루션"), ("props", "MSBuild 속성"),
        ("targets", "MSBuild 대상"), ("manifest", "매니페스트"), ("rc", "리소스 스크립트"), ("def", "모듈 정의"),
        ("ovpn", "OpenVPN 설정"), ("rdp", "원격 데스크톱 연결"), ("pem", "PEM 인증서/키"), ("crt", "인증서"), ("cer", "인증서"),
        ("csr", "인증서 서명 요청"), ("key", "개인 키"), ("asc", "ASCII 서명/키"), ("ppk", "PuTTY 키"), ("pac", "프록시 자동 설정"),
        ("hosts", "hosts 파일"),
    ]),
    ("이메일 / 연락처 / 일정", Text, &[
        ("eml", "이메일 메시지"), ("emlx", "Apple Mail 메시지"), ("mbox", "메일함"), ("mbx", "메일함"),
        ("vcf", "연락처 (vCard)"), ("vcard", "연락처 (vCard)"), ("ics", "일정 (iCalendar)"), ("ical", "일정 (iCalendar)"),
    ]),
    ("이미지 (OCR 검사 + 흐림)", Image, &[
        ("png", "PNG 그림"), ("jpg", "JPEG 그림"), ("jpeg", "JPEG 그림"), ("jpe", "JPEG 그림"), ("jfif", "JPEG 그림"),
        ("pjpeg", "점진적 JPEG"), ("bmp", "비트맵"), ("dib", "비트맵"),
    ]),
    ("압축 / 패키지 (ZIP 기반)", Zip, &[
        ("zip", "ZIP 압축"), ("jar", "Java 아카이브"), ("war", "웹 애플리케이션 아카이브"), ("ear", "엔터프라이즈 아카이브"),
        ("apk", "Android 패키지"), ("aab", "Android 앱 번들"), ("aar", "Android 라이브러리"), ("xapk", "Android 확장 패키지"),
        ("ipa", "iOS 앱"), ("nupkg", "NuGet 패키지"), ("vsix", "Visual Studio 확장"), ("xpi", "Firefox 확장"),
        ("whl", "Python 휠"), ("egg", "Python egg"), ("appx", "Windows 앱 패키지"), ("msix", "Windows 앱 패키지"),
        ("cbz", "만화책 압축"), ("sketch", "Sketch 디자인"),
    ]),
    ("데이터베이스 / 덤프 / 기타 바이너리", Binary, &[
        ("db", "데이터베이스"), ("sqlite", "SQLite"), ("sqlite3", "SQLite 3"), ("db3", "SQLite 3"), ("mdb", "Access 97-2003"),
        ("accdb", "Access 데이터베이스"), ("sdf", "SQL Server Compact"), ("ldb", "잠금/레벨DB"), ("dat", "데이터 파일"),
        ("bin", "바이너리 데이터"), ("bak", "백업"), ("dmp", "메모리 덤프"), ("mdmp", "미니 덤프"), ("pcap", "패킷 캡처"),
        ("pcapng", "패킷 캡처 (NG)"), ("cap", "패킷 캡처"), ("evtx", "Windows 이벤트 로그"), ("etl", "이벤트 추적 로그"),
        ("pst", "Outlook 데이터 파일 (대용량)"), ("ost", "Outlook 오프라인 파일 (대용량)"),
    ]),
];

/// 기본으로 꺼 두는 확장자 (사용자 지시: HWP 불필요 / 수 GB 급 메일 저장소)
const DEFAULT_OFF: &[&str] = &["hwp", "pst", "ost"];

pub fn catalog() -> &'static [ExtInfo] {
    static C: OnceLock<Vec<ExtInfo>> = OnceLock::new();
    C.get_or_init(|| {
        let mut v = Vec::new();
        for (cat, fam, list) in TABLE {
            for (ext, desc) in list.iter() {
                v.push(ExtInfo { ext, category: cat, desc, family: *fam, default_on: !DEFAULT_OFF.contains(ext) });
            }
        }
        v
    })
}

pub fn categories() -> Vec<&'static str> {
    TABLE.iter().map(|t| t.0).collect()
}

pub fn lookup(ext: &str) -> Option<&'static ExtInfo> {
    catalog().iter().find(|e| e.ext == ext)
}

/// 파일명의 확장자 (소문자, 점 제외)
pub fn ext_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_lowercase())
}

#[derive(Clone, Debug)]
pub struct ExtFilter {
    /// 검사할 확장자 (카탈로그 안)
    pub enabled: HashSet<String>,
    /// 목록에 없는 확장자·확장자 없는 파일도 검사
    pub scan_unknown: bool,
    /// 체크 해제됐어도 실제 형식이 확장자와 다르면(위장) 검사
    pub check_disguised: bool,
}

impl Default for ExtFilter {
    fn default() -> Self {
        ExtFilter {
            enabled: catalog().iter().filter(|e| e.default_on).map(|e| e.ext.to_string()).collect(),
            scan_unknown: true,
            check_disguised: true,
        }
    }
}

pub enum Decision {
    Inspect,
    /// 위장 의심으로 검사 (알림 문구)
    Disguised(String),
    /// 검사 제외 (알림 문구)
    Skip(String),
}

impl ExtFilter {
    pub fn decide(&self, name: &str, kind: &FileKind) -> Decision {
        let actual = Family::of(kind);
        let ext = ext_of(name);
        let info = ext.as_deref().and_then(lookup);
        match (info, ext) {
            (Some(i), _) if self.enabled.contains(i.ext) => Decision::Inspect,
            (Some(i), _) => match actual {
                Some(a) if self.check_disguised && a != i.family && a != Family::Binary => {
                    Decision::Disguised(format!(".{} 이지만 실제 형식은 {} → 위장 의심, 검사함", i.ext, kind.label()))
                }
                _ => Decision::Skip(format!(".{} 확장자 검사 제외 - 원본 복사", i.ext)),
            },
            (None, _) if self.scan_unknown => Decision::Inspect,
            (None, ext) => match actual {
                Some(a) if self.check_disguised && a != Family::Binary => Decision::Disguised(format!(
                    "목록에 없는 확장자({})지만 실제 형식은 {} → 검사함",
                    ext.map(|e| format!(".{e}")).unwrap_or_else(|| "없음".into()),
                    kind.label()
                )),
                _ => Decision::Skip("목록에 없는 확장자 - 원본 복사".into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::TextEncoding;

    #[test]
    fn catalog_is_large_and_unique() {
        let c = catalog();
        assert!(c.len() >= 200, "{}", c.len());
        let mut s: Vec<_> = c.iter().map(|e| e.ext).collect();
        s.sort();
        let n = s.len();
        s.dedup();
        assert_eq!(n, s.len(), "중복 확장자");
    }

    #[test]
    fn decisions() {
        let mut f = ExtFilter::default();
        f.enabled.remove("log");
        let text = FileKind::Text(TextEncoding::Utf8);
        assert!(matches!(f.decide("a.log", &text), Decision::Skip(_)));
        assert!(matches!(f.decide("a.log", &FileKind::Zip("DOCX")), Decision::Disguised(_)));
        assert!(matches!(f.decide("a.txt", &text), Decision::Inspect));
        assert!(matches!(f.decide("a.unknownext", &text), Decision::Inspect));
        f.scan_unknown = false;
        assert!(matches!(f.decide("a.unknownext", &FileKind::Binary("BINARY")), Decision::Skip(_)));
        assert!(matches!(f.decide("a.mp3", &FileKind::Zip("DOCX")), Decision::Disguised(_)));
    }
}
