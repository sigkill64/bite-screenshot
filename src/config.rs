use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default = "default_filename_pattern")]
    pub filename_pattern: String,
    #[serde(default = "default_app_name_pattern")]
    pub app_name_pattern: String,
    #[serde(default)]
    pub hotkeys: HotkeyConfig,
    #[serde(default = "default_true")]
    pub show_notification: bool,
    #[serde(default)]
    pub auto_open: bool,
    pub open_command: Option<String>,
    #[serde(default = "default_region_side")]
    pub region_side: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyConfig {
    #[serde(default = "default_fullscreen_hotkey")]
    pub fullscreen: String,
    #[serde(default = "default_window_hotkey")]
    pub current_window: String,
    #[serde(default = "default_region_hotkey")]
    pub square_region: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            fullscreen: default_fullscreen_hotkey(),
            current_window: default_window_hotkey(),
            square_region: default_region_hotkey(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            let config = Self::default();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
            }
            fs::write(&path, config.example_text())
                .with_context(|| format!("写入默认配置失败: {}", path.display()))?;
            return Ok(config);
        }

        let text = fs::read_to_string(&path)
            .with_context(|| format!("读取配置失败: {}", path.display()))?;
        let config = toml::from_str::<Config>(&text)
            .with_context(|| format!("解析配置失败: {}", path.display()))?;
        Ok(config)
    }

    pub fn path() -> Result<PathBuf> {
        let base = BaseDirs::new().context("无法定位 APPDATA 目录")?;
        Ok(base.data_dir().join("BiteToys/config/screenshot.conf"))
    }

    pub fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            filename_pattern: default_filename_pattern(),
            app_name_pattern: default_app_name_pattern(),
            hotkeys: HotkeyConfig::default(),
            show_notification: true,
            auto_open: false,
            open_command: None,
            region_side: default_region_side(),
        }
    }

    fn example_text(&self) -> String {
        format!(
            r#"output_dir = "{}"
filename_pattern = "{}"
app_name_pattern = "{}"
show_notification = {}
auto_open = {}
# open_command = "mspaint.exe"
region_side = {}

[hotkeys]
fullscreen = "{}"
current_window = "{}"
square_region = "{}"
"#,
            self.output_dir.display(),
            self.filename_pattern,
            self.app_name_pattern,
            self.show_notification,
            self.auto_open,
            self.region_side,
            self.hotkeys.fullscreen,
            self.hotkeys.current_window,
            self.hotkeys.square_region,
        )
    }
}

fn default_output_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures/BiteScreenshots")
}

fn default_filename_pattern() -> String {
    "%Y-%m-%d_%H-%M-%S_{mode}{app}.png".to_string()
}

fn default_app_name_pattern() -> String {
    "_{app_name}".to_string()
}

fn default_fullscreen_hotkey() -> String {
    "Ctrl+Alt+1".to_string()
}

fn default_window_hotkey() -> String {
    "Ctrl+Alt+2".to_string()
}

fn default_region_hotkey() -> String {
    "Ctrl+Alt+3".to_string()
}

fn default_true() -> bool {
    true
}

fn default_region_side() -> u32 {
    512
}
