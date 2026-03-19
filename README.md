# Bite Screenshot

一个使用 Rust 编写的 Windows 截图工具原型，支持：

- 监听 3 组全局快捷键：整屏、当前应用窗口、方形区域。
- 从 `%APPDATA%/BiteToys/config/screenshot.conf` 读取配置。
- 可配置保存路径、文件名格式、应用名片段格式、快捷键、通知、自动打开及打开命令。
- 鼠标所在屏幕为目标屏幕。
- SDR 截图保存为无损 PNG，HDR 目标目前保存为 `.hdr` 文件。
- 使用 Windows 通知提示截图模式和文件路径。

> 说明：当前实现优先完成配置、热键、屏幕/前台窗口裁剪、通知与保存流程。真正的 Windows HDR 检测与“点击通知后打开文件”需要在 Windows 机器上继续联调完善。
