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
                    D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE_WINDING, D2D1_PIXEL_FORMAT, D2D_SIZE_F,
                },
                D2D1CreateFactory, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory,
                ID2D1PathGeometry, ID2D1SolidColorBrush,
                ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT,
                D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_BRUSH_PROPERTIES, D2D1_ELLIPSE,
                D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
                HGDIOBJ,
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
                GetMessageW, GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage,
                RegisterClassExW, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage,
                UnregisterClassW, UpdateLayeredWindow, CREATESTRUCTW, GWLP_USERDATA, MSG, SW_HIDE,
                SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
                WM_HOTKEY, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_TIMER,
                WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;

use super::{
    default_gradient, gradient_stops, ramp_color, Click, MenuChoices, PendingOverlay, Rgba,
    RingModel, Sector, SectorEmphasis, ANIMATION_FRAME_MS, OVERLAY_SIZE as OVERLAY_SIZE_F32,
    RING_OUTER_RADIUS,
};
use zest_core::{Gradient, MenuAction, MenuNode};

const OVERLAY_SIZE: i32 = OVERLAY_SIZE_F32 as i32;
const ESCAPE_HOTKEY_ID: i32 = 0x5A57;
const ANIMATION_TIMER_ID: usize = 0x5A58;
const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_HIDE: u32 = WM_APP + 2;
const WM_APP_CLOSE: u32 = WM_APP + 3;
const CLASS_NAME: &str = "ZestOverlayWindow";
const WINDOW_TITLE: &str = "Zest Overlay";

/// Inactive sector: muted dark gray, thin border, per the feel contract.
const INACTIVE_FILL: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.22,
    g: 0.24,
    b: 0.28,
    a: 0.82,
};
const INACTIVE_BORDER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.38,
    g: 0.40,
    b: 0.46,
    a: 0.95,
};

/// How lit each sector was on the last frame the renderer drew. It writes this
/// and the Windows tests read it, which is the only way to see the fade from
/// outside the message loop.
#[cfg(test)]
static LIT_SECTORS: Mutex<Vec<f32>> = Mutex::new(Vec::new());

/// Colours the renderer actually painted across a lit sector, sampled along that
/// sector's radius from the inner edge outwards. Emphasis values only say the
/// fade ran; these say whether a gradient reached the screen at all.
#[cfg(test)]
static RAMP_SAMPLES: Mutex<Vec<(f32, u32)>> = Mutex::new(Vec::new());

pub struct Overlay {
    thread: Option<JoinHandle<()>>,
    window: Arc<AtomicIsize>,
    visible: Arc<AtomicBool>,
    pending: Arc<Mutex<Option<PendingOverlay>>>,
}

