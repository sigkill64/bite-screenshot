use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};

use crate::{config::Config, naming};

#[derive(Debug, Clone, Copy)]
pub enum CaptureKind {
    FullScreen,
    CurrentWindow,
    SquareRegion,
}

impl CaptureKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullScreen => "fullscreen",
            Self::CurrentWindow => "app",
            Self::SquareRegion => "region",
        }
    }

    pub fn notification_label(self) -> &'static str {
        match self {
            Self::FullScreen => "全屏",
            Self::CurrentWindow => "应用",
            Self::SquareRegion => "区域",
        }
    }
}

#[cfg(target_os = "windows")]
pub fn run() -> Result<()> {
    crate::windows_impl::run()
}

pub fn persist_capture(
    config: &Config,
    image: DynamicImage,
    hdr: bool,
    kind: CaptureKind,
    app_name: Option<&str>,
) -> Result<PathBuf> {
    fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("创建截图输出目录失败: {}", config.output_dir.display()))?;

    let extension = if hdr { ".hdr" } else { ".png" };
    let file_name = naming::build_file_name(config, kind, app_name, extension);
    let path = config.output_dir.join(file_name);

    if hdr {
        image
            .save_with_format(&path, ImageFormat::Hdr)
            .with_context(|| format!("写入 HDR 截图失败: {}", path.display()))?;
    } else {
        image
            .save_with_format(&path, ImageFormat::Png)
            .with_context(|| format!("写入 PNG 截图失败: {}", path.display()))?;
    }

    Ok(path)
}
