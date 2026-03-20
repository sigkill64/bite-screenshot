#![cfg(target_os = "windows")]

use std::{
    ffi::c_void,
    path::Path,
    process::Command,
    sync::{mpsc, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use image::ImageBuffer;
use windows::{
    core::PCWSTR,
    Win32::{
        Devices::Display::{
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO,
            DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
            DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
        },
        Foundation::{
            CloseHandle, BOOL, COLORREF, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HINSTANCE, HWND,
            LPARAM, LRESULT, POINT, RECT, WPARAM,
        },
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
            Gdi::{
                BeginPaint, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, EndPaint,
                FillRect, FrameRect, GetMonitorInfoW, InvalidateRect, MonitorFromPoint,
                SetWindowRgn, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
                RGN_DIFF,
            },
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
        UI::{
            Input::KeyboardAndMouse::{
                RegisterHotKey, ReleaseCapture, SetCapture, SetFocus, UnregisterHotKey,
                HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
                VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_4, VK_5, VK_6, VK_7, VK_8, VK_9, VK_A,
                VK_B, VK_C, VK_D, VK_E, VK_ESCAPE, VK_F, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2,
                VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_G, VK_H, VK_I, VK_J, VK_K,
                VK_L, VK_M, VK_N, VK_O, VK_P, VK_Q, VK_R, VK_S, VK_T, VK_U, VK_V, VK_W, VK_X, VK_Y,
                VK_Z,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
                GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect,
                GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, LoadCursorW,
                RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW,
                ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA,
                HMENU, IDC_CROSS, LWA_ALPHA, MSG, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY,
                WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT,
                WM_RBUTTONDOWN, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
                WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};
use windows_capture::{
    capture::{Context as WgcContext, GraphicsCaptureApiError, GraphicsCaptureApiHandler},
    frame::Frame as WgcFrameHandle,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor as WgcMonitor,
    settings::{
        ColorFormat as WgcColorFormat, CursorCaptureSettings, DirtyRegionSettings,
        DrawBorderSettings, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};
use winrt_notification::{Duration as ToastDuration, Sound, Toast};

use crate::{
    app::{self, CaptureKind, CapturedImage},
    config::Config,
};

static REGION_SELECTOR_CLASS_NAME: OnceLock<Vec<u16>> = OnceLock::new();
static REGION_SELECTOR_CLASS_REGISTERED: OnceLock<std::result::Result<(), String>> =
    OnceLock::new();

pub fn run() -> Result<()> {
    let config = Config::load()?;
    let registrations = [
        register_hotkey(1, &config.hotkeys.fullscreen)?,
        register_hotkey(2, &config.hotkeys.current_window)?,
        register_hotkey(3, &config.hotkeys.square_region)?,
    ];

    let result = hotkey_message_loop(&config, &registrations);

    for registration in registrations {
        unsafe {
            let _ = UnregisterHotKey(HWND::default(), registration.id);
        }
    }

    result
}

fn capture_once(config: &Config, kind: CaptureKind) -> Result<()> {
    let cursor = cursor_position()?;
    let monitor = monitor_from_point(cursor)?;

    let app_context = match kind {
        CaptureKind::CurrentWindow => Some(active_window_context()?),
        _ => None,
    };

    let selected_region = match kind {
        CaptureKind::SquareRegion => match select_region(monitor.display_rect)? {
            Some(rect) => {
                thread::sleep(Duration::from_millis(75));
                Some(rect)
            }
            None => return Ok(()),
        },
        _ => None,
    };

    let capture_rect = match kind {
        CaptureKind::FullScreen => None,
        CaptureKind::CurrentWindow => {
            let window = app_context.as_ref().context("未找到当前应用窗口")?;
            Some(clamp_rect_to_monitor(window.rect, monitor.display_rect)?)
        }
        CaptureKind::SquareRegion => Some(clamp_rect_to_monitor(
            selected_region.context("未选择区域")?,
            monitor.display_rect,
        )?),
    };

    let captured = capture_monitor_image(&monitor, capture_rect)?;
    let captured_is_hdr = captured.is_hdr();

    let path = app::persist_capture(
        config,
        captured,
        kind,
        app_context.as_ref().map(|ctx| ctx.app_name.as_str()),
    )?;

    if config.show_notification {
        show_notification(kind, &path)?;
    }
    if config.auto_open_for(captured_is_hdr) {
        open_image(config, captured_is_hdr, &path)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct HotKeyRegistration {
    id: i32,
    kind: CaptureKind,
}

#[derive(Debug, Clone, Copy)]
struct MonitorDetails {
    handle: HMONITOR,
    display_rect: Rect,
    is_hdr: bool,
}

#[derive(Debug, Clone, Copy)]
struct CaptureRect {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

#[derive(Debug)]
struct WgcCaptureRequest {
    crop: Option<CaptureRect>,
    color_format: WgcColorFormat,
    result_tx: mpsc::Sender<Result<WgcCapturedFrame>>,
}

#[derive(Debug)]
enum WgcCapturedFrame {
    Rgba8 {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Rgba16Float {
        width: u32,
        height: u32,
        pixels: Vec<u16>,
    },
}

struct WgcSingleFrameCapture {
    crop: Option<CaptureRect>,
    color_format: WgcColorFormat,
    result_tx: mpsc::Sender<Result<WgcCapturedFrame>>,
    completed: bool,
}

#[derive(Debug, Clone)]
struct WindowContext {
    rect: Rect,
    app_name: String,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn width(self) -> u32 {
        (self.right - self.left).max(0) as u32
    }

    fn height(self) -> u32 {
        (self.bottom - self.top).max(0) as u32
    }

    fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    fn from_points(start: POINT, end: POINT) -> Self {
        Self {
            left: start.x.min(end.x),
            top: start.y.min(end.y),
            right: start.x.max(end.x),
            bottom: start.y.max(end.y),
        }
    }

    fn inset(self, amount: i32) -> Self {
        Self {
            left: self.left + amount,
            top: self.top + amount,
            right: self.right - amount,
            bottom: self.bottom - amount,
        }
    }

    fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }

    fn to_win_rect(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

#[derive(Debug)]
struct RegionSelectionState {
    monitor_rect: Rect,
    anchor: Option<POINT>,
    current: Option<POINT>,
    result: Option<Rect>,
    finished: bool,
}

impl RegionSelectionState {
    fn new(monitor_rect: Rect) -> Self {
        Self {
            monitor_rect,
            anchor: None,
            current: None,
            result: None,
            finished: false,
        }
    }

    fn client_bounds(&self) -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: self.monitor_rect.width() as i32,
            bottom: self.monitor_rect.height() as i32,
        }
    }

    fn current_client_rect(&self) -> Option<Rect> {
        Some(Rect::from_points(self.anchor?, self.current?))
    }
}

fn hotkey_message_loop(config: &Config, hotkeys: &[HotKeyRegistration]) -> Result<()> {
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) }.0;
        if status == -1 {
            return Err(anyhow!("监听快捷键消息失败"));
        }
        if status == 0 {
            return Ok(());
        }

        if message.message == WM_HOTKEY {
            let hotkey_id = message.wParam.0 as i32;
            if let Some(registration) = hotkeys.iter().find(|hotkey| hotkey.id == hotkey_id) {
                if let Err(error) = capture_once(config, registration.kind) {
                    eprintln!("截图失败: {error:#}");
                }
                continue;
            }
        }

        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn register_hotkey(id: i32, value: &str) -> Result<HotKeyRegistration> {
    let (modifiers, key, kind) = parse_hotkey(value, id)?;
    unsafe { RegisterHotKey(HWND::default(), id, modifiers, key.0 as u32) }
        .with_context(|| format!("注册快捷键失败: {value}"))?;
    Ok(HotKeyRegistration { id, kind })
}

fn parse_hotkey(value: &str, id: i32) -> Result<(HOT_KEY_MODIFIERS, VIRTUAL_KEY, CaptureKind)> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key = None;

    for part in value.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "super" | "win" | "meta" => modifiers |= MOD_WIN,
            other => key = Some(parse_vk(other)?),
        }
    }

    if modifiers.0 == 0 {
        return Err(anyhow!("快捷键至少需要一个修饰键: {value}"));
    }

    let kind = match id {
        1 => CaptureKind::FullScreen,
        2 => CaptureKind::CurrentWindow,
        3 => CaptureKind::SquareRegion,
        _ => return Err(anyhow!("未知快捷键编号: {id}")),
    };

    Ok((
        modifiers | MOD_NOREPEAT,
        key.context("快捷键缺少主按键")?,
        kind,
    ))
}

fn parse_vk(value: &str) -> Result<VIRTUAL_KEY> {
    let code = match value {
        "0" => VK_0,
        "1" => VK_1,
        "2" => VK_2,
        "3" => VK_3,
        "4" => VK_4,
        "5" => VK_5,
        "6" => VK_6,
        "7" => VK_7,
        "8" => VK_8,
        "9" => VK_9,
        "a" => VK_A,
        "b" => VK_B,
        "c" => VK_C,
        "d" => VK_D,
        "e" => VK_E,
        "f" => VK_F,
        "g" => VK_G,
        "h" => VK_H,
        "i" => VK_I,
        "j" => VK_J,
        "k" => VK_K,
        "l" => VK_L,
        "m" => VK_M,
        "n" => VK_N,
        "o" => VK_O,
        "p" => VK_P,
        "q" => VK_Q,
        "r" => VK_R,
        "s" => VK_S,
        "t" => VK_T,
        "u" => VK_U,
        "v" => VK_V,
        "w" => VK_W,
        "x" => VK_X,
        "y" => VK_Y,
        "z" => VK_Z,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        _ => return Err(anyhow!("不支持的按键: {value}")),
    };
    Ok(code)
}

fn cursor_position() -> Result<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }
        .ok()
        .context("读取鼠标位置失败")?;
    Ok(point)
}

fn monitor_from_point(point: POINT) -> Result<MonitorDetails> {
    let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    unsafe { GetMonitorInfoW(handle, &mut info as *mut _ as *mut _) }
        .ok()
        .context("读取显示器信息失败")?;

    let rect = info.monitorInfo.rcMonitor;
    Ok(MonitorDetails {
        handle,
        display_rect: Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        is_hdr: monitor_hdr_enabled(&info).unwrap_or(false),
    })
}

fn capture_monitor_image(monitor: &MonitorDetails, rect: Option<Rect>) -> Result<CapturedImage> {
    let crop = rect
        .map(|value| rect_to_capture_rect(value, monitor.display_rect))
        .transpose()?;
    let color_format = if monitor.is_hdr {
        WgcColorFormat::Rgba16F
    } else {
        WgcColorFormat::Rgba8
    };

    let (result_tx, result_rx) = mpsc::channel::<Result<WgcCapturedFrame>>();
    let failure_tx = result_tx.clone();
    let capture_monitor = WgcMonitor::from_raw_hmonitor(monitor.handle.0);
    let settings = Settings::new(
        capture_monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        color_format,
        WgcCaptureRequest {
            crop,
            color_format,
            result_tx,
        },
    );

    let thread = thread::spawn(move || {
        let result = <WgcSingleFrameCapture as GraphicsCaptureApiHandler>::start(settings)
            .map_err(|error| anyhow!("WGC 截图会话失败: {}", describe_wgc_error(&error)));
        if let Err(error) = &result {
            let _ = failure_tx.send(Err(anyhow!("{error}")));
        }
        result
    });

    let frame = result_rx
        .recv_timeout(Duration::from_secs(5))
        .context("等待 WGC 返回首帧超时")??;

    match thread.join() {
        Ok(result) => result?,
        Err(_) => return Err(anyhow!("WGC 截图线程异常退出")),
    }

    match frame {
        WgcCapturedFrame::Rgba8 {
            width,
            height,
            pixels,
        } => Ok(CapturedImage::SdrRgba8(
            ImageBuffer::from_raw(width, height, pixels).context("转换 WGC RGBA8 缓冲区失败")?,
        )),
        WgcCapturedFrame::Rgba16Float {
            width,
            height,
            pixels,
        } => Ok(CapturedImage::HdrRgba16Float {
            width,
            height,
            pixels,
        }),
    }
}

fn active_window_context() -> Result<WindowContext> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(anyhow!("没有可用的前台窗口"));
    }

    let title = window_title(hwnd).unwrap_or_default();
    let process_name = process_name(hwnd).unwrap_or(title);
    let rect = window_bounds(hwnd)?;

    Ok(WindowContext {
        rect,
        app_name: process_name,
    })
}

fn window_bounds(hwnd: HWND) -> Result<Rect> {
    let mut rect = RECT::default();

    let dwm_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };

    if dwm_result.is_err() {
        unsafe { GetWindowRect(hwnd, &mut rect) }
            .ok()
            .context("读取窗口尺寸失败")?;
    }

    Ok(Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

fn window_title(hwnd: HWND) -> Result<String> {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0u16; length as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    Ok(String::from_utf16_lossy(&buffer[..read as usize]))
}

fn process_name(hwnd: HWND) -> Result<String> {
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 {
        return Err(anyhow!("未找到前台窗口进程"));
    }

    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            BOOL(0),
            process_id,
        )
    }
    .context("打开前台窗口进程失败")?;

    let mut buffer = vec![0u16; 260];
    let read = unsafe { K32GetModuleBaseNameW(process, None, &mut buffer) };
    let value = String::from_utf16_lossy(&buffer[..read as usize]);
    unsafe {
        let _ = CloseHandle(process);
    }
    Ok(value.trim_end_matches(".exe").to_string())
}

