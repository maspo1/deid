//! 검사 엔진 프로세스(`deid --engine`) 연결. 읽기 스레드가 출력 줄을 메시지로 바꿔 채널에 넣고,
//! 창에는 `WM_ENGINE` 만 보내 깨운다 (메시지에 포인터를 싣지 않으므로 누수 경로가 없다).

use crate::ipc::{self, Event, Job, Parser};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{channel, Receiver};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub struct Engine {
    pub rx: Receiver<Event>,
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Engine {
    /// 중지 요청 — 엔진이 처리 중인 파일을 마치고 멈춘다
    pub fn stop(&mut self) {
        if let Some(mut s) = self.stdin.take() {
            let _ = s.write_all(b"STOP\n");
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub fn spawn(job: &Job, hwnd: HWND, msg: u32) -> Result<Engine, String> {
    let mut child = ipc::spawn_engine(job).map_err(|e| format!("검사 엔진을 시작할 수 없습니다: {e}"))?;
    let stdout = child.stdout.take().ok_or("엔진 출력 연결 실패")?;
    let stdin = child.stdin.take();
    let (tx, rx) = channel();
    let target = hwnd.0 as usize; // HWND 는 Send 가 아니므로 값으로 넘긴다
    std::thread::spawn(move || {
        let wake = || unsafe {
            let _ = PostMessageW(HWND(target as _), msg, WPARAM(0), LPARAM(0));
        };
        let mut parser = Parser::default();
        let mut finished = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(ev) = parser.feed(&line) {
                finished |= matches!(ev, Event::Done { .. } | Event::Failed(_));
                if tx.send(ev).is_err() {
                    return;
                }
                wake();
            }
        }
        if !finished {
            let _ = tx.send(Event::Failed("검사 엔진이 비정상 종료했습니다 (출력 폴더는 불완전합니다)".into()));
            wake();
        }
    });
    Ok(Engine { rx, child, stdin })
}
