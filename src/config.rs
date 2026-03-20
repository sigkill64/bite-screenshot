use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::Deserialize;
#[cfg(target_os = "windows")]
use windows::core::GUID;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HANDLE,
    System::Com::CoTaskMemFree,
    UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    },
};

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
    auto_open: bool,
    #[serde(default)]
    pub auto_open_sdr: Option<bool>,
    #[serde(default)]
    pub auto_open_hdr: Option<bool>,
    open_command: Option<String>,
    pub open_command_sdr: Option<String>,
    pub open_command_hdr: Option<String>,
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
        let mut config = toml::from_str::<Config>(&text)
            .with_context(|| format!("解析配置失败: {}", path.display()))?;
        config.expand_placeholders();
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
            auto_open_sdr: None,
            auto_open_hdr: None,
            open_command: None,
            open_command_sdr: None,
            open_command_hdr: None,
        }
    }

    fn example_text(&self) -> String {
        let output_dir = if self.output_dir == default_output_dir() {
            DEFAULT_OUTPUT_DIR_PLACEHOLDER.to_string()
        } else {
            self.output_dir.display().to_string().replace('/', "\\")
        };
        format!(
            r#"output_dir = '{}'
filename_pattern = "{}"
app_name_pattern = "{}"
show_notification = {}
auto_open_sdr = {}
auto_open_hdr = {}
# open_command_sdr = "mspaint.exe"
# open_command_hdr = "hdr-viewer.exe"

[hotkeys]
fullscreen = "{}"
current_window = "{}"
square_region = "{}"
"#,
            output_dir,
            self.filename_pattern,
            self.app_name_pattern,
            self.show_notification,
            self.auto_open_for(false),
            self.auto_open_for(true),
            self.hotkeys.fullscreen,
            self.hotkeys.current_window,
            self.hotkeys.square_region,
        )
    }

    fn expand_placeholders(&mut self) {
        self.output_dir = expand_path_placeholders(&self.output_dir);
    }

    pub fn auto_open_for(&self, hdr: bool) -> bool {
        if hdr {
            self.auto_open_hdr.unwrap_or(self.auto_open)
        } else {
            self.auto_open_sdr.unwrap_or(self.auto_open)
        }
    }

    pub fn open_command_for(&self, hdr: bool) -> Option<&str> {
        if hdr {
            self.open_command_hdr
                .as_deref()
                .or(self.open_command.as_deref())
        } else {
            self.open_command_sdr
                .as_deref()
                .or(self.open_command.as_deref())
        }
    }
}

const DEFAULT_OUTPUT_DIR_PLACEHOLDER: &str = r"{Pictures}\Screenshots";

fn default_output_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(path) = known_folder_path(&FOLDERID_Pictures) {
        return path.join("Screenshots");
    }

    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures")
        .join("Screenshots")
}

fn expand_path_placeholders(path: &PathBuf) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let mut value = path.display().to_string();
        for (placeholder, folder_id) in known_folder_placeholders() {
            if value.contains(placeholder) {
                if let Some(folder) = known_folder_path(folder_id) {
                    value = value.replace(placeholder, &folder.display().to_string());
                }
            }
        }
        return PathBuf::from(value);
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.clone()
    }
}

#[cfg(target_os = "windows")]
fn known_folder_placeholders() -> [(&'static str, &'static GUID); 6] {
    [
        ("{Desktop}", &FOLDERID_Desktop),
        ("{Documents}", &FOLDERID_Documents),
        ("{Downloads}", &FOLDERID_Downloads),
        ("{Music}", &FOLDERID_Music),
        ("{Pictures}", &FOLDERID_Pictures),
        ("{Videos}", &FOLDERID_Videos),
    ]
}

#[cfg(target_os = "windows")]
fn known_folder_path(folder_id: &GUID) -> Option<PathBuf> {
    let path =
        unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, HANDLE::default()) }.ok()?;

    let value = unsafe { path.to_string() }.ok().map(PathBuf::from);
    unsafe {
        CoTaskMemFree(Some(path.0 as _));
    }
    value
}

fn default_filename_pattern() -> String {
    "%Y-%m-%d_%H-%M-%S_{Mode}{App}.png".to_string()
}

fn default_app_name_pattern() -> String {
    "_{AppName}".to_string()
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
