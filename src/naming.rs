use chrono::Local;

use crate::app::CaptureKind;
use crate::config::Config;

pub fn build_file_name(
    config: &Config,
    kind: CaptureKind,
    app_name: Option<&str>,
    ext: &str,
) -> String {
    let mut pattern = config.filename_pattern.clone();
    let app_segment = if matches!(kind, CaptureKind::CurrentWindow) {
        app_name
            .filter(|value| !value.is_empty())
            .map(|value| replace_app_name_placeholders(&config.app_name_pattern, &sanitize(value)))
            .unwrap_or_default()
    } else {
        String::new()
    };

    pattern = replace_mode_placeholders(&pattern, kind.label());
    pattern = replace_app_placeholders(&pattern, &app_segment);

    let mut rendered = Local::now().format(&pattern).to_string();
    if let Some((stem, _)) = rendered.rsplit_once('.') {
        rendered = stem.to_string();
    }
    format!("{rendered}{ext}")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn replace_mode_placeholders(pattern: &str, mode: &str) -> String {
    pattern.replace("{Mode}", mode)
}

fn replace_app_placeholders(pattern: &str, app: &str) -> String {
    pattern.replace("{App}", app)
}

fn replace_app_name_placeholders(pattern: &str, app_name: &str) -> String {
    pattern.replace("{AppName}", app_name)
}
