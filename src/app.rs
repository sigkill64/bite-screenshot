use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use image::{ImageBuffer, ImageFormat, Rgba};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::{config::Config, naming};

const SCRGB_REFERENCE_WHITE_NITS: u32 = 80;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

pub enum CapturedImage {
    SdrRgba8(ImageBuffer<Rgba<u8>, Vec<u8>>),
    HdrRgba16Float {
        width: u32,
        height: u32,
        pixels: Vec<u16>,
    },
}

impl CapturedImage {
    pub fn is_hdr(&self) -> bool {
        matches!(self, Self::HdrRgba16Float { .. })
    }
}

pub fn persist_capture(
    config: &Config,
    image: CapturedImage,
    kind: CaptureKind,
    app_name: Option<&str>,
) -> Result<PathBuf> {
    fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("创建截图输出目录失败: {}", config.output_dir.display()))?;

    let extension = if image.is_hdr() { ".avif" } else { ".png" };
    let file_name = naming::build_file_name(config, kind, app_name, extension);
    let path = config.output_dir.join(file_name);

    match image {
        CapturedImage::SdrRgba8(image) => {
            image
                .save_with_format(&path, ImageFormat::Png)
                .with_context(|| format!("写入 PNG 截图失败: {}", path.display()))?;
        }
        CapturedImage::HdrRgba16Float {
            width,
            height,
            pixels,
        } => {
            save_hdr_avif(&path, width, height, &pixels)?;
        }
    }

    Ok(path)
}

fn save_hdr_avif(path: &Path, width: u32, height: u32, pixels: &[u16]) -> Result<()> {
    let input = rgba16f_sc_rgb_to_gbrpf32le(pixels)?;
    let size = format!("{width}x{height}");
    let filter = format!(
        "zscale=pin=bt709:tin=linear:min=gbr:rin=full:p=bt2020:t=smpte2084:m=bt2020nc:r=full:npl={SCRGB_REFERENCE_WHITE_NITS},format=yuv444p10le"
    );

    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "gbrpf32le",
        "-s",
        &size,
        "-i",
        "pipe:0",
        "-vf",
        &filter,
        "-frames:v",
        "1",
        "-c:v",
        "libaom-av1",
        "-still-picture",
        "1",
        "-cpu-used",
        "6",
        "-crf",
        "12",
        "-pix_fmt",
        "yuv444p10le",
        "-color_primaries",
        "bt2020",
        "-color_trc",
        "smpte2084",
        "-colorspace",
        "bt2020nc",
        "-f",
        "avif",
        path.to_string_lossy().as_ref(),
    ]);
    configure_background_command(&mut command);

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("启动 ffmpeg 失败，无法写入 HDR AVIF")?;

    {
        let stdin = child.stdin.as_mut().context("无法获取 ffmpeg 标准输入")?;
        stdin
            .write_all(&input)
            .context("向 ffmpeg 写入 HDR 像素失败")?;
    }

    let output = child
        .wait_with_output()
        .context("等待 ffmpeg 写入 HDR AVIF 失败")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(anyhow!("ffmpeg 写入 HDR AVIF 失败"));
        }
        return Err(anyhow!("ffmpeg 写入 HDR AVIF 失败: {stderr}"));
    }

    Ok(())
}

fn rgba16f_sc_rgb_to_gbrpf32le(pixels: &[u16]) -> Result<Vec<u8>> {
    if pixels.len() % 4 != 0 {
        return Err(anyhow!("HDR 像素缓冲区长度无效"));
    }

    let pixel_count = pixels.len() / 4;
    let plane_len = pixel_count * std::mem::size_of::<f32>();
    let mut output = vec![0u8; plane_len * 3];

    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        let r = half_to_f32(pixel[0]);
        let g = half_to_f32(pixel[1]);
        let b = half_to_f32(pixel[2]);

        write_f32(&mut output[index * 4..index * 4 + 4], g);
        write_f32(
            &mut output[plane_len + index * 4..plane_len + index * 4 + 4],
            b,
        );
        write_f32(
            &mut output[plane_len * 2 + index * 4..plane_len * 2 + index * 4 + 4],
            r,
        );
    }

    Ok(output)
}

fn write_f32(buffer: &mut [u8], value: f32) {
    buffer.copy_from_slice(&value.to_le_bytes());
}

fn configure_background_command(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exponent = ((bits >> 10) & 0x1F) as u32;
    let mantissa = (bits & 0x03FF) as u32;

    let value = if exponent == 0 {
        if mantissa == 0 {
            sign
        } else {
            let mut exponent = -14i32;
            let mut mantissa = mantissa;
            while (mantissa & 0x0400) == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            mantissa &= 0x03FF;
            sign | (((exponent + 127) as u32) << 23) | (mantissa << 13)
        }
    } else if exponent == 0x1F {
        sign | 0x7F80_0000 | (mantissa << 13)
    } else {
        sign | ((exponent + 112) << 23) | (mantissa << 13)
    };

    f32::from_bits(value)
}