impl Overlay {
    /// Pre-create the hidden overlay. Returns the overlay plus the stream of
    /// menu actions the user picks.
    pub fn precreate() -> Result<(Self, MenuChoices)> {
        let visible = Arc::new(AtomicBool::new(false));
        let window = Arc::new(AtomicIsize::new(0));
        let pending = Arc::new(Mutex::new(None));
        let (choice_tx, choice_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_visible = Arc::clone(&visible);
        let thread_window = Arc::clone(&window);
        let thread_pending = Arc::clone(&pending);
        let thread = std::thread::Builder::new()
            .name("zest-overlay".into())
            .spawn(move || {
                overlay_thread(
                    thread_visible,
                    thread_window,
                    thread_pending,
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
                pending,
            },
            MenuChoices {
                receiver: choice_rx,
            },
        ))
    }

    /// Show `menu` at the cursor, lit with the configured sector `gradient`.
    /// Picking a category replaces the ring; picking a leaf closes the overlay
    /// and hands back the action.
    pub fn show(&self, menu: &[MenuNode], gradient: &Gradient) -> Result<()> {
        if menu.is_empty() {
            return Err(anyhow!("overlay menu is empty"));
        }
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow!("overlay model lock poisoned"))?;
            *pending = Some(PendingOverlay {
                ring: RingModel::new(menu.to_vec()),
                gradient: gradient.clone(),
            });
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
    pending: Arc<Mutex<Option<PendingOverlay>>>,
    choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    ready: mpsc::SyncSender<Result<()>>,
) {
    match run_overlay(visible, window, pending, choices, &ready) {
        Ok(()) => {}
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

fn run_overlay(
    visible: Arc<AtomicBool>,
    window: Arc<AtomicIsize>,
    pending: Arc<Mutex<Option<PendingOverlay>>>,
    choices: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    ready: &mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }.context("get overlay module handle")?;
    let instance = HINSTANCE(instance.0);
    register_class(instance)?;

    let state =
        match NativeOverlay::new(Arc::clone(&visible), Arc::clone(&window), pending, choices) {
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
    pending: Arc<Mutex<Option<PendingOverlay>>>,
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
    /// Per-sector hover fades, 0 inactive to 1 lit.
    emphasis: SectorEmphasis,
    /// The configured gradient, one entry per brush stop.
    stops: Vec<Rgba>,
    /// Mean of `stops`: the flat color borders and the no-brush fallback use.
    accent: D2D1_COLOR_F,
    /// The flat ramp bands per sector on the current ring, inner edge outwards.
    band_geometry: Vec<Option<Vec<ID2D1PathGeometry>>>,
    origin: Option<POINT>,
}

impl NativeOverlay {
    fn new(
        visible: Arc<AtomicBool>,
        window: Arc<AtomicIsize>,
        pending: Arc<Mutex<Option<PendingOverlay>>>,
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
                pending,
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
                emphasis: SectorEmphasis::new(0),
                stops: default_gradient(),
                accent: D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.54,
                    b: 0.24,
                    a: 1.0,
                },
                band_geometry: Vec::new(),
                origin: None,
            };
            state.redraw_surface()?;
            Ok(state)
        }
    }

