//! exe 에 아이콘·매니페스트·대화상자 리소스(assets/app.rc)를 포함한다.
//! GNU 툴체인은 windres, MSVC 는 rc.exe 를 사용 (embed-resource 가 자동 선택).
//! Windows 가 아닌 대상(macOS·Linux)은 아무것도 하지 않는다 — macOS 아이콘·권한은 assets/macos/make_app.sh.

use std::path::PathBuf;

fn main() {
    for f in ["assets/app.rc", "assets/resource.h", "assets/app.manifest", "assets/icon/app.ico"] {
        println!("cargo:rerun-if-changed={f}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        suppress_mingw_default_manifest();
    }
    let res = embed_resource::compile("assets/app.rc", embed_resource::NONE);
    // Windows 에서 빌드하면 리소스 컴파일러가 반드시 있으므로 실패는 오류.
    // 다른 OS 에서 Windows 대상으로 교차 빌드할 때는 llvm-rc / mingw windres 가 없을 수 있다 → 경고만 (아이콘·매니페스트 없는 exe)
    if cfg!(windows) {
        res.manifest_required().unwrap();
    } else if let Err(e) = res.manifest_required() {
        println!("cargo:warning=Windows 리소스(아이콘·매니페스트)를 넣지 못했습니다: {e}. 배포용 exe 는 Windows 에서 빌드하세요.");
    }
}

/// MinGW gcc 는 링크 시 `default-manifest.o` 를 자동으로 넣는데(언어 0x409), 우리 매니페스트와 충돌해
/// `.rsrc merge failure: multiple non-default manifests` 로 리소스(대화상자·아이콘)가 로더에 보이지 않는다.
/// gcc 는 `-B` 경로를 먼저 찾으므로, 같은 이름의 **빈 COFF 오브젝트**(섹션 0개, 헤더 20바이트)로 대체한다.
fn suppress_mingw_default_manifest() {
    let dir = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("nomanifest");
    std::fs::create_dir_all(&dir).unwrap();
    let machine: u16 = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => 0x014c,
        Ok("aarch64") => 0xaa64,
        _ => 0x8664,
    };
    let mut coff = Vec::with_capacity(20);
    coff.extend(machine.to_le_bytes()); // Machine
    coff.extend([0u8; 18]); // 섹션 0, 시각 0, 심볼표 없음, 선택 헤더 없음, 특성 0
    std::fs::write(dir.join("default-manifest.o"), coff).unwrap();
    println!("cargo:rustc-link-arg-bins=-B{}", dir.display());
}
