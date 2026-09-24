use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering},
        mpsc, Arc,
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
                Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT},
                D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
                D2D1_BRUSH_PROPERTIES, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
                D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
                BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
                GetMessageW, GetWindowLongPtrW, PostMessageW, PostQuitMessage, RegisterClassExW,
                SetWindowLongPtrW, ShowWindow, TranslateMessage, UnregisterClassW,
                UpdateLayeredWindow, CREATESTRUCTW, GWLP_USERDATA, MSG, SW_HIDE, SW_SHOWNOACTIVATE,
                ULW_ALPHA, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_NCCREATE,
                WM_NCDESTROY, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_POPUP,
            },
        },
    },
};

use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;

const OVERLAY_SIZE: i32 = 256;
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
}

impl Overlay {
    pub fn precreate() -> Result<Self> {
        let visible = Arc::new(AtomicBool::new(false));
        let window = Arc::new(AtomicIsize::new(0));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_visible = Arc::clone(&visible);
        let thread_window = Arc::clone(&window);
        let thread = std::thread::Builder::new()
            .name("zest-overlay".into())
            .spawn(move || overlay_thread(thread_visible, thread_window, ready_tx))
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

        Ok(Self {
            thread: Some(thread),
            window,
            visible,
        })
    }

    pub fn show(&self, _labels: &[String]) -> Result<()> {
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
    ready: mpsc::SyncSender<Result<()>>,
) {
    match run_overlay(visible, window, &ready) {
        Ok(()) => {}
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

fn run_overlay(
    visible: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    ready: &mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }.context("get overlay module handle")?;
    let instance = HINSTANCE(instance.0);
    register_class(instance)?;

    let state = match NativeOverlay::new(Arc::clone(&visible), Arc::clone(&window)) {
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
    memory_dc: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    old_bitmap: HGDIOBJ,
    _render_target: Option<ID2D1DCRenderTarget>,
    _fill_brush: Option<ID2D1SolidColorBrush>,
    _stroke_brush: Option<ID2D1SolidColorBrush>,
}

impl NativeOverlay {
    fn new(visible: Arc<AtomicBool>, window: Arc<AtomicIsize>) -> Result<Self> {
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
            render_target.BeginDraw();
            render_target.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let mut ellipse = D2D1_ELLIPSE::default();
            ellipse.point.X = OVERLAY_SIZE as f32 / 2.0;
            ellipse.point.Y = OVERLAY_SIZE as f32 / 2.0;
            ellipse.radiusX = 96.0;
            ellipse.radiusY = 96.0;
            render_target.FillEllipse(&ellipse, &fill_brush);
            render_target.DrawEllipse(&ellipse, &stroke_brush, 2.0, None);
            if let Err(error) = render_target.EndDraw(None, None) {
                drop(stroke_brush);
                drop(fill_brush);
                drop(render_target);
                release_dc(memory_dc, old_bitmap, bitmap);
                return Err(error).context("finish overlay Direct2D draw");
            }

            Ok(Self {
                hwnd: HWND(std::ptr::null_mut()),
                visible,
                window,
                memory_dc,
                bitmap,
                old_bitmap,
                _render_target: Some(render_target),
                _fill_brush: Some(fill_brush),
                _stroke_brush: Some(stroke_brush),
            })
        }
    }

    fn show(&self) -> Result<()> {
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
        }

        let mut cursor = POINT::default();
        let result = unsafe {
            GetCursorPos(&mut cursor)
                .context("get cursor position")
                .and_then(|_| {
                    let origin = centered_origin(cursor.x, cursor.y, OVERLAY_SIZE);
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
                    UpdateLayeredWindow(
                        self.hwnd,
                        None,
                        Some(&POINT {
                            x: origin.0,
                            y: origin.1,
                        }),
                        Some(&size),
                        Some(self.memory_dc),
                        Some(&source),
                        COLORREF(0),
                        Some(&blend),
                        ULW_ALPHA,
                    )
                    .context("update overlay layered window")
                })
        };
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
                if let Err(error) = state.show() {
                    tracing::warn!("overlay show failed: {error:#}");
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

    #[test]
    fn centers_overlay_on_cursor() {
        assert_eq!(centered_origin(256, 256, OVERLAY_SIZE), (128, 128));
        assert_eq!(centered_origin(-20, -40, OVERLAY_SIZE), (-148, -168));
    }

    #[test]
    fn precreates_and_toggles_native_overlay() {
        let overlay = Overlay::precreate().expect("precreate overlay");
        assert!(!overlay.is_visible());
        overlay.show(&[]).expect("show overlay");
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
    fn external_close_stops_the_overlay_thread() {
        let overlay = Overlay::precreate().expect("precreate overlay");
        unsafe { PostMessageW(Some(overlay.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .expect("post external close");
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(overlay);
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