    fn take_pending(&self) -> Option<PendingOverlay> {
        self.pending
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

    /// The path for one flat band of a sector: the slice of the wedge between
    /// `inner` and `outer` along that sector's own radius.
    fn create_band_geometry(
        &self,
        sector: &Sector,
        inner: f32,
        outer: f32,
    ) -> Result<ID2D1PathGeometry> {
        let start_angle = sector.center - sector.width / 2.0;
        let end_angle = start_angle + sector.width;
        let center = OVERLAY_SIZE_F32 / 2.0;
        let on = |radius: f32, angle: f64| {
            let mut point = D2D1_ELLIPSE::default().point;
            point.X = center + (radius * angle.cos() as f32);
            point.Y = center + (radius * angle.sin() as f32);
            point
        };
        let arc = |point, radius: f32, large: bool, clockwise: bool| D2D1_ARC_SEGMENT {
            point,
            size: D2D_SIZE_F {
                width: radius,
                height: radius,
            },
            rotationAngle: 0.0,
            sweepDirection: if clockwise {
                D2D1_SWEEP_DIRECTION_CLOCKWISE
            } else {
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
            },
            arcSize: if large {
                D2D1_ARC_SIZE_LARGE
            } else {
                D2D1_ARC_SIZE_SMALL
            },
        };
        let outer_start = on(outer, start_angle);
        let outer_end = on(outer, end_angle);
        let inner_start = on(inner, start_angle);
        let inner_end = on(inner, end_angle);
        let full = sector.width >= std::f64::consts::TAU - f64::EPSILON;
        let large = sector.width > std::f64::consts::PI;

        let geometry =
            unsafe { self.factory.CreatePathGeometry() }.context("create band geometry")?;
        let sink = unsafe { geometry.Open() }.context("open band geometry sink")?;
        unsafe {
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            if full {
                // A lone sector is the whole ring, so the band is an annulus. Each
                // circle takes two half arcs, because one arc cannot close a full
                // turn, and the two are wound opposite ways so the hole is left.
                let outer_mid = on(outer, start_angle + std::f64::consts::PI);
                let inner_mid = on(inner, start_angle + std::f64::consts::PI);
                sink.BeginFigure(outer_start, D2D1_FIGURE_BEGIN_FILLED);
                sink.AddArc(&arc(outer_mid, outer, false, true));
                sink.AddArc(&arc(outer_start, outer, false, true));
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.BeginFigure(inner_mid, D2D1_FIGURE_BEGIN_FILLED);
                sink.AddArc(&arc(inner_start, inner, false, false));
                sink.AddArc(&arc(inner_mid, inner, false, false));
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            } else {
                // Out along the start edge, round the outside, back down the end
                // edge, and home along the inside. The closing arc has to run
                // anticlockwise or it sweeps the long way and swallows the sector.
                sink.BeginFigure(inner_start, D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLine(outer_start);
                sink.AddArc(&arc(outer_end, outer, large, true));
                sink.AddLine(inner_end);
                sink.AddArc(&arc(inner_start, inner, large, false));
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
        }
        unsafe { sink.Close() }.context("close band geometry sink")?;
        Ok(geometry)
    }

    /// Rebuild the per-sector bands for the ring on screen. A sector whose bands
    /// could not be built falls back to the flat accent.
    /// Build the ramp bands for one sector, unless they are already there.
    /// Only a lit sector ever needs them, and 48 paths per sector built on every
    /// show and every submenu expansion is work nobody asked for.
    fn ensure_band_geometry(&mut self, index: usize) {
        if self.band_geometry.get(index).is_some_and(Option::is_some) {
            return;
        }
        let Some(sector) = self.ring.sectors.get(index) else {
            return;
        };
        let built = (0..RAMP_BANDS)
            .map(|band| {
                let (inner, outer) = band_radii(band);
                self.create_band_geometry(sector, inner, outer)
            })
            .collect::<Result<Vec<_>>>();
        match built {
            Ok(bands) => {
                self.band_geometry[index] = Some(bands);
            }
            Err(error) => {
                tracing::warn!("sector ramp unavailable: {error:#}");
                self.band_geometry[index] = None;
            }
        }
    }

    /// Adopt a freshly handed-over ring, with the gradient the app configured.
    fn set_ring(&mut self, pending: PendingOverlay) {
        self.stops = gradient_stops(pending.gradient.colors());
        self.accent = mean_color(&self.stops);
        self.ring = pending.ring;
        self.relayout_ring();
    }

    /// The ring on screen changed shape: point the brushes at the new sector
    /// angles and start it dark.
    fn relayout_ring(&mut self) {
        self.emphasis.reset(self.ring.sectors.len());
        self.active_sector = None;
        // Bands belong to the shape on screen, so the new ring invalidates them.
        // They are rebuilt per sector the first time that sector lights up.
        self.band_geometry = (0..self.ring.sectors.len()).map(|_| None).collect();
        #[cfg(test)]
        {
            *LIT_SECTORS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Vec::new();
        }
    }

    /// Point the hover at `active`, starting the fade that eases it in.
    fn set_active_sector(&mut self, active: Option<usize>) {
        self.active_sector = active;
        if let Some(index) = active {
            self.ensure_band_geometry(index);
        }
        self.emphasis.set_active(self.ring.sectors.len(), active);
        self.start_animation();
    }

    fn start_animation(&self) {
        unsafe {
            SetTimer(
                Some(self.hwnd),
                ANIMATION_TIMER_ID,
                ANIMATION_FRAME_MS as u32,
                None,
            );
        }
    }

    fn stop_animation(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), ANIMATION_TIMER_ID);
        }
    }

    /// One frame of the hover fade. Reports whether another frame is due.
    fn advance_animation(&mut self) -> bool {
        self.emphasis.advance(ANIMATION_FRAME_MS)
    }

