#![cfg(target_os = "windows")]

use std::{path::Path, process::Command};

use anyhow::{anyhow, Context, Result};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};
use screenshots::Screen;
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, BOOL, HWND, POINT, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
        },
        System::{
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
        UI::{
            Input::KeyboardAndMouse::GetCursorPos,
            WindowsAndMessaging::{
                GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                GetWindowThreadProcessId,
            },
        },
    },
};
use winrt_notification::{Duration, Sound, Toast};

use crate::{app, app::CaptureKind, config::Config};

pub fn run() -> Result<()> {
    let config = Config::load()?;
    let manager = GlobalHotKeyManager::new().context("初始化全局快捷键管理器失败")?;

    let fullscreen = register_hotkey(&manager, &config.hotkeys.fullscreen)?;
    let window = register_hotkey(&manager, &config.hotkeys.current_window)?;
    let region = register_hotkey(&manager, &config.hotkeys.square_region)?;

    loop {
        let event = GlobalHotKeyEvent::receiver()
            .recv()
            .context("监听快捷键事件失败")?;

        let kind = if event.id == fullscreen.id() {
            CaptureKind::FullScreen
        } else if event.id == window.id() {
            CaptureKind::CurrentWindow
        } else if event.id == region.id() {
            CaptureKind::SquareRegion
        } else {
            continue;
        };

        if let Err(error) = capture_once(&config, kind) {
            eprintln!("截图失败: {error:#}");
        }
    }
}

fn capture_once(config: &Config, kind: CaptureKind) -> Result<()> {
    let cursor = cursor_position()?;
    let monitor = monitor_from_point(cursor)?;
    let screen = find_screen(monitor.display_rect.left, monitor.display_rect.top)?;
    let hdr = monitor.is_hdr;

    let app_context = active_window_context().ok();
    let base_image = screen.capture().context("采集屏幕像素失败")?;
    let rgba = image_from_screenshot(base_image)?;

    let cropped = match kind {
        CaptureKind::FullScreen => rgba,
        CaptureKind::CurrentWindow => {
            let window = app_context.as_ref().context("未找到当前应用窗口")?;
            crop_to_rect(&rgba, monitor.display_rect, window.rect)?
        }
        CaptureKind::SquareRegion => {
            crop_square_around_cursor(&rgba, monitor.display_rect, cursor, config.region_side)?
        }
    };

    let path = app::persist_capture(
        config,
        DynamicImage::ImageRgba8(cropped),
        hdr,
        kind,
        app_context.as_ref().map(|ctx| ctx.app_name.as_str()),
    )?;

    if config.show_notification {
        show_notification(kind, &path)?;
    }
    if config.auto_open {
        open_image(config, &path)?;
    }

    Ok(())
}

fn register_hotkey(manager: &GlobalHotKeyManager, value: &str) -> Result<HotKey> {
    let hotkey = parse_hotkey(value)?;
    manager
        .register(hotkey)
        .with_context(|| format!("注册快捷键失败: {value}"))?;
    Ok(hotkey)
}

fn parse_hotkey(value: &str) -> Result<HotKey> {
    let mut modifiers = Modifiers::empty();
    let mut key = None;

    for part in value.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "win" | "meta" => modifiers |= Modifiers::META,
            other => key = Some(parse_code(other)?),
        }
    }

    Ok(HotKey::new(
        Some(modifiers),
        key.context("快捷键缺少主按键")?,
    ))
}

fn parse_code(value: &str) -> Result<Code> {
    use Code::*;
    let code = match value {
        "0" => Digit0,
        "1" => Digit1,
        "2" => Digit2,
        "3" => Digit3,
        "4" => Digit4,
        "5" => Digit5,
        "6" => Digit6,
        "7" => Digit7,
        "8" => Digit8,
        "9" => Digit9,
        "a" => KeyA,
        "b" => KeyB,
        "c" => KeyC,
        "d" => KeyD,
        "e" => KeyE,
        "f" => KeyF,
        "g" => KeyG,
        "h" => KeyH,
        "i" => KeyI,
        "j" => KeyJ,
        "k" => KeyK,
        "l" => KeyL,
        "m" => KeyM,
        "n" => KeyN,
        "o" => KeyO,
        "p" => KeyP,
        "q" => KeyQ,
        "r" => KeyR,
        "s" => KeyS,
        "t" => KeyT,
        "u" => KeyU,
        "v" => KeyV,
        "w" => KeyW,
        "x" => KeyX,
        "y" => KeyY,
        "z" => KeyZ,
        "f1" => F1,
        "f2" => F2,
        "f3" => F3,
        "f4" => F4,
        "f5" => F5,
        "f6" => F6,
        "f7" => F7,
        "f8" => F8,
        "f9" => F9,
        "f10" => F10,
        "f11" => F11,
        "f12" => F12,
        _ => return Err(anyhow!("不支持的按键: {value}")),
    };
    Ok(code)
}

