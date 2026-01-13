//! Application settings model.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Appearance
    pub theme_id: String,
    pub custom_css: Option<String>,
    pub window_transparency: i32,
    pub window_blur: i32,

    // Startup
    pub startup_behavior: StartupBehavior,

    // Behavior
    pub refresh_interval: i32,
    pub fetch_concurrency: i32,
    pub mark_read_on_scroll: bool,
    pub mark_read_delay: i32,
    pub open_links_in_browser: bool,
    pub read_later_auto_remove: bool,

    // Podcast
    pub podcast_playback_speed: f32,
    pub podcast_skip_forward: i32,
    pub podcast_skip_back: i32,
    pub podcast_auto_download: bool,
    pub podcast_auto_cleanup_days: Option<i32>,

    // Notifications
    pub notifications_enabled: bool,
    pub notification_sound: bool,

    // Cache
    pub image_cache_days: i32,
    pub thumbnail_cache_days: i32,
    pub image_cache_max_gb: f32,
    pub article_retention_days: Option<i32>,

    // yt-dlp
    pub ytdlp_auto_update: bool,
    pub ytdlp_update_check_days: i32,

    // Export
    pub export_settings: ExportSettings,
    pub obsidian_vault_path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_id: "dark".to_string(),
            custom_css: None,
            window_transparency: 100,
            window_blur: 0,
            startup_behavior: StartupBehavior::RestoreLastView,
            refresh_interval: 900, // 15 minutes
            fetch_concurrency: 20,
            mark_read_on_scroll: false,
            mark_read_delay: 2000,
            open_links_in_browser: true,
            read_later_auto_remove: false,
            podcast_playback_speed: 1.0,
            podcast_skip_forward: 30,
            podcast_skip_back: 15,
            podcast_auto_download: false,
            podcast_auto_cleanup_days: None,
            notifications_enabled: true,
            notification_sound: true,
            image_cache_days: 30,
            thumbnail_cache_days: 30,
            image_cache_max_gb: 2.0,
            article_retention_days: None,
            ytdlp_auto_update: true,
            ytdlp_update_check_days: 7,
            export_settings: ExportSettings::default(),
            obsidian_vault_path: None,
        }
    }
}

impl Settings {
    /// Get refresh interval as duration
    pub fn refresh_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.refresh_interval as u64)
    }

    /// Check if window has transparency enabled
    pub fn has_transparency(&self) -> bool {
        self.window_transparency < 100
    }

    /// Check if window has blur enabled
    pub fn has_blur(&self) -> bool {
        self.window_blur > 0
    }
}

/// Startup behavior options
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type", content = "value")]
pub enum StartupBehavior {
    #[default]
    RestoreLastView,
    AllUnread,
    SpecificFolder(Uuid),
    SpecificFeed(Uuid),
}

/// Export settings for Markdown export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub include_frontmatter: bool,
    pub frontmatter_fields: Vec<String>,
    pub image_handling: ImageExportMode,
    pub link_style: LinkStyle,
    pub include_source_link: bool,
    pub filename_template: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            include_frontmatter: true,
            frontmatter_fields: vec![
                "title".to_string(),
                "author".to_string(),
                "source".to_string(),
                "published".to_string(),
            ],
            image_handling: ImageExportMode::Inline,
            link_style: LinkStyle::Inline,
            include_source_link: true,
            filename_template: "{date}-{title}".to_string(),
        }
    }
}

/// How to handle images in exports
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageExportMode {
    #[default]
    Inline,
    LocalCopy,
    Base64Embed,
    Strip,
}

/// Link format in Markdown exports
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LinkStyle {
    #[default]
    Inline,
    Reference,
}

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

/// Theme color definitions
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

/// Theme typography settings
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

/// Theme spacing settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSpacing {
    pub unit: String,
    pub border_radius: String,
    pub sidebar_width: String,
    pub list_width: String,
}

/// Theme visual effects
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeEffects {
    pub transparency: i32,
    pub blur: i32,
    pub shadow: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let settings = Settings::default();

        assert_eq!(settings.theme_id, "dark");
        assert_eq!(settings.refresh_interval, 900);
        assert_eq!(settings.fetch_concurrency, 20);
        assert!(settings.notifications_enabled);
        assert_eq!(settings.podcast_playback_speed, 1.0);
    }

    #[test]
    fn test_settings_refresh_duration() {
        let settings = Settings::default();
        let duration = settings.refresh_duration();

        assert_eq!(duration.as_secs(), 900);
    }

    #[test]
    fn test_settings_transparency() {
        let mut settings = Settings {
            window_transparency: 100,
            ..Default::default()
        };
        assert!(!settings.has_transparency());

        settings.window_transparency = 80;
        assert!(settings.has_transparency());
    }

    #[test]
    fn test_settings_blur() {
        let mut settings = Settings {
            window_blur: 0,
            ..Default::default()
        };
        assert!(!settings.has_blur());

        settings.window_blur = 10;
        assert!(settings.has_blur());
    }

    #[test]
    fn test_startup_behavior_serialization() {
        let restore = StartupBehavior::RestoreLastView;
        let json = serde_json::to_string(&restore).unwrap();
        assert!(json.contains("RestoreLastView"));

        let folder_id = Uuid::new_v4();
        let specific = StartupBehavior::SpecificFolder(folder_id);
        let json = serde_json::to_string(&specific).unwrap();
        assert!(json.contains("SpecificFolder"));
        assert!(json.contains(&folder_id.to_string()));
    }

    #[test]
    fn test_export_settings_default() {
        let settings = ExportSettings::default();

        assert!(settings.include_frontmatter);
        assert!(settings.include_source_link);
        assert!(!settings.frontmatter_fields.is_empty());
        assert!(matches!(settings.image_handling, ImageExportMode::Inline));
        assert!(matches!(settings.link_style, LinkStyle::Inline));
    }

    #[test]
    fn test_image_export_mode_serialization() {
        let mode = ImageExportMode::LocalCopy;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"local_copy\"");

        let deserialized: ImageExportMode = serde_json::from_str("\"base64_embed\"").unwrap();
        assert!(matches!(deserialized, ImageExportMode::Base64Embed));
    }

    #[test]
    fn test_settings_serialization() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();

        assert!(json.contains("\"theme_id\":\"dark\""));
        assert!(json.contains("\"refresh_interval\":900"));

        let deserialized: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.theme_id, settings.theme_id);
        assert_eq!(deserialized.refresh_interval, settings.refresh_interval);
    }
}