    fn create_sector_geometry(&self, sector: &Sector) -> Result<ID2D1PathGeometry> {
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
                #[cfg(test)]
                {
                    *LIT_SECTORS
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.emphasis.values();
                }
                for (index, sector) in self.ring.sectors.iter().enumerate() {
                    let emphasis = self.emphasis.value(index);
                    let full = sector.width >= std::f64::consts::TAU - f64::EPSILON;
                    let geometry = match full {
                        true => None,
                        false => Some(self.create_sector_geometry(sector)?),
                    };
                    let lit = emphasis > 0.0;
                    let bands = match lit {
                        true => self.band_geometry.get(index).and_then(Option::as_ref),
                        false => None,
                    };
                    let fill = match bands {
                        Some(_) => INACTIVE_FILL,
                        None => mix_color(INACTIVE_FILL, self.accent, emphasis),
                    };
                    let border = mix_color(
                        INACTIVE_BORDER,
                        D2D1_COLOR_F {
                            a: INACTIVE_BORDER.a,
                            ..self.accent
                        },
                        emphasis,
                    );
                    fill_brush.SetColor(&fill);
                    stroke_brush.SetColor(&border);
                    fill_sector(render_target, fill_brush, full, geometry.as_ref(), &outer)?;
                    if let Some(bands) = bands {
                        // Bands are painted opaque so the seam overlap composites
                        // to the same colour whichever band wins it. Two
                        // translucent fills in that overlap would add up, and the
                        // ramp would show radial seams while the fade runs.
                        for (offset, band) in bands.iter().enumerate() {
                            let color =
                                ramp_color(&self.stops, (offset as f32 + 0.5) / RAMP_BANDS as f32);
                            fill_brush.SetColor(&D2D1_COLOR_F {
                                r: color.r,
                                g: color.g,
                                b: color.b,
                                a: 1.0,
                            });
                            fill_sector(render_target, fill_brush, false, Some(band), &outer)?;
                        }
                        // Fade the whole lit ramp back towards the resting fill in
                        // one pass, so the ease costs a single blend rather than
                        // one per band.
                        let mut veil = INACTIVE_FILL;
                        veil.a *= 1.0 - emphasis;
                        fill_brush.SetColor(&veil);
                        fill_sector(render_target, fill_brush, full, geometry.as_ref(), &outer)?;
                    }
                    if full {
                        render_target.DrawEllipse(&outer, stroke_brush, 1.5, None);
                    } else if let Some(geometry) = &geometry {
                        render_target.DrawGeometry(
                            geometry,
                            stroke_brush,
                            1.5,
                            None::<&ID2D1StrokeStyle>,
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
        #[cfg(test)]
        self.record_active_ramp();
        self.present()
    }

    /// Read back what the lit sector painted, walking its radius from just
    /// inside the inner edge to just inside the outer one. Sampling the memory
    /// DC reads the DIB the renderer drew into, so this sees real pixels rather
    /// than the settings that asked for them.
    #[cfg(test)]
    fn record_active_ramp(&self) {
        use windows::Win32::Graphics::Gdi::GetPixel;
        let Some(index) = self.active_sector else { return };
        if self.emphasis.value(index) < 0.9 {
            return;
        }
        let sector = &self.ring.sectors[index];
        let center = OVERLAY_SIZE_F32 / 2.0;
        let (sin, cos) = (sector.center.sin() as f32, sector.center.cos() as f32);
        let span = RING_OUTER_RADIUS - super::RING_INNER_RADIUS;
        let samples = (1..=5)
            .map(|step| {
                // Stay clear of both edges, where the 1.5px borders and the
                // antialiased wedge tips would tint the reading.
                let radius = super::RING_INNER_RADIUS + span * (0.15 + 0.14 * step as f32);
                let x = (center + cos * radius).round() as i32;
                let y = (center + sin * radius).round() as i32;
                let color = unsafe { GetPixel(self.memory_dc, x, y) };
                (radius, color.0)
            })
            .collect();
        *RAMP_SAMPLES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = samples;
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
        self.stop_animation();
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
        self.stop_animation();
        if self.visible.swap(false, Ordering::AcqRel) {
            let _ = unsafe { UnregisterHotKey(Some(self.hwnd), ESCAPE_HOTKEY_ID) };
        }
        self._stroke_brush.take();
        self._fill_brush.take();
        self.band_geometry.clear();
        self._render_target.take();
        unsafe { release_dc(self.memory_dc, self.old_bitmap, self.bitmap) };
    }
}

/// A sector is painted either with the flat muted fill or, as it lights up,
/// with the configured gradient over that fill.
/// Fill one sector: the whole disc when a lone sector is the entire ring, the
/// wedge path otherwise.
fn fill_sector(
    render_target: &ID2D1DCRenderTarget,
    brush: &ID2D1SolidColorBrush,
    full_circle: bool,
    geometry: Option<&ID2D1PathGeometry>,
    outer: &D2D1_ELLIPSE,
) -> Result<()> {
    unsafe {
        if full_circle {
            render_target.FillEllipse(outer, brush);
        } else if let Some(geometry) = geometry {
            render_target.FillGeometry(geometry, brush, None::<&ID2D1Brush>);
        }
    }
    Ok(())
}

/// Spread `stops` evenly across the brush's 0.0..=1.0 range.
/// How many flat bands the active sector is painted in. A linear gradient brush
/// collapses to a single colour on this render target, so the ramp is built out
/// of solid slices instead; 48 is enough that the seams do not read as steps.
const RAMP_BANDS: u32 = 48;

/// Bands are grown slightly past their nominal outer edge so rounding cannot
/// leave a hairline of background between two of them.
const BAND_SEAM: f32 = 0.5;

/// The slice of the ring that band `index` covers, from the inner edge outwards.
fn band_radii(index: u32) -> (f32, f32) {
    let inner_limit = super::RING_INNER_RADIUS;
    let step = (RING_OUTER_RADIUS - inner_limit) / RAMP_BANDS as f32;
    let inner = inner_limit + step * index as f32;
    let outer = if index + 1 == RAMP_BANDS {
        RING_OUTER_RADIUS
    } else {
        (inner + step + BAND_SEAM).min(RING_OUTER_RADIUS)
    };
    (inner, outer)
}


/// One flat color for a gradient: what borders, and the no-brush fallback, use.
fn mean_color(stops: &[Rgba]) -> D2D1_COLOR_F {
    let count = stops.len().max(1) as f32;
    let mean = |channel: fn(&Rgba) -> f32| stops.iter().map(channel).sum::<f32>() / count;
    D2D1_COLOR_F {
        r: mean(|stop| stop.r),
        g: mean(|stop| stop.g),
        b: mean(|stop| stop.b),
        a: 1.0,
    }
}

fn mix_color(from: D2D1_COLOR_F, to: D2D1_COLOR_F, amount: f32) -> D2D1_COLOR_F {
    let mix = |from: f32, to: f32| from + (to - from) * amount;
    D2D1_COLOR_F {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
        a: mix(from.a, to.a),
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
                if let Some(pending) = state.take_pending() {
                    state.set_ring(pending);
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
                    state.set_active_sector(active);
                    if let Err(error) = state.redraw_and_present() {
                        tracing::warn!("overlay redraw failed: {error:#}");
                    }
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_TIMER if wparam.0 == ANIMATION_TIMER_ID => {
            if let Some(state) = state {
                if !state.advance_animation() {
                    state.stop_animation();
                }
                if let Err(error) = state.redraw_and_present() {
                    tracing::warn!("overlay hover animation failed: {error:#}");
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = state {
                if let Some(index) = state.ring.hit_test(point_from_lparam(lparam)) {
                    match state.ring.click(index) {
                        Click::Expanded => {
                            state.relayout_ring();
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
                if state.active_sector.is_some() {
                    state.set_active_sector(None);
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
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetCursorPos};

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
        overlay
            .show(&test_menu(), &Gradient::default())
            .expect("show overlay");
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
        overlay
            .show(&test_menu(), &Gradient::default())
            .expect("show overlay");
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
    fn hovering_a_sector_lights_it_with_the_configured_gradient() {
        let _guard = native_test_guard();
        let (overlay, _choices) = Overlay::precreate().expect("precreate overlay");
        overlay
            .show(
                &test_menu(),
                &Gradient::new(vec![
                    zest_core::HexColor::new(0x00, 0xff, 0x00),
                    zest_core::HexColor::new(0x00, 0x00, 0xff),
                ]),
            )
            .expect("show overlay");
        assert!(wait_for(|| overlay.is_visible()));
        assert!(
            lit_sectors().iter().all(|lit| *lit == 0.0),
            "nothing is lit before the cursor arrives, read {:?}",
            lit_sectors()
        );

        let lit =
            hover_until_settled(&overlay, Target::Ring, &[]).expect("cursor reached the ring");
        assert_eq!(lit.len(), 2, "the ring has two sectors");
        assert_eq!(
            lit.iter().filter(|value| **value > 0.99).count(),
            1,
            "exactly one sector should be lit, read {lit:?}"
        );
        assert!(
            lit.iter().all(|value| *value == 0.0 || *value > 0.99),
            "the unlit sectors should stay dark, read {lit:?}"
        );
    }

    /// Read back the pixels the renderer painted across the lit sector and insist
    /// they climb from the inner edge outwards.
    fn assert_lit_sector_ramps() {
        let samples = RAMP_SAMPLES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(
            samples.len() >= 3,
            "expected the lit sector to be sampled, read {samples:?}"
        );
        // A memory DC is BGRA, so GetPixel hands back 0x00BBGGRR.
        let luma = |color: u32| {
            let (r, g, b) = (color & 0xff, (color >> 8) & 0xff, (color >> 16) & 0xff);
            0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
        };
        for pair in samples.windows(2) {
            assert!(
                luma(pair[1].1) > luma(pair[0].1),
                "every step outward should be brighter, read {samples:?}"
            );
        }
        let inner = luma(samples[0].1);
        let outer = luma(samples[samples.len() - 1].1);
        assert!(
            outer - inner > 24.0,
            "the sector should ramp from near its first stop to near its last, read {samples:?}"
        );
    }

    /// Show `menu`, light a sector, and insist the painted pixels ramp.
    fn ramp_over_menu(menu: &[MenuNode], expected_sectors: usize) {
        let _guard = native_test_guard();
        let (overlay, _choices) = Overlay::precreate().expect("precreate overlay");
        // Black to white, so every step outward has to get brighter.
        overlay
            .show(
                menu,
                &Gradient::new(vec![
                    zest_core::HexColor::new(0x00, 0x00, 0x00),
                    zest_core::HexColor::new(0xff, 0xff, 0xff),
                ]),
            )
            .expect("show overlay");
        assert!(wait_for(|| overlay.is_visible()));

        RAMP_SAMPLES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let lit =
            hover_until_settled(&overlay, Target::Ring, &[]).expect("cursor reached the ring");
        assert_eq!(
            lit.len(),
            expected_sectors,
            "the ring should have {expected_sectors} sectors, read {lit:?}"
        );
        assert_lit_sector_ramps();
    }

    /// A single top-level node is the whole ring, so its one sector takes the
    /// annulus path instead of the wedge path. That is separate code, and the
    /// wedge path was itself wrong until its closing arc was turned the right
    /// way round, so the annulus deserves the same pixel check.
    #[test]
    fn a_whole_ring_sector_paints_a_gradient_rather_than_a_flat_fill() {
        ramp_over_menu(&[leaf_menu("Everything")], 1);
    }

    /// Emphasis values prove the fade ran, not that a gradient reached the
    /// screen. Read the painted pixels across the lit sector: a flat fill gives
    /// one colour at every radius, which is the bug this pins down.
    #[test]
    fn the_lit_sector_paints_a_gradient_rather_than_a_flat_fill() {
        ramp_over_menu(&test_menu(), 2);
    }

    #[test]
    fn moving_off_a_sector_fades_it_back_out() {
        let _guard = native_test_guard();
        let (overlay, _choices) = Overlay::precreate().expect("precreate overlay");
        overlay
            .show(&test_menu(), &Gradient::default())
            .expect("show overlay");
        assert!(wait_for(|| overlay.is_visible()));

        let lit =
            hover_until_settled(&overlay, Target::Ring, &[]).expect("cursor reached the ring");
        let sector = lit
            .iter()
            .position(|value| *value > 0.99)
            .unwrap_or_else(|| panic!("a sector should be lit, read {lit:?}"));
        let dark = hover_until_settled(&overlay, Target::Centre, &lit).expect("cursor stayed put");
        assert!(
            dark.iter().all(|value| *value < 0.01),
            "leaving the ring should fade every sector back out, read {dark:?} after {lit:?}"
        );
        assert!(
            dark[sector] < 0.01,
            "sector {sector} was left lit in {dark:?}"
        );
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

    /// A single top-level node with nothing under it: one sector, full width.
    fn leaf_menu(label: &str) -> MenuNode {
        MenuNode {
            label: label.to_string(),
            action: None,
            children: Vec::new(),
        }
    }

    fn click_at(overlay: &Overlay, x: f32, y: f32) {
        let lparam = LPARAM(((y as i32 as isize) << 16) | (x as i32 as isize));
        unsafe { PostMessageW(Some(overlay.hwnd()), WM_LBUTTONUP, WPARAM(0), lparam) }
            .expect("post left button up");
    }

    /// Where to leave the real cursor.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Target {
        /// A spot out over the ring, lighting a sector up.
        Ring,
        /// The dead centre in the middle of the ring, where nothing is lit.
        Centre,
    }

    /// Leave the real cursor on `target` and wait for the hover to finish
    /// easing, then report how lit every sector was on the last frame.
    /// `previous` is the last snapshot this test read, so waiting for a
    /// *change* is how a second hover is told from a leftover. Real mouse input
    /// is deliberate: it is what drives the fade, and a posted move would fight
    /// the cursor Windows keeps delivering for the real one.
    fn hover_until_settled(
        overlay: &Overlay,
        target: Target,
        previous: &[f32],
    ) -> Option<Vec<f32>> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(overlay.hwnd(), &mut rect) }.ok()?;
        let mut was = POINT::default();
        unsafe { GetCursorPos(&mut was) }.ok()?;
        let centre = POINT {
            x: (rect.left + rect.right) / 2,
            y: (rect.top + rect.bottom) / 2,
        };
        let reached = match target {
            Target::Centre => unsafe { SetCursorPos(centre.x, centre.y) }.is_ok(),
            // The window is centred on the cursor, so at least one side of the
            // ring is on screen unless the desktop is smaller than the ring.
            Target::Ring => [centre.y - 80, centre.y + 80]
                .into_iter()
                .any(|y| unsafe { SetCursorPos(centre.x, y) }.is_ok()),
        };
        if !reached {
            return None;
        }

        for _ in 0..100 {
            let lit = lit_sectors();
            let settled = match target {
                Target::Centre => lit.iter().all(|value| *value < 0.01),
                Target::Ring => lit.iter().any(|value| *value > 0.99),
            };
            if lit != previous && settled {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let lit = lit_sectors();
        unsafe {
            let _ = SetCursorPos(was.x, was.y);
        }
        Some(lit)
    }

    fn lit_sectors() -> Vec<f32> {
        LIT_SECTORS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