#[derive(Debug, Clone, Copy)]
struct MonitorDetails {
    display_rect: Rect,
    is_hdr: bool,
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
        display_rect: Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        is_hdr: false,
    })
}

fn find_screen(left: i32, top: i32) -> Result<Screen> {
    Screen::all()?
        .into_iter()
        .find(|screen| screen.display_info.x == left && screen.display_info.y == top)
        .context("未匹配到鼠标所在显示器")
}

fn active_window_context() -> Result<WindowContext> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0 == 0 {
        return Err(anyhow!("没有可用的前台窗口"));
    }

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .ok()
        .context("读取窗口尺寸失败")?;

    let title = window_title(hwnd).unwrap_or_default();
    let process_name = process_name(hwnd).unwrap_or(title);

    Ok(WindowContext {
        rect: Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        app_name: process_name,
    })
}

fn window_title(hwnd: HWND) -> Result<String> {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0u16; length as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, PWSTR(buffer.as_mut_ptr()), buffer.len() as i32) };
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
    let read = unsafe {
        K32GetModuleBaseNameW(
            process,
            None,
            PWSTR(buffer.as_mut_ptr()),
            buffer.len() as u32,
        )
    };
    let value = String::from_utf16_lossy(&buffer[..read as usize]);
    unsafe {
        let _ = CloseHandle(process);
    }
    Ok(value.trim_end_matches(".exe").to_string())
}

fn image_from_screenshot(
    capture: screenshots::image::RgbaImage,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    ImageBuffer::from_raw(capture.width(), capture.height(), capture.into_raw())
        .context("转换截图缓冲区失败")
}

fn crop_to_rect(
    image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    monitor: Rect,
    window: Rect,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let left = (window.left - monitor.left).max(0) as u32;
    let top = (window.top - monitor.top).max(0) as u32;
    let right = (window.right - monitor.left).min(monitor.width() as i32) as u32;
    let bottom = (window.bottom - monitor.top).min(monitor.height() as i32) as u32;

    if right <= left || bottom <= top {
        return Err(anyhow!("当前应用窗口不在鼠标所在屏幕内"));
    }

    Ok(image.view(left, top, right - left, bottom - top).to_image())
}

fn crop_square_around_cursor(
    image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    monitor: Rect,
    cursor: POINT,
    side: u32,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let side = side.min(image.width()).min(image.height()).max(1);
    let half = side as i32 / 2;
    let max_left = (monitor.width() as i32 - side as i32).max(0);
    let max_top = (monitor.height() as i32 - side as i32).max(0);
    let mut left = cursor.x - monitor.left - half;
    let mut top = cursor.y - monitor.top - half;

    left = left.clamp(0, max_left);
    top = top.clamp(0, max_top);

    Ok(image.view(left as u32, top as u32, side, side).to_image())
}

fn show_notification(kind: CaptureKind, path: &Path) -> Result<()> {
    Toast::new(Toast::POWERSHELL_APP_ID)
        .title("Bite Screenshot")
        .text1(&format!("截图方式：{}", kind.notification_label()))
        .text2(&path.display().to_string())
        .duration(Duration::Short)
        .sound(Some(Sound::Default))
        .show()
        .context("显示系统通知失败")?;
    Ok(())
}

fn open_image(config: &Config, path: &Path) -> Result<()> {
    if let Some(command) = &config.open_command {
        Command::new(command)
            .arg(path)
            .spawn()
            .with_context(|| format!("启动图片查看器失败: {command}"))?;
    } else {
        open::that(path).context("调用系统默认程序打开图片失败")?;
    }
    Ok(())
}
