//! deid — 파일 내부 개인정보/민감정보 비식별화 엔진
//!
//! 모듈 구성
//! * `common`   : 공통 타입·마스킹·인코딩 (모든 모듈이 재사용)
//! * `config`   : 검사 옵션 (사용자 정의 정규식, 임계값 등)
//! * `filetype` : 매직바이트 판별
//! * `exts`     : 확장자 카탈로그(약 230종)와 검사 대상 선택
//! * `rules`    : 탐지 규칙 (1단계 패턴 / 2단계 사전 / 맥락 모델 / 주석)
//! * `handlers` : 형식별 처리기 (텍스트, ZIP 문서, 이미지 메타, 바이너리)
//! * `ocr`      : OS 내장 OCR (Windows 10/11, macOS Vision)
//! * `naming`   : 파일/폴더 이름 비식별화
//! * `report`   : CSV 보고서
//! * `pipeline` : 폴더 단위 실행
//! * `cli`      : 명령줄 실행
//! * `ipc`      : 화면 프로세스 ↔ 엔진 프로세스(`--engine`) 통신
//! * `gui`      : egui GUI (CPU 렌더링) — macOS 화면. 설정 검증·결과 저장·대화상자는 Windows 화면도 재사용
//! * `win32_gui`: Windows 네이티브 Win32 GUI — Windows 화면

pub mod cli;
pub mod common;
pub mod config;
pub mod exts;
pub mod filetype;
pub mod handlers;
pub mod ipc;
pub mod naming;
pub mod ocr;
pub mod pipeline;
pub mod report;
pub mod rules;
pub mod gui;
#[cfg(windows)]
pub mod win32_gui;
