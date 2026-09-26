use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};

use anyhow::{anyhow, Context, Result};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, POINT, RECT, SIZE, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
                    D2D1_FILL_MODE_WINDING, D2D1_PIXEL_FORMAT, D2D_SIZE_F,
                },
                D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry,
                ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT,
                D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_BRUSH_PROPERTIES, D2D1_ELLIPSE,
                D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                D2D1_SWEEP_DIRECTION_CLOCKWISE,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
                BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::WM_MOUSELEAVE,
            Input::KeyboardAndMouse::{
                RegisterHotKey, TrackMouseEvent, UnregisterHotKey, MOD_NOREPEAT, TME_LEAVE,
                TRACKMOUSEEVENT, VK_ESCAPE,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
                GetMessageW, GetWindowLongPtrW, PostMessageW, PostQuitMessage, RegisterClassExW,
                SetWindowLongPtrW, ShowWindow, TranslateMessage, UnregisterClassW,
                UpdateLayeredWindow, CREATESTRUCTW, GWLP_USERDATA, MSG, SW_HIDE, SW_SHOWNOACTIVATE,
                ULW_ALPHA, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONUP,
                WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;

use super::{Click, MenuChoices, RingModel, OVERLAY_SIZE as OVERLAY_SIZE_F32, RING_OUTER_RADIUS};
use zest_core::{MenuAction, MenuNode};

const OVERLAY_SIZE: i32 = OVERLAY_SIZE_F32 as i32;
const ESCAPE_HOTKEY_ID: i32 = 0x5A57;
const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_HIDE: u32 = WM_APP + 2;
const WM_APP_CLOSE: u32 = WM_APP + 3;
const CLASS_NAME: &str = "ZestOverlayWindow";
const WINDOW_TITLE: &str = "Zest Overlay";

pub struct Overlay {
    thread: Option<JoinHandle<()>>,
    window: Arc<AtomicIsize>,
    visible: Arc<AtomicBool>,
    pending_model: Arc<Mutex<Option<RingModel>>>,
}

impl Overlay {
    /// Pre-create the hidden overlay. Returns the overlay plus the stream of
    /// menu actions the user picks.
    pub fn precreate() -> Result<(Self, MenuChoices)> {
        let visible = Arc::new(AtomicBool::new(false));
        let window = Arc::new(AtomicIsize::new(0));
        let pending_model = Arc::new(Mutex::new(None));
        let (choice_tx, choice_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_visible = Arc::clone(&visible);
        let thread_window = Arc::clone(&window);
        let thread_pending_model = Arc::clone(&pending_model);
        let thread = std::thread::Builder::new()
            .name("zest-overlay".into())
            .spawn(move || {
                overlay_thread(
                    thread_visible,
                    thread_window,
                    thread_pending_model,
                    choice_tx,
                    ready_tx,
                )
            })
            .context("spawn overlay thread")?;

        match ready_rx.recv() {
            Ok(Ok(())) => (),
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(anyhow!("overlay thread exited before becoming ready"));
            }
        };

        Ok((
            Self {
                thread: Some(thread),
                window,
                visible,
                pending_model,
            },
            MenuChoices {
                receiver: choice_rx,
            },
        ))
    }

    /// Show `menu` at the cursor. Picking a category replaces the ring;
    /// picking a leaf closes the overlay and hands back the action.
    pub fn show(&self, menu: &[MenuNode]) -> Result<()> {
        if menu.is_empty() {
            return Err(anyhow!("overlay menu is empty"));
        }
        {
            let mut pending = self
                .pending_model
                .lock()
                .map_err(|_| anyhow!("overlay model lock poisoned"))?;
            *pending = Some(RingModel::new(menu.to_vec()));
        }
        post(self.hwnd(), WM_APP_SHOW)
    }

    pub fn hide(&self) -> Result<()> {
        post(self.hwnd(), WM_APP_HIDE)
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }

    fn hwnd(&self) -> HWND {
        HWND(self.window.load(Ordering::Acquire) as *mut c_void)
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = post(self.hwnd(), WM_APP_CLOSE);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.visible.store(false, Ordering::Release);
    }
}

fn post(hwnd: HWND, message: u32) -> Result<()> {
    if hwnd.0.is_null() {
        return Err(anyhow!("overlay window is not available"));
    }
    unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) }
        .context("post overlay message")
}

