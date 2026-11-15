//! 검사 작업 — 엔진 프로세스(`deid --engine`)를 띄우고 출력 줄을 메시지로 바꿔 화면에 보낸다.
//! 화면 프로세스에는 검사 엔진이 올라오지 않으므로 검사 후에도 메모리가 늘어난 채 남지 않는다.

use crate::ipc::{self, Event, Job, Parser};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{channel, Receiver};

pub struct Handle {
    pub rx: Receiver<Event>,
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Handle {
    /// 중지 요청 — 엔진이 처리 중인 파일을 마치고 멈춘다
    pub fn stop(&mut self) {
        if let Some(mut s) = self.stdin.take() {
            let _ = s.write_all(b"STOP\n");
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // 화면이 먼저 닫히면 엔진도 정리 (정상 종료된 경우엔 아무 일 없음)
        drop(self.stdin.take());
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub fn spawn(job: &Job, ctx: egui::Context) -> Result<Handle, String> {
    let mut child = ipc::spawn_engine(job).map_err(|e| format!("검사 엔진을 시작할 수 없습니다: {e}"))?;
    let stdout = child.stdout.take().ok_or("엔진 출력 연결 실패")?;
    let stdin = child.stdin.take();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut parser = Parser::default();
        let mut finished = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(ev) = parser.feed(&line) {
                finished |= matches!(ev, Event::Done { .. } | Event::Failed(_));
                if tx.send(ev).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
        if !finished {
            let _ = tx.send(Event::Failed("검사 엔진이 비정상 종료했습니다 (출력 폴더는 불완전합니다)".into()));
            ctx.request_repaint();
        }
    });
    Ok(Handle { rx, child, stdin })
}
