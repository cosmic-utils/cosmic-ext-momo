// SPDX-License-Identifier: {{LICENSE}}

use cosmic::{
    cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, Config, CosmicConfigEntry},
    Application,
};

use crate::app::AppModel;

#[derive(
    Debug, Default, Clone, serde::Serialize, serde::Deserialize, CosmicConfigEntry, Eq, PartialEq,
)]
#[serde(default)]
#[version = 1]
pub struct MomoConfig {
    pub server: String,
    /// First-run setup was completed or public browsing was chosen.
    pub onboarding_completed: bool,
    /// Hide boosted (reblogged) statuses from timelines.
    pub hide_boosts: bool,
    pub disable_desktop_notifications: bool,
    /// Hide reply statuses from timelines.
    pub hide_replies: bool,
    /// How much of each post to render in feeds.
    pub feed_density: FeedDensity,
    /// Preferred theme.
    pub theme_mode: ThemeMode,
}

/// The user's preferred theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

    pub fn label(self) -> String {
        match self {
            ThemeMode::Light => crate::fl!("settings-theme-light"),
            ThemeMode::Dark => crate::fl!("settings-theme-dark"),
            ThemeMode::System => crate::fl!("settings-theme-system"),
        }
    }

    pub fn theme(self) -> cosmic::Theme {
        match self {
            ThemeMode::Light => cosmic::Theme::light(),
            ThemeMode::Dark => cosmic::Theme::dark(),
            ThemeMode::System => cosmic::theme::system_preference(),
        }
    }
}

/// How much of each post to render in feeds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FeedDensity {
    /// Just the header and text content — no card, media, or tags.
    TextOnly,
    /// Card, media, and tags too, but shown smaller.
    Compact,
    /// Everything, at full size.
    #[default]
    Full,
}

impl FeedDensity {
    pub const ALL: [FeedDensity; 3] = [
        FeedDensity::TextOnly,
        FeedDensity::Compact,
        FeedDensity::Full,
    ];

    pub fn label(self) -> String {
        match self {
            FeedDensity::TextOnly => crate::fl!("settings-density-text"),
            FeedDensity::Compact => crate::fl!("settings-density-compact"),
            FeedDensity::Full => crate::fl!("settings-density-full"),
        }
    }
}

impl MomoConfig {
    pub fn config_handler() -> Option<Config> {
        Config::new(AppModel::APP_ID, MomoConfig::VERSION).ok()
    }

    pub fn config() -> MomoConfig {
        match Self::config_handler() {
            Some(config_handler) => {
                MomoConfig::get_entry(&config_handler).unwrap_or_else(|(errs, config)| {
                    tracing::error!("errors loading config: {:?}", errs);
                    config
                })
            }
            None => MomoConfig::default(),
        }
    }
}
