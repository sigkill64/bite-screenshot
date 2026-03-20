#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod naming;

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    app::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("bite-screenshot 目前仅支持 Windows 运行。请在 Windows 上构建和执行此程序。");
}
