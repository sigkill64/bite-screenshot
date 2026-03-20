# Bite Screenshot

一个使用 Rust 编写的 Windows 截图工具原型，支持：

- 监听 3 组全局快捷键：整屏、当前应用窗口、手动框选区域。
- 从 `%APPDATA%/BiteToys/config/screenshot.conf` 读取配置。
- 可配置保存路径、文件名格式、应用名片段格式、快捷键、通知、自动打开及打开命令。
- 鼠标所在屏幕为目标屏幕。
- SDR 截图使用 WGC 保存为无损 PNG，HDR 截图使用 WGC 捕获浮点像素后保存为 AVIF。
- 使用 Windows 通知提示截图模式和文件路径。

> 说明：当前 Windows 截图后端已经切到 WGC。HDR 屏幕会请求 `Rgba16F/scRGB` 帧并通过 `ffmpeg` 编码为 AVIF；编码时按 Windows 这条链路里 `scRGB 1.0 = 80 nit` 的基准白做 PQ 映射，所以运行环境需要 `ffmpeg` 在 `PATH` 中可用。