fn overlay_thread(
    visible: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    pending_model: Arc<Mutex<Option<RingModel>>>,
    choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    ready: mpsc::SyncSender<Result<()>>,
) {
    match run_overlay(visible, window, pending_model, choices, &ready) {
        Ok(()) => {}
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

fn run_overlay(
    visible: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    pending_model: Arc<Mutex<Option<RingModel>>>,
    choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    ready: &mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }.context("get overlay module handle")?;
    let instance = HINSTANCE(instance.0);
    register_class(instance)?;

    let state = match NativeOverlay::new(
        Arc::clone(&visible),
        Arc::clone(&window),
        pending_model,
        choices,
    ) {
        Ok(state) => state,
        Err(error) => {
            unregister_class(instance);
            return Err(error);
        }
    };
    let state_ptr = Box::into_raw(Box::new(state));
    let class_name = wide(CLASS_NAME);
    let window_title = wide(WINDOW_TITLE);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_title.as_ptr()),
            WS_POPUP,
            0,
            0,
            OVERLAY_SIZE,
            OVERLAY_SIZE,
            None,
            None,
            Some(instance),
            Some(state_ptr.cast()),
        )
    };
    let hwnd = match hwnd {
        Ok(hwnd) => hwnd,
        Err(error) => {
            unsafe { drop(Box::from_raw(state_ptr)) };
            unregister_class(instance);
            return Err(error).context("create overlay window");
        }
    };
    unsafe {
        (*state_ptr).hwnd = hwnd;
    }
    window.store(hwnd.0 as isize, Ordering::Release);

    if ready.send(Ok(())).is_err() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        unregister_class(instance);
        return Err(anyhow!("overlay ready receiver dropped"));
    }
    let result = message_loop();
    if window.load(Ordering::Acquire) == hwnd.0 as isize {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    unregister_class(instance);
    result
}

fn register_class(instance: HINSTANCE) -> Result<()> {
    let class_name = wide(CLASS_NAME);
    let class = windows::Win32::UI::WindowsAndMessaging::WNDCLASSEXW {
        cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 {
        let error = unsafe { windows::Win32::Foundation::GetLastError() };
        if error != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS {
            return Err(windows::core::Error::from_win32())
                .context("register overlay window class");
        }
    }
    Ok(())
}

fn unregister_class(instance: HINSTANCE) {
    let class_name = wide(CLASS_NAME);
    let _ = unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance)) };
}