fn clamp_rect_to_monitor(target: Rect, monitor: Rect) -> Result<Rect> {
    let rect = Rect {
        left: target.left.max(monitor.left),
        top: target.top.max(monitor.top),
        right: target.right.min(monitor.right),
        bottom: target.bottom.min(monitor.bottom),
    };

    if rect.is_empty() {
        return Err(anyhow!("选中的区域不在鼠标所在屏幕内"));
    }

    Ok(rect)
}

fn rect_to_capture_rect(target: Rect, monitor: Rect) -> Result<CaptureRect> {
    let rect = clamp_rect_to_monitor(target, monitor)?;
    Ok(CaptureRect {
        left: (rect.left - monitor.left) as u32,
        top: (rect.top - monitor.top) as u32,
        right: (rect.right - monitor.left) as u32,
        bottom: (rect.bottom - monitor.top) as u32,
    })
}

fn select_region(monitor: Rect) -> Result<Option<Rect>> {
    ensure_region_selector_class()?;

    let state = Box::new(RegionSelectionState::new(monitor));
    let state_ptr = Box::into_raw(state);

    let hwnd = match create_region_selector_window(monitor, state_ptr) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            unsafe {
                drop(Box::from_raw(state_ptr));
            }
            return Err(error);
        }
    };

    let loop_result = run_region_selection_loop(state_ptr);

    unsafe {
        if !hwnd.0.is_null() {
            let _ = DestroyWindow(hwnd);
        }
    }

    let state = unsafe { Box::from_raw(state_ptr) };
    loop_result?;
    Ok(state.result)
}

