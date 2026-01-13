//! Built-in theme definitions.

use serde::{Deserialize, Serialize};

/// Theme definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub colors: ThemeColors,
    pub typography: ThemeTypography,
    pub spacing: ThemeSpacing,
    pub effects: ThemeEffects,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeColors {
    pub background: String,
    pub background_secondary: String,
    pub background_tertiary: String,
    pub foreground: String,
    pub foreground_muted: String,
    pub foreground_subtle: String,
    pub accent: String,
    pub accent_hover: String,
    pub accent_foreground: String,
    pub border: String,
    pub border_subtle: String,
    pub error: String,
    pub warning: String,
    pub success: String,
    pub unread: String,
    pub read: String,
    pub link: String,
    pub link_visited: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeTypography {
    pub font_family: String,
    pub font_family_mono: String,
    pub font_size_base: String,
    pub line_height: String,
    pub reader_font_family: String,
    pub reader_font_size: String,
    pub reader_line_height: String,
    pub reader_max_width: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSpacing {
    pub unit: String,
    pub border_radius: String,
    pub sidebar_width: String,
    pub list_width: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeEffects {
    pub transparency: i32,
    pub blur: i32,
    pub shadow: String,
}

/// Get all built-in themes
pub fn get_builtin_themes() -> Vec<Theme> {
    vec![
        light_theme(),
        dark_theme(),
        nord_theme(),
        catppuccin_latte(),
        catppuccin_frappe(),
        catppuccin_macchiato(),
        catppuccin_mocha(),
        solarized_light(),
        solarized_dark(),
        dracula_theme(),
    ]
}

/// Get a theme by ID
pub fn get_theme(id: &str) -> Option<Theme> {
    get_builtin_themes().into_iter().find(|t| t.id == id)
}

fn default_typography() -> ThemeTypography {
    ThemeTypography {
        font_family: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif"
            .to_string(),
        font_family_mono: "'SF Mono', Monaco, 'Cascadia Code', monospace".to_string(),
        font_size_base: "14px".to_string(),
        line_height: "1.5".to_string(),
        reader_font_family: "Georgia, 'Times New Roman', serif".to_string(),
        reader_font_size: "18px".to_string(),
        reader_line_height: "1.7".to_string(),
        reader_max_width: "680px".to_string(),
    }
}

fn default_spacing() -> ThemeSpacing {
    ThemeSpacing {
        unit: "4px".to_string(),
        border_radius: "6px".to_string(),
        sidebar_width: "260px".to_string(),
        list_width: "380px".to_string(),
    }
}

fn default_effects() -> ThemeEffects {
    ThemeEffects {
        transparency: 100,
        blur: 0,
        shadow: "0 1px 3px rgba(0, 0, 0, 0.1)".to_string(),
    }
}

pub fn light_theme() -> Theme {
    Theme {
        id: "light".to_string(),
        name: "Light".to_string(),
        author: Some("Curio Reader".to_string()),
        colors: ThemeColors {
            background: "#ffffff".to_string(),
            background_secondary: "#f9fafb".to_string(),
            background_tertiary: "#f3f4f6".to_string(),
            foreground: "#111827".to_string(),
            foreground_muted: "#6b7280".to_string(),
            foreground_subtle: "#9ca3af".to_string(),
            accent: "#3b82f6".to_string(),
            accent_hover: "#2563eb".to_string(),
            accent_foreground: "#ffffff".to_string(),
            border: "#e5e7eb".to_string(),
            border_subtle: "#f3f4f6".to_string(),
            error: "#ef4444".to_string(),
            warning: "#f59e0b".to_string(),
            success: "#22c55e".to_string(),
            unread: "#111827".to_string(),
            read: "#9ca3af".to_string(),
            link: "#3b82f6".to_string(),
            link_visited: "#8b5cf6".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn dark_theme() -> Theme {
    Theme {
        id: "dark".to_string(),
        name: "Dark".to_string(),
        author: Some("Curio Reader".to_string()),
        colors: ThemeColors {
            background: "#0f0f0f".to_string(),
            background_secondary: "#171717".to_string(),
            background_tertiary: "#262626".to_string(),
            foreground: "#fafafa".to_string(),
            foreground_muted: "#a1a1aa".to_string(),
            foreground_subtle: "#71717a".to_string(),
            accent: "#3b82f6".to_string(),
            accent_hover: "#60a5fa".to_string(),
            accent_foreground: "#ffffff".to_string(),
            border: "#27272a".to_string(),
            border_subtle: "#1f1f1f".to_string(),
            error: "#f87171".to_string(),
            warning: "#fbbf24".to_string(),
            success: "#4ade80".to_string(),
            unread: "#fafafa".to_string(),
            read: "#71717a".to_string(),
            link: "#60a5fa".to_string(),
            link_visited: "#a78bfa".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: ThemeEffects {
            transparency: 100,
            blur: 0,
            shadow: "0 1px 3px rgba(0, 0, 0, 0.3)".to_string(),
        },
    }
}

pub fn nord_theme() -> Theme {
    Theme {
        id: "nord".to_string(),
        name: "Nord".to_string(),
        author: Some("Arctic Ice Studio".to_string()),
        colors: ThemeColors {
            background: "#2e3440".to_string(),
            background_secondary: "#3b4252".to_string(),
            background_tertiary: "#434c5e".to_string(),
            foreground: "#eceff4".to_string(),
            foreground_muted: "#d8dee9".to_string(),
            foreground_subtle: "#4c566a".to_string(),
            accent: "#88c0d0".to_string(),
            accent_hover: "#8fbcbb".to_string(),
            accent_foreground: "#2e3440".to_string(),
            border: "#4c566a".to_string(),
            border_subtle: "#3b4252".to_string(),
            error: "#bf616a".to_string(),
            warning: "#ebcb8b".to_string(),
            success: "#a3be8c".to_string(),
            unread: "#eceff4".to_string(),
            read: "#4c566a".to_string(),
            link: "#81a1c1".to_string(),
            link_visited: "#b48ead".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn catppuccin_latte() -> Theme {
    Theme {
        id: "catppuccin-latte".to_string(),
        name: "Catppuccin Latte".to_string(),
        author: Some("Catppuccin".to_string()),
        colors: ThemeColors {
            background: "#eff1f5".to_string(),
            background_secondary: "#e6e9ef".to_string(),
            background_tertiary: "#dce0e8".to_string(),
            foreground: "#4c4f69".to_string(),
            foreground_muted: "#6c6f85".to_string(),
            foreground_subtle: "#9ca0b0".to_string(),
            accent: "#1e66f5".to_string(),
            accent_hover: "#7287fd".to_string(),
            accent_foreground: "#eff1f5".to_string(),
            border: "#ccd0da".to_string(),
            border_subtle: "#dce0e8".to_string(),
            error: "#d20f39".to_string(),
            warning: "#df8e1d".to_string(),
            success: "#40a02b".to_string(),
            unread: "#4c4f69".to_string(),
            read: "#9ca0b0".to_string(),
            link: "#1e66f5".to_string(),
            link_visited: "#8839ef".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn catppuccin_frappe() -> Theme {
    Theme {
        id: "catppuccin-frappe".to_string(),
        name: "Catppuccin Frappé".to_string(),
        author: Some("Catppuccin".to_string()),
        colors: ThemeColors {
            background: "#303446".to_string(),
            background_secondary: "#292c3c".to_string(),
            background_tertiary: "#414559".to_string(),
            foreground: "#c6d0f5".to_string(),
            foreground_muted: "#a5adce".to_string(),
            foreground_subtle: "#626880".to_string(),
            accent: "#8caaee".to_string(),
            accent_hover: "#babbf1".to_string(),
            accent_foreground: "#303446".to_string(),
            border: "#414559".to_string(),
            border_subtle: "#292c3c".to_string(),
            error: "#e78284".to_string(),
            warning: "#e5c890".to_string(),
            success: "#a6d189".to_string(),
            unread: "#c6d0f5".to_string(),
            read: "#626880".to_string(),
            link: "#8caaee".to_string(),
            link_visited: "#ca9ee6".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn catppuccin_macchiato() -> Theme {
    Theme {
        id: "catppuccin-macchiato".to_string(),
        name: "Catppuccin Macchiato".to_string(),
        author: Some("Catppuccin".to_string()),
        colors: ThemeColors {
            background: "#24273a".to_string(),
            background_secondary: "#1e2030".to_string(),
            background_tertiary: "#363a4f".to_string(),
            foreground: "#cad3f5".to_string(),
            foreground_muted: "#a5adcb".to_string(),
            foreground_subtle: "#5b6078".to_string(),
            accent: "#8aadf4".to_string(),
            accent_hover: "#b7bdf8".to_string(),
            accent_foreground: "#24273a".to_string(),
            border: "#363a4f".to_string(),
            border_subtle: "#1e2030".to_string(),
            error: "#ed8796".to_string(),
            warning: "#eed49f".to_string(),
            success: "#a6da95".to_string(),
            unread: "#cad3f5".to_string(),
            read: "#5b6078".to_string(),
            link: "#8aadf4".to_string(),
            link_visited: "#c6a0f6".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn catppuccin_mocha() -> Theme {
    Theme {
        id: "catppuccin-mocha".to_string(),
        name: "Catppuccin Mocha".to_string(),
        author: Some("Catppuccin".to_string()),
        colors: ThemeColors {
            background: "#1e1e2e".to_string(),
            background_secondary: "#181825".to_string(),
            background_tertiary: "#313244".to_string(),
            foreground: "#cdd6f4".to_string(),
            foreground_muted: "#a6adc8".to_string(),
            foreground_subtle: "#585b70".to_string(),
            accent: "#89b4fa".to_string(),
            accent_hover: "#b4befe".to_string(),
            accent_foreground: "#1e1e2e".to_string(),
            border: "#313244".to_string(),
            border_subtle: "#181825".to_string(),
            error: "#f38ba8".to_string(),
            warning: "#f9e2af".to_string(),
            success: "#a6e3a1".to_string(),
            unread: "#cdd6f4".to_string(),
            read: "#585b70".to_string(),
            link: "#89b4fa".to_string(),
            link_visited: "#cba6f7".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn solarized_light() -> Theme {
    Theme {
        id: "solarized-light".to_string(),
        name: "Solarized Light".to_string(),
        author: Some("Ethan Schoonover".to_string()),
        colors: ThemeColors {
            background: "#fdf6e3".to_string(),
            background_secondary: "#eee8d5".to_string(),
            background_tertiary: "#e4ddc8".to_string(),
            foreground: "#657b83".to_string(),
            foreground_muted: "#839496".to_string(),
            foreground_subtle: "#93a1a1".to_string(),
            accent: "#268bd2".to_string(),
            accent_hover: "#2aa198".to_string(),
            accent_foreground: "#fdf6e3".to_string(),
            border: "#eee8d5".to_string(),
            border_subtle: "#e4ddc8".to_string(),
            error: "#dc322f".to_string(),
            warning: "#cb4b16".to_string(),
            success: "#859900".to_string(),
            unread: "#073642".to_string(),
            read: "#93a1a1".to_string(),
            link: "#268bd2".to_string(),
            link_visited: "#6c71c4".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn solarized_dark() -> Theme {
    Theme {
        id: "solarized-dark".to_string(),
        name: "Solarized Dark".to_string(),
        author: Some("Ethan Schoonover".to_string()),
        colors: ThemeColors {
            background: "#002b36".to_string(),
            background_secondary: "#073642".to_string(),
            background_tertiary: "#0a4050".to_string(),
            foreground: "#839496".to_string(),
            foreground_muted: "#657b83".to_string(),
            foreground_subtle: "#586e75".to_string(),
            accent: "#268bd2".to_string(),
            accent_hover: "#2aa198".to_string(),
            accent_foreground: "#002b36".to_string(),
            border: "#073642".to_string(),
            border_subtle: "#0a4050".to_string(),
            error: "#dc322f".to_string(),
            warning: "#cb4b16".to_string(),
            success: "#859900".to_string(),
            unread: "#fdf6e3".to_string(),
            read: "#586e75".to_string(),
            link: "#268bd2".to_string(),
            link_visited: "#6c71c4".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

pub fn dracula_theme() -> Theme {
    Theme {
        id: "dracula".to_string(),
        name: "Dracula".to_string(),
        author: Some("Zeno Rocha".to_string()),
        colors: ThemeColors {
            background: "#282a36".to_string(),
            background_secondary: "#21222c".to_string(),
            background_tertiary: "#44475a".to_string(),
            foreground: "#f8f8f2".to_string(),
            foreground_muted: "#bfbfbf".to_string(),
            foreground_subtle: "#6272a4".to_string(),
            accent: "#bd93f9".to_string(),
            accent_hover: "#ff79c6".to_string(),
            accent_foreground: "#282a36".to_string(),
            border: "#44475a".to_string(),
            border_subtle: "#21222c".to_string(),
            error: "#ff5555".to_string(),
            warning: "#ffb86c".to_string(),
            success: "#50fa7b".to_string(),
            unread: "#f8f8f2".to_string(),
            read: "#6272a4".to_string(),
            link: "#8be9fd".to_string(),
            link_visited: "#ff79c6".to_string(),
        },
        typography: default_typography(),
        spacing: default_spacing(),
        effects: default_effects(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_builtin_themes() {
        let themes = get_builtin_themes();
        assert!(themes.len() >= 10);

        // Verify all themes have unique IDs
        let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
        let unique_count = ids.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(ids.len(), unique_count);
    }

    #[test]
    fn test_get_theme() {
        assert!(get_theme("light").is_some());
        assert!(get_theme("dark").is_some());
        assert!(get_theme("nord").is_some());
        assert!(get_theme("nonexistent").is_none());
    }

    #[test]
    fn test_theme_colors_valid_hex() {
        let themes = get_builtin_themes();
        for theme in themes {
            assert!(theme.colors.background.starts_with('#'));
            assert!(theme.colors.foreground.starts_with('#'));
            assert!(theme.colors.accent.starts_with('#'));
        }
    }
}