fn message_loop() -> Result<()> {
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == 0 {
            break;
        }
        if result.0 < 0 {
            return Err(windows::core::Error::from_win32()).context("read overlay message");
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

struct NativeOverlay {
    hwnd: HWND,
    visible: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    pending_model: Arc<Mutex<Option<RingModel>>>,
    choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    memory_dc: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    old_bitmap: HGDIOBJ,
    _render_target: Option<ID2D1DCRenderTarget>,
    _fill_brush: Option<ID2D1SolidColorBrush>,
    _stroke_brush: Option<ID2D1SolidColorBrush>,
    factory: ID2D1Factory,
    ring: RingModel,
    active_sector: Option<usize>,
    origin: Option<POINT>,
}

impl NativeOverlay {
    fn new(
        visible: Arc<AtomicBool>,
        window: Arc<AtomicIsize>,
        pending_model: Arc<Mutex<Option<RingModel>>>,
        choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    ) -> Result<Self> {
        unsafe {
            let memory_dc = CreateCompatibleDC(None);
            if memory_dc.0.is_null() {
                return Err(windows::core::Error::from_win32()).context("create overlay memory DC");
            }

            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: OVERLAY_SIZE,
                    biHeight: -OVERLAY_SIZE,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(
                Some(memory_dc),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            ) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(memory_dc);
                    return Err(error).context("create overlay DIB");
                }
            };
            let old_bitmap = SelectObject(memory_dc, bitmap.into());
            if old_bitmap.0.is_null() {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory_dc);
                return Err(windows::core::Error::from_win32()).context("select overlay DIB");
            }

            let factory =
                match D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
                    Ok(factory) => factory,
                    Err(error) => {
                        release_dc(memory_dc, old_bitmap, bitmap);
                        return Err(error).context("create Direct2D factory");
                    }
                };
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode:
                        windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                ..Default::default()
            };
            let render_target = match factory.CreateDCRenderTarget(&properties) {
                Ok(target) => target,
                Err(error) => {
                    release_dc(memory_dc, old_bitmap, bitmap);
                    return Err(error).context("create Direct2D render target");
                }
            };
            let render_rect = RECT {
                left: 0,
                top: 0,
                right: OVERLAY_SIZE,
                bottom: OVERLAY_SIZE,
            };
            if let Err(error) = render_target.BindDC(memory_dc, &render_rect) {
                drop(render_target);
                release_dc(memory_dc, old_bitmap, bitmap);
                return Err(error).context("bind Direct2D render target");
            }

            let fill_color = D2D1_COLOR_F {
                r: 0.10,
                g: 0.10,
                b: 0.10,
                a: 0.94,
            };
            let stroke_color = D2D1_COLOR_F {
                r: 1.0,
                g: 0.54,
                b: 0.24,
                a: 1.0,
            };
            let brush_properties = D2D1_BRUSH_PROPERTIES {
                opacity: 1.0,
                ..Default::default()
            };
            let fill_brush =
                match render_target.CreateSolidColorBrush(&fill_color, Some(&brush_properties)) {
                    Ok(brush) => brush,
                    Err(error) => {
                        drop(render_target);
                        release_dc(memory_dc, old_bitmap, bitmap);
                        return Err(error).context("create overlay fill brush");
                    }
                };
            let stroke_brush =
                match render_target.CreateSolidColorBrush(&stroke_color, Some(&brush_properties)) {
                    Ok(brush) => brush,
                    Err(error) => {
                        drop(fill_brush);
                        drop(render_target);
                        release_dc(memory_dc, old_bitmap, bitmap);
                        return Err(error).context("create overlay stroke brush");
                    }
                };
            let state = Self {
                hwnd: HWND(std::ptr::null_mut()),
                visible,
                window,
                pending_model,
                choices,
                memory_dc,
                bitmap,
                old_bitmap,
                factory,
                _render_target: Some(render_target),
                _fill_brush: Some(fill_brush),
                _stroke_brush: Some(stroke_brush),
                ring: RingModel::default(),
                active_sector: None,
                origin: None,
            };
            state.redraw_surface()?;
            Ok(state)
        }
    }

    fn take_pending_model(&self) -> Option<RingModel> {
        self.pending_model
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }

    fn track_mouse_leave(&self) {
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        let _ = unsafe { TrackMouseEvent(&mut tracking) };
    }

    fn create_sector_geometry(&self, sector: &super::Sector) -> Result<ID2D1PathGeometry> {
        let start_angle = sector.center - sector.width / 2.0;
        let end_angle = start_angle + sector.width;
        let center = OVERLAY_SIZE_F32 / 2.0;
        let mut center_point = D2D1_ELLIPSE::default().point;
        center_point.X = center;
        center_point.Y = center;
        let mut start_point = D2D1_ELLIPSE::default().point;
        start_point.X = center + (RING_OUTER_RADIUS * start_angle.cos() as f32);
        start_point.Y = center + (RING_OUTER_RADIUS * start_angle.sin() as f32);
        let mut end_point = D2D1_ELLIPSE::default().point;
        end_point.X = center + (RING_OUTER_RADIUS * end_angle.cos() as f32);
        end_point.Y = center + (RING_OUTER_RADIUS * end_angle.sin() as f32);

        let geometry =
            unsafe { self.factory.CreatePathGeometry() }.context("create sector geometry")?;
        let sink = unsafe { geometry.Open() }.context("open sector geometry sink")?;
        unsafe {
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            sink.BeginFigure(center_point, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(start_point);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: end_point,
                size: D2D_SIZE_F {
                    width: RING_OUTER_RADIUS,
                    height: RING_OUTER_RADIUS,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if sector.width > std::f64::consts::PI {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        unsafe { sink.Close() }.context("close sector geometry sink")?;
        Ok(geometry)
    }

    fn redraw_surface(&self) -> Result<()> {
        let render_target = self
            ._render_target
            .as_ref()
            .ok_or_else(|| anyhow!("overlay render target unavailable"))?;
        let fill_brush = self
            ._fill_brush
            .as_ref()
            .ok_or_else(|| anyhow!("overlay fill brush unavailable"))?;
        let stroke_brush = self
            ._stroke_brush
            .as_ref()
            .ok_or_else(|| anyhow!("overlay stroke brush unavailable"))?;
        let center = OVERLAY_SIZE_F32 / 2.0;
        let mut outer = D2D1_ELLIPSE::default();
        outer.point.X = center;
        outer.point.Y = center;
        outer.radiusX = RING_OUTER_RADIUS;
        outer.radiusY = RING_OUTER_RADIUS;

        unsafe {
            render_target.BeginDraw();
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let draw_result = (|| -> Result<()> {
                render_target.Clear(Some(&D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }));
                for (index, sector) in self.ring.sectors.iter().enumerate() {
                    let active = self.active_sector == Some(index);
                    let fill = if active {
                        D2D1_COLOR_F {
                            r: 1.0,
                            g: 0.54,
                            b: 0.24,
                            a: 0.98,
                        }
                    } else {
                        D2D1_COLOR_F {
                            r: 0.22,
                            g: 0.24,
                            b: 0.28,
                            a: 0.82,
                        }
                    };
                    fill_brush.SetColor(&fill);
                    stroke_brush.SetColor(&D2D1_COLOR_F {
                        r: fill.r,
                        g: fill.g,
                        b: fill.b,
                        a: 0.95,
                    });
                    if sector.width >= std::f64::consts::TAU - f64::EPSILON {
                        render_target.FillEllipse(&outer, fill_brush);
                        render_target.DrawEllipse(&outer, stroke_brush, 1.5, None);
                    } else {
                        let geometry = self.create_sector_geometry(sector)?;
                        render_target.FillGeometry(
                            &geometry,
                            fill_brush,
                            None::<&windows::Win32::Graphics::Direct2D::ID2D1Brush>,
                        );
                        render_target.DrawGeometry(
                            &geometry,
                            stroke_brush,
                            1.5,
                            None::<&windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle>,
                        );
                    }
                }

                let mut center_ellipse = D2D1_ELLIPSE::default();
                center_ellipse.point.X = center;
                center_ellipse.point.Y = center;
                center_ellipse.radiusX = super::RING_INNER_RADIUS;
                center_ellipse.radiusY = super::RING_INNER_RADIUS;
                fill_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.10,
                    g: 0.10,
                    b: 0.10,
                    a: 0.96,
                });
                stroke_brush.SetColor(&D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.54,
                    b: 0.24,
                    a: 1.0,
                });
                render_target.FillEllipse(&center_ellipse, fill_brush);
                render_target.DrawEllipse(&center_ellipse, stroke_brush, 1.5, None);
                Ok(())
            })();
            let end_result = render_target
                .EndDraw(None, None)
                .context("finish overlay Direct2D draw");
            draw_result.and(end_result)
        }
    }

    fn present(&self) -> Result<()> {
        let origin = self
            .origin
            .ok_or_else(|| anyhow!("overlay origin unavailable"))?;
        let size = SIZE {
            cx: OVERLAY_SIZE,
            cy: OVERLAY_SIZE,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&origin),
                Some(&size),
                Some(self.memory_dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .context("update overlay layered window")
    }

    fn redraw_and_present(&self) -> Result<()> {
        self.redraw_surface()?;
        self.present()
    }

    fn show(&mut self) -> Result<()> {
        let was_visible = self.visible.load(Ordering::Acquire);
        if !was_visible {
            unsafe {
                RegisterHotKey(
                    Some(self.hwnd),
                    ESCAPE_HOTKEY_ID,
                    MOD_NOREPEAT,
                    VK_ESCAPE.0.into(),
                )
            }
            .context("register overlay Escape hotkey")?;
            self.track_mouse_leave();
        }

        let result = (|| -> Result<()> {
            let mut cursor = POINT::default();
            unsafe { GetCursorPos(&mut cursor) }.context("get cursor position")?;
            let (x, y) = centered_origin(cursor.x, cursor.y, OVERLAY_SIZE);
            self.origin = Some(POINT { x, y });
            self.redraw_and_present()
        })();
        if let Err(error) = result {
            if !was_visible {
                let _ = unsafe { UnregisterHotKey(Some(self.hwnd), ESCAPE_HOTKEY_ID) };
            }
            return Err(error);
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.visible.store(true, Ordering::Release);
        Ok(())
    }

    fn hide(&self) -> Result<()> {
        if self.visible.swap(false, Ordering::AcqRel) {
            let _ = unsafe { UnregisterHotKey(Some(self.hwnd), ESCAPE_HOTKEY_ID) };
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        Ok(())
    }
}

impl Drop for NativeOverlay {
    fn drop(&mut self) {
        if self.visible.swap(false, Ordering::AcqRel) {
            let _ = unsafe { UnregisterHotKey(Some(self.hwnd), ESCAPE_HOTKEY_ID) };
        }
        self._stroke_brush.take();
        self._fill_brush.take();
        self._render_target.take();
        unsafe { release_dc(self.memory_dc, self.old_bitmap, self.bitmap) };
    }
}

unsafe fn release_dc(
    memory_dc: windows::Win32::Graphics::Gdi::HDC,
    old_bitmap: HGDIOBJ,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
) {
    let _ = SelectObject(memory_dc, old_bitmap);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(memory_dc);
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn centered_origin(cursor_x: i32, cursor_y: i32, size: i32) -> (i32, i32) {
    (cursor_x - size / 2, cursor_y - size / 2)
}

fn point_from_lparam(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 as i16) as f32;
    let y = ((lparam.0 >> 16) as i16) as f32;
    (x, y)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }

    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut NativeOverlay };
    let state = (!state_ptr.is_null()).then_some(&mut *state_ptr);
    match message {
        WM_HOTKEY if wparam.0 as i32 == ESCAPE_HOTKEY_ID => {
            if let Some(state) = state {
                let _ = state.hide();
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_APP_SHOW => {
            if let Some(state) = state {
                if let Some(model) = state.take_pending_model() {
                    state.ring = model;
                    state.active_sector = None;
                }
                if let Err(error) = state.show() {
                    tracing::warn!("overlay show failed: {error:#}");
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state {
                state.track_mouse_leave();
                let active = state.ring.hit_test(point_from_lparam(lparam));
                if active != state.active_sector {
                    state.active_sector = active;
                    if let Err(error) = state.redraw_and_present() {
                        tracing::warn!("overlay redraw failed: {error:#}");
                    }
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = state {
                if let Some(index) = state.ring.hit_test(point_from_lparam(lparam)) {
                    match state.ring.click(index) {
                        Click::Expanded => {
                            state.active_sector = None;
                            if let Err(error) = state.redraw_and_present() {
                                tracing::warn!("overlay ring expansion failed: {error:#}");
                            }
                        }
                        Click::Action(action) => {
                            if let Err(error) = state.choices.send(action) {
                                tracing::warn!("menu choice channel closed: {error:#}");
                            }
                            if let Err(error) = state.hide() {
                                tracing::warn!("overlay hide failed: {error:#}");
                            }
                        }
                        Click::Miss => {}
                    }
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(state) = state {
                if state.active_sector.take().is_some() {
                    if let Err(error) = state.redraw_and_present() {
                        tracing::warn!("overlay hover clear failed: {error:#}");
                    }
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_APP_HIDE => {
            if let Some(state) = state {
                let _ = state.hide();
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_APP_CLOSE | WM_CLOSE | WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_ERASEBKGND => windows::Win32::Foundation::LRESULT(1),
        WM_NCDESTROY => {
            if let Some(state) = state {
                state.window.store(0, Ordering::Release);
            }
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            if !state_ptr.is_null() {
                unsafe { drop(Box::from_raw(state_ptr)) };
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NATIVE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn native_test_guard() -> std::sync::MutexGuard<'static, ()> {
        NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn centers_overlay_on_cursor() {
        assert_eq!(centered_origin(256, 256, OVERLAY_SIZE), (128, 128));
        assert_eq!(centered_origin(-20, -40, OVERLAY_SIZE), (-148, -168));
    }

    #[test]
    fn decodes_mouse_coordinates() {
        let lparam = LPARAM(((50_i32 as isize) << 16) | 100);
        assert_eq!(point_from_lparam(lparam), (100.0, 50.0));
    }

    #[test]
    fn precreates_and_toggles_native_overlay() {
        let _guard = native_test_guard();
        let (overlay, _choices) = Overlay::precreate().expect("precreate overlay");
        assert!(!overlay.is_visible());
        overlay.show(&test_menu()).expect("show overlay");
        assert!(wait_for(|| overlay.is_visible()));
        unsafe {
            PostMessageW(
                Some(overlay.hwnd()),
                WM_HOTKEY,
                WPARAM(ESCAPE_HOTKEY_ID as usize),
                LPARAM(0),
            )
        }
        .expect("post Escape hotkey");
        assert!(wait_for(|| !overlay.is_visible()));
    }

    #[test]
    fn clicking_a_leaf_reports_its_action() {
        let _guard = native_test_guard();
        let (overlay, mut choices) = Overlay::precreate().expect("precreate overlay");
        overlay.show(&test_menu()).expect("show overlay");
        assert!(wait_for(|| overlay.is_visible()));

        // Ring 1: "Convert" sits at the top, so a click straight up expands it.
        click_at(&overlay, 128.0, 28.0);
        // Ring 2: first Convert target is "png", again at the top.
        click_at(&overlay, 128.0, 28.0);

        let action = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("tokio runtime")
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), choices.recv()).await
            });
        assert_eq!(
            action.expect("menu choice within timeout"),
            Some(MenuAction::Convert {
                ext: "png".to_string()
            })
        );
        assert!(wait_for(|| !overlay.is_visible()));
    }

    #[test]
    fn external_close_stops_the_overlay_thread() {
        let _guard = native_test_guard();
        let (overlay, _choices) = Overlay::precreate().expect("precreate overlay");
        unsafe { PostMessageW(Some(overlay.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .expect("post external close");
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(overlay);
    }

    fn test_menu() -> Vec<MenuNode> {
        vec![
            MenuNode {
                label: "Convert".to_string(),
                action: None,
                children: vec![MenuNode {
                    label: "png".to_string(),
                    action: Some(MenuAction::Convert {
                        ext: "png".to_string(),
                    }),
                    children: Vec::new(),
                }],
            },
            MenuNode {
                label: "Archive".to_string(),
                action: None,
                children: Vec::new(),
            },
        ]
    }

    fn click_at(overlay: &Overlay, x: f32, y: f32) {
        let lparam = LPARAM(((y as i32 as isize) << 16) | (x as i32 as isize));
        unsafe { PostMessageW(Some(overlay.hwnd()), WM_LBUTTONUP, WPARAM(0), lparam) }
            .expect("post left button up");
    }

    fn wait_for(condition: impl Fn() -> bool) -> bool {
        for _ in 0..100 {
            if condition() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }
}
