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
            .map(|value| {
                config
                    .app_name_pattern
                    .replace("{app_name}", &sanitize(value))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    pattern = pattern.replace("{mode}", kind.label());
    pattern = pattern.replace("{app}", &app_segment);

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
