/// Persists GUI layout settings (pane ratio, window geometry) across launches.
///
/// Stored as a simple key=value file at `<data_dir>/config/gui_layout.toml`.
/// Uses the same config directory as the rest of wyncast for consistency.
use std::path::PathBuf;

use wyncast_core::app_dirs;

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub pane_ratio: f32,
    pub window_width: f32,
    pub window_height: f32,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            pane_ratio: 0.65,
            window_width: 1280.0,
            window_height: 800.0,
            window_x: None,
            window_y: None,
        }
    }
}

fn config_path() -> PathBuf {
    let dir = app_dirs::config_dir();
    std::fs::create_dir_all(&dir).ok();
    dir.join("gui_layout.toml")
}

pub fn load() -> LayoutConfig {
    match std::fs::read_to_string(config_path()) {
        Ok(content) => parse(&content),
        Err(_) => LayoutConfig::default(),
    }
}

pub fn save(config: &LayoutConfig) {
    let mut lines = format!(
        "pane_ratio = {}\nwindow_width = {}\nwindow_height = {}\n",
        config.pane_ratio, config.window_width, config.window_height
    );
    if let (Some(x), Some(y)) = (config.window_x, config.window_y) {
        lines.push_str(&format!("window_x = {x}\nwindow_y = {y}\n"));
    }
    if let Err(e) = std::fs::write(config_path(), lines) {
        tracing::warn!("Failed to save gui_layout.toml: {e}");
    }
}

fn parse(s: &str) -> LayoutConfig {
    let mut c = LayoutConfig::default();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "pane_ratio" => {
                    if let Ok(v) = val.parse::<f32>() {
                        c.pane_ratio = v.clamp(0.1, 0.9);
                    }
                }
                "window_width" => {
                    if let Ok(v) = val.parse::<f32>() {
                        c.window_width = v.max(800.0);
                    }
                }
                "window_height" => {
                    if let Ok(v) = val.parse::<f32>() {
                        c.window_height = v.max(600.0);
                    }
                }
                "window_x" => {
                    if let Ok(v) = val.parse::<i32>() {
                        c.window_x = Some(v);
                    }
                }
                "window_y" => {
                    if let Ok(v) = val.parse::<i32>() {
                        c.window_y = Some(v);
                    }
                }
                _ => {}
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_ratio() {
        let c = LayoutConfig::default();
        assert!((c.pane_ratio - 0.65).abs() < 1e-6);
    }

    #[test]
    fn parse_round_trips() {
        let original = LayoutConfig {
            pane_ratio: 0.72,
            window_width: 1440.0,
            window_height: 900.0,
            window_x: Some(200),
            window_y: Some(150),
        };

        let text = format!(
            "pane_ratio = {}\nwindow_width = {}\nwindow_height = {}\nwindow_x = {}\nwindow_y = {}\n",
            original.pane_ratio,
            original.window_width,
            original.window_height,
            original.window_x.unwrap(),
            original.window_y.unwrap(),
        );

        let parsed = parse(&text);
        assert!((parsed.pane_ratio - original.pane_ratio).abs() < 1e-6);
        assert!((parsed.window_width - original.window_width).abs() < 1e-6);
        assert!((parsed.window_height - original.window_height).abs() < 1e-6);
        assert_eq!(parsed.window_x, original.window_x);
        assert_eq!(parsed.window_y, original.window_y);
    }

    #[test]
    fn parse_ignores_unknown_keys() {
        let text = "pane_ratio = 0.5\nunknown_key = value\nwindow_width = 1000.0\n";
        let c = parse(text);
        assert!((c.pane_ratio - 0.5).abs() < 1e-6);
        assert!((c.window_width - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn parse_clamps_ratio() {
        let text = "pane_ratio = 0.99\n";
        let c = parse(text);
        assert!(c.pane_ratio <= 0.9);

        let text2 = "pane_ratio = 0.01\n";
        let c2 = parse(text2);
        assert!(c2.pane_ratio >= 0.1);
    }

    #[test]
    fn parse_empty_string_returns_default() {
        let c = parse("");
        assert!((c.pane_ratio - 0.65).abs() < 1e-6);
    }

    #[test]
    fn parse_no_window_position_returns_none() {
        let text = "pane_ratio = 0.65\nwindow_width = 1280.0\nwindow_height = 800.0\n";
        let c = parse(text);
        assert!(c.window_x.is_none());
        assert!(c.window_y.is_none());
    }
}
