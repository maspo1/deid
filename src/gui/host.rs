//! 창 호스트 — winit 창 + egui 입력 처리 + CPU 렌더러 + softbuffer 출력.
//!
//! 화면은 바뀔 때만 다시 그린다 (입력, 작업 진행 알림, egui 애니메이션 예약). 가만히 있으면 CPU 0%.

use super::raster::Raster;
use super::App;
use egui::ViewportId;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Icon, Window, WindowId};

/// 다른 스레드(작업 결과 수신)에서 다시 그리기 요청
#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    Repaint(Duration),
}

struct Gfx {
    window: Rc<Window>,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    state: egui_winit::State,
}

struct Host {
    ctx: egui::Context,
    app: App,
    gfx: Option<Gfx>,
    raster: Raster,
    next_repaint: Option<Instant>,
    title: String,
    icon: Option<egui::IconData>,
    error: Option<String>,
}

pub fn run(app: App, title: &str, icon: Option<egui::IconData>) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().map_err(|e| e.to_string())?;
    let proxy: EventLoopProxy<UserEvent> = event_loop.create_proxy();
    let ctx = egui::Context::default();
    ctx.set_request_repaint_callback(move |info| {
        let _ = proxy.send_event(UserEvent::Repaint(info.delay));
    });
    let mut host = Host { ctx, app, gfx: None, raster: Raster::default(), next_repaint: None, title: title.into(), icon, error: None };
    event_loop.run_app(&mut host).map_err(|e| e.to_string())?;
    host.error.map_or(Ok(()), Err)
}

impl Host {
    fn create(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let icon = self.icon.take().and_then(|i| Icon::from_rgba(i.rgba, i.width, i.height).ok());
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(1080.0, 720.0))
            .with_min_inner_size(LogicalSize::new(780.0, 520.0))
            .with_window_icon(icon);
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(2048),
        );
        self.app.attach(&self.ctx, &window);
        window.request_redraw();
        self.gfx = Some(Gfx { window, _context: context, surface, state });
        Ok(())
    }

    fn paint(&mut self, el: &ActiveEventLoop) {
        let Some(g) = self.gfx.as_mut() else { return };
        let raw = g.state.take_egui_input(&g.window);
        let app = &mut self.app;
        let out = self.ctx.run(raw, |ctx| app.update(ctx));
        g.state.handle_platform_output(&g.window, out.platform_output);
        if self.app.quit {
            el.exit();
            return;
        }
        self.raster.set_textures(&out.textures_delta);
        let size = g.window.inner_size();
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
            if g.surface.resize(w, h).is_ok() {
                if let Ok(mut buf) = g.surface.buffer_mut() {
                    let clear = self.ctx.style().visuals.panel_fill;
                    self.raster.render(&mut buf, size.width as usize, size.height as usize, &prims, out.pixels_per_point, clear);
                    let _ = buf.present();
                }
            }
        }
        self.raster.free_textures(&out.textures_delta);
        if let Some(vo) = out.viewport_output.get(&ViewportId::ROOT) {
            self.schedule(vo.repaint_delay);
        }
    }

    fn schedule(&mut self, delay: Duration) {
        if delay.is_zero() {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        } else if let Some(t) = Instant::now().checked_add(delay) {
            self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
        }
    }
}

/// 창 안쪽(클라이언트 영역) 기준 실제 커서 위치 (물리 화소)
#[cfg(windows)]
fn cursor_in_client(w: &Window) -> Option<winit::dpi::PhysicalPosition<f64>> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt) }.ok()?;
    let origin = w.inner_position().ok()?;
    Some(winit::dpi::PhysicalPosition::new((pt.x - origin.x) as f64, (pt.y - origin.y) as f64))
}

#[cfg(not(windows))]
fn cursor_in_client(_: &Window) -> Option<winit::dpi::PhysicalPosition<f64>> {
    None
}

impl ApplicationHandler<UserEvent> for Host {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.create(el) {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::RedrawRequested => return self.paint(el),
            WindowEvent::CloseRequested => {
                if self.app.request_close() {
                    el.exit();
                    return;
                }
            }
            _ => {}
        }
        let Some(g) = self.gfx.as_mut() else { return };
        // 버튼 이벤트에는 좌표가 없다. 원격 데스크톱·자동화 입력처럼 이동 메시지가 늦거나 합쳐지면
        // 이전 위치로 클릭되므로, 실제 커서 위치를 OS 에서 읽어 먼저 알려 준다.
        if matches!(event, WindowEvent::MouseInput { .. }) {
            if let Some(position) = cursor_in_client(&g.window) {
                let moved = WindowEvent::CursorMoved { device_id: winit::event::DeviceId::dummy(), position };
                let _ = g.state.on_window_event(&g.window, &moved);
            }
        }
        let r = g.state.on_window_event(&g.window, &event);
        if r.repaint || matches!(event, WindowEvent::CloseRequested | WindowEvent::Resized(_)) {
            g.window.request_redraw();
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        let UserEvent::Repaint(d) = ev;
        self.schedule(d);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.next_repaint {
            Some(t) if t <= Instant::now() => {
                self.next_repaint = None;
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}