fn ensure_region_selector_class() -> Result<()> {
    let result = REGION_SELECTOR_CLASS_REGISTERED.get_or_init(|| {
        let class_name = region_selector_class_name();
        let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }
            .map_err(|error| format!("获取模块句柄失败: {error}"))?;
        let cursor = unsafe { LoadCursorW(HINSTANCE::default(), IDC_CROSS) }
            .map_err(|error| format!("加载十字光标失败: {error}"))?;

        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(region_selector_proc),
            hInstance: hinstance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        let atom = unsafe { RegisterClassW(&window_class) };
        if atom == 0 {
            return Err("注册区域框选窗口失败".to_string());
        }

        Ok(())
    });

    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn create_region_selector_window(
    monitor: Rect,
    state_ptr: *mut RegionSelectionState,
) -> Result<HWND> {
    let class_name = region_selector_class_name();
    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }.context("获取模块句柄失败")?;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WS_POPUP,
            monitor.left,
            monitor.top,
            monitor.width() as i32,
            monitor.height() as i32,
            HWND::default(),
            HMENU::default(),
            hinstance,
            Some(state_ptr.cast::<c_void>() as *const c_void),
        )
    }
    .context("创建区域框选窗口失败")?;

    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 120, LWA_ALPHA)
            .context("设置区域框选窗口透明度失败")?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
        update_region_selector_window(hwnd);
        let _ = InvalidateRect(hwnd, None, BOOL(1));
    }

    Ok(hwnd)
}

fn run_region_selection_loop(state_ptr: *mut RegionSelectionState) -> Result<()> {
    let mut message = MSG::default();
    while unsafe { !(*state_ptr).finished } {
        let status = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) }.0;
        if status == -1 {
            return Err(anyhow!("监听区域框选消息失败"));
        }
        if status == 0 {
            break;
        }

        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn region_selector_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return LRESULT(1);
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = region_selector_state_mut(hwnd) {
                let point =
                    clamp_point_to_rect(client_point_from_lparam(lparam), state.client_bounds());
                state.anchor = Some(point);
                state.current = Some(point);
                let _ = SetCapture(hwnd);
                update_region_selector_window(hwnd);
                let _ = InvalidateRect(hwnd, None, BOOL(1));
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            if let Some(state) = region_selector_state_mut(hwnd) {
                if state.anchor.is_some() {
                    let point = clamp_point_to_rect(
                        client_point_from_lparam(lparam),
                        state.client_bounds(),
                    );
                    state.current = Some(point);
                    update_region_selector_window(hwnd);
                    let _ = InvalidateRect(hwnd, None, BOOL(1));
                }
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            if let Some(state) = region_selector_state_mut(hwnd) {
                let _ = ReleaseCapture();
                if state.anchor.is_some() {
                    let point = clamp_point_to_rect(
                        client_point_from_lparam(lparam),
                        state.client_bounds(),
                    );
                    state.current = Some(point);
                    state.result = state
                        .current_client_rect()
                        .filter(|rect| !rect.is_empty())
                        .map(|rect| rect.offset(state.monitor_rect.left, state.monitor_rect.top));
                    state.finished = true;
                    let _ = DestroyWindow(hwnd);
                }
            }
            return LRESULT(0);
        }
        WM_RBUTTONDOWN => {
            cancel_region_selection(hwnd);
            return LRESULT(0);
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if (wparam.0 as u16) == VK_ESCAPE.0 {
                cancel_region_selection(hwnd);
                return LRESULT(0);
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            paint_region_selector_window(hwnd);
            return LRESULT(0);
        }
        WM_DESTROY => {
            if let Some(state) = region_selector_state_mut(hwnd) {
                state.finished = true;
            }
            return LRESULT(0);
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn region_selector_state_mut(hwnd: HWND) -> Option<&'static mut RegionSelectionState> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RegionSelectionState;
    pointer.as_mut()
}

unsafe fn cancel_region_selection(hwnd: HWND) {
    if let Some(state) = region_selector_state_mut(hwnd) {
        state.result = None;
        state.finished = true;
        let _ = ReleaseCapture();
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn update_region_selector_window(hwnd: HWND) {
    let Some(state) = region_selector_state_mut(hwnd) else {
        return;
    };

    let bounds = state.client_bounds();
    let outer = CreateRectRgn(0, 0, bounds.right, bounds.bottom);
    if outer.0.is_null() {
        return;
    }

    if let Some(selection) = state.current_client_rect().filter(|rect| !rect.is_empty()) {
        let transparent = selection.inset(1);
        if !transparent.is_empty() {
            let inner = CreateRectRgn(
                transparent.left,
                transparent.top,
                transparent.right,
                transparent.bottom,
            );
            if !inner.0.is_null() {
                let _ = CombineRgn(outer, outer, inner, RGN_DIFF);
                let _ = DeleteObject(inner);
            }
        }
    }

    if SetWindowRgn(hwnd, outer, BOOL(1)) == 0 {
        let _ = DeleteObject(outer);
    }
}

unsafe fn paint_region_selector_window(hwnd: HWND) {
    let Some(state) = region_selector_state_mut(hwnd) else {
        return;
    };

    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);

    let background = CreateSolidBrush(COLORREF(0x00202020));
    if !background.0.is_null() {
        let bounds = state.client_bounds().to_win_rect();
        let _ = FillRect(hdc, &bounds, background);
        let _ = DeleteObject(background);
    }

    if let Some(selection) = state.current_client_rect().filter(|rect| !rect.is_empty()) {
        let border = CreateSolidBrush(COLORREF(0x00FFFFFF));
        if !border.0.is_null() {
            let border_rect = selection.to_win_rect();
            let _ = FrameRect(hdc, &border_rect, border);
            let _ = DeleteObject(border);
        }
    }

    let _ = EndPaint(hwnd, &paint);
}

fn client_point_from_lparam(lparam: LPARAM) -> POINT {
    let value = lparam.0 as u32;
    POINT {
        x: (value & 0xFFFF) as i16 as i32,
        y: ((value >> 16) & 0xFFFF) as i16 as i32,
    }
}

fn clamp_point_to_rect(point: POINT, bounds: Rect) -> POINT {
    POINT {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

fn region_selector_class_name() -> &'static Vec<u16> {
    REGION_SELECTOR_CLASS_NAME.get_or_init(|| encode_wide("BiteScreenshotRegionSelector"))
}

fn encode_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl GraphicsCaptureApiHandler for WgcSingleFrameCapture {
    type Flags = WgcCaptureRequest;
    type Error = anyhow::Error;

    fn new(ctx: WgcContext<Self::Flags>) -> Result<Self> {
        Ok(Self {
            crop: ctx.flags.crop,
            color_format: ctx.flags.color_format,
            result_tx: ctx.flags.result_tx,
            completed: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut WgcFrameHandle,
        capture_control: InternalCaptureControl,
    ) -> Result<()> {
        if self.completed {
            return Ok(());
        }

        let captured = extract_wgc_frame(frame, self.crop, self.color_format)?;
        self.completed = true;
        let _ = self.result_tx.send(Ok(captured));
        capture_control.stop();

        Ok(())
    }

    fn on_closed(&mut self) -> Result<()> {
        if !self.completed {
            self.completed = true;
            let _ = self
                .result_tx
                .send(Err(anyhow!("WGC 捕获在拿到首帧之前已经结束")));
        }
        Ok(())
    }
}

fn extract_wgc_frame(
    frame: &mut WgcFrameHandle,
    crop: Option<CaptureRect>,
    color_format: WgcColorFormat,
) -> Result<WgcCapturedFrame> {
    let mut buffer = match crop {
        Some(crop) => frame
            .buffer_crop(crop.left, crop.top, crop.right, crop.bottom)
            .context("从 WGC 帧裁剪目标区域失败")?,
        None => frame.buffer().context("读取 WGC 帧缓冲区失败")?,
    };

    let width = buffer.width();
    let height = buffer.height();
    let bytes = buffer
        .as_nopadding_buffer()
        .context("整理 WGC 帧缓冲区失败")?
        .to_vec();

    match color_format {
        WgcColorFormat::Rgba8 => Ok(WgcCapturedFrame::Rgba8 {
            width,
            height,
            pixels: bytes,
        }),
        WgcColorFormat::Rgba16F => Ok(WgcCapturedFrame::Rgba16Float {
            width,
            height,
            pixels: bytes_to_u16_le(&bytes)?,
        }),
        WgcColorFormat::Bgra8 => {
            let mut rgba = bytes;
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            Ok(WgcCapturedFrame::Rgba8 {
                width,
                height,
                pixels: rgba,
            })
        }
    }
}

fn bytes_to_u16_le(bytes: &[u8]) -> Result<Vec<u16>> {
    if bytes.len() % 2 != 0 {
        return Err(anyhow!("WGC HDR 像素缓冲区长度无效"));
    }

    Ok(bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn describe_wgc_error(error: &GraphicsCaptureApiError<anyhow::Error>) -> String {
    error.to_string()
}

fn monitor_hdr_enabled(monitor_info: &MONITORINFOEXW) -> Result<bool> {
    let device_name = wide_array_to_string(&monitor_info.szDevice);
    if device_name.is_empty() {
        return Ok(false);
    }

    for path in active_display_paths()? {
        let source_name = source_device_name(&path)?;
        if source_name != device_name {
            continue;
        }

        return advanced_color_enabled(&path);
    }

    Ok(false)
}

fn active_display_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        let result = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if result != ERROR_SUCCESS {
            return Err(anyhow!("读取显示配置缓冲区大小失败: {}", result.0));
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![Default::default(); mode_count as usize];
        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };

        if result == ERROR_SUCCESS {
            paths.truncate(path_count as usize);
            return Ok(paths);
        }

        if result != ERROR_INSUFFICIENT_BUFFER || attempts >= 3 {
            return Err(anyhow!("查询显示配置失败: {}", result.0));
        }
    }
}

fn source_device_name(path: &DISPLAYCONFIG_PATH_INFO) -> Result<String> {
    let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };

    let status = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) };
    if status != 0 {
        return Err(anyhow!("读取显示源名称失败: {status}"));
    }

    Ok(wide_array_to_string(&request.viewGdiDeviceName))
}

fn advanced_color_enabled(path: &DISPLAYCONFIG_PATH_INFO) -> Result<bool> {
    let mut target_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };
    let _ = unsafe { DisplayConfigGetDeviceInfo(&mut target_name.header) };

    let mut request = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            size: std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };

    let status = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) };
    if status != 0 {
        return Err(anyhow!("读取高级颜色状态失败: {status}"));
    }

    let flags = unsafe { request.Anonymous.value };
    let advanced_color_supported = (flags & 0b0001) != 0;
    let advanced_color_enabled = (flags & 0b0010) != 0;

    Ok(advanced_color_supported && advanced_color_enabled)
}

fn wide_array_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn show_notification(kind: CaptureKind, path: &Path) -> Result<()> {
    Toast::new(Toast::POWERSHELL_APP_ID)
        .title("Bite Screenshot")
        .text1(&format!("截图方式：{}", kind.notification_label()))
        .text2(&path.display().to_string())
        .duration(ToastDuration::Short)
        .sound(Some(Sound::Default))
        .show()
        .context("显示系统通知失败")?;
    Ok(())
}

fn open_image(config: &Config, hdr: bool, path: &Path) -> Result<()> {
    if let Some(command) = config.open_command_for(hdr) {
        Command::new(command)
            .arg(path)
            .spawn()
            .with_context(|| format!("启动图片查看器失败: {command}"))?;
    } else {
        open::that(path).context("调用系统默认程序打开图片失败")?;
    }
    Ok(())
}
