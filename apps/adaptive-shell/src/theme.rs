//! Fieldglass semantic theme roles and accessibility settings.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Supported semantic palettes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    /// Warm dark-neutral default.
    #[default]
    Dark,
    /// Light-neutral palette.
    Light,
    /// Explicit borders and high contrast, with no transparency dependency.
    Contrast,
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Contrast => "contrast",
        })
    }
}

/// Spacing/control density.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// 4/7/10 spacing and 31 px compact controls.
    Tight,
    /// 4/7/10/14 spacing and 36 px controls.
    #[default]
    Cozy,
    /// 7/10/14/20 spacing and 42 px controls.
    Open,
}

impl fmt::Display for Density {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tight => "tight",
            Self::Cozy => "cozy",
            Self::Open => "open",
        })
    }
}

/// Text scale values supported by the shell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum TextScale {
    /// 90% scale.
    P90,
    /// 100% scale.
    #[default]
    P100,
    /// 118% scale.
    P118,
    /// 200% accessibility scale.
    P200,
}

impl TextScale {
    /// Return the scale as a percentage.
    pub const fn percent(self) -> u16 {
        match self {
            Self::P90 => 90,
            Self::P100 => 100,
            Self::P118 => 118,
            Self::P200 => 200,
        }
    }
}

/// A typed role value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TokenValue {
    /// RGBA color.
    Color([u8; 4]),
    /// Logical pixels.
    Pixels(u16),
    /// Text size in logical pixels.
    TypeScale(u16),
    /// Motion duration in milliseconds.
    Millis(u16),
}

/// Complete role-resolved Fieldglass token map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeTokens {
    /// Stable semantic token role map.
    pub roles: BTreeMap<String, TokenValue>,
}

impl ThemeTokens {
    /// Every role required by `aos.theme/1` in this tranche.
    pub const REQUIRED_ROLES: [&'static str; 39] = [
        "aos.color.canvas",
        "aos.color.canvas-raised",
        "aos.color.fieldglass",
        "aos.color.layer.1",
        "aos.color.layer.2",
        "aos.color.layer.3",
        "aos.color.text",
        "aos.color.text-soft",
        "aos.color.text-dim",
        "aos.color.line",
        "aos.color.line-strong",
        "aos.color.accent",
        "aos.color.accent-strong",
        "aos.color.accent-wash",
        "aos.color.success",
        "aos.color.warning",
        "aos.color.danger",
        "aos.color.info",
        "aos.color.focus",
        "aos.radius.detail",
        "aos.radius.control",
        "aos.radius.window",
        "aos.space.1",
        "aos.space.2",
        "aos.space.3",
        "aos.space.4",
        "aos.space.5",
        "aos.space.6",
        "aos.control.height.compact",
        "aos.control.height.cozy",
        "aos.control.height.spacious",
        "aos.type.caption",
        "aos.type.body",
        "aos.type.control",
        "aos.type.title",
        "aos.type.display",
        "aos.motion.fast",
        "aos.motion.normal",
        "aos.motion.layout",
    ];

    /// Resolve the requested palette and accessibility settings.
    pub fn resolve(
        name: ThemeName,
        density: Density,
        scale: TextScale,
        reduced_motion: bool,
    ) -> Self {
        let (canvas, raised, fieldglass, text, text_soft, text_dim, line, line_strong) = match name
        {
            ThemeName::Dark => (
                [12, 14, 17, 255],
                [23, 25, 30, 255],
                [27, 29, 35, 246],
                [244, 242, 247, 255],
                [194, 193, 204, 255],
                [156, 155, 166, 255],
                [61, 62, 71, 255],
                [85, 86, 99, 255],
            ),
            ThemeName::Light => (
                [245, 245, 247, 255],
                [255, 255, 255, 255],
                [247, 247, 250, 248],
                [28, 29, 34, 255],
                [77, 78, 88, 255],
                [95, 96, 105, 255],
                [205, 206, 214, 255],
                [170, 171, 183, 255],
            ),
            ThemeName::Contrast => (
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [245, 245, 245, 255],
                [220, 220, 220, 255],
                [255, 255, 255, 255],
                [255, 255, 0, 255],
            ),
        };
        let accent = match name {
            ThemeName::Contrast => [255, 210, 0, 255],
            ThemeName::Light => [95, 47, 213, 255],
            ThemeName::Dark => [157, 122, 255, 255],
        };
        let (success, warning, danger, info, focus) = match name {
            ThemeName::Light => (
                [17, 124, 88, 255],
                [154, 94, 7, 255],
                [169, 39, 55, 255],
                [12, 91, 165, 255],
                [9, 75, 145, 255],
            ),
            ThemeName::Contrast => (
                [76, 208, 149, 255],
                [242, 181, 89, 255],
                [235, 105, 121, 255],
                [92, 170, 245, 255],
                [111, 180, 255, 255],
            ),
            ThemeName::Dark => (
                [76, 208, 149, 255],
                [242, 181, 89, 255],
                [235, 105, 121, 255],
                [92, 170, 245, 255],
                [111, 180, 255, 255],
            ),
        };
        let mut roles = BTreeMap::new();
        for (key, value) in [
            ("aos.color.canvas", TokenValue::Color(canvas)),
            ("aos.color.canvas-raised", TokenValue::Color(raised)),
            ("aos.color.fieldglass", TokenValue::Color(fieldglass)),
            ("aos.color.layer.1", TokenValue::Color(raised)),
            ("aos.color.layer.2", TokenValue::Color([31, 33, 40, 255])),
            ("aos.color.layer.3", TokenValue::Color([43, 44, 52, 255])),
            ("aos.color.text", TokenValue::Color(text)),
            ("aos.color.text-soft", TokenValue::Color(text_soft)),
            ("aos.color.text-dim", TokenValue::Color(text_dim)),
            ("aos.color.line", TokenValue::Color(line)),
            ("aos.color.line-strong", TokenValue::Color(line_strong)),
            ("aos.color.accent", TokenValue::Color(accent)),
            (
                "aos.color.accent-strong",
                TokenValue::Color([187, 159, 255, 255]),
            ),
            (
                "aos.color.accent-wash",
                TokenValue::Color([132, 96, 225, 90]),
            ),
            ("aos.color.success", TokenValue::Color(success)),
            ("aos.color.warning", TokenValue::Color(warning)),
            ("aos.color.danger", TokenValue::Color(danger)),
            ("aos.color.info", TokenValue::Color(info)),
            ("aos.color.focus", TokenValue::Color(focus)),
        ] {
            roles.insert(key.to_owned(), value);
        }
        for (key, value) in [
            ("aos.radius.detail", 6),
            ("aos.radius.control", 9),
            ("aos.radius.window", 15),
        ] {
            roles.insert(key.to_owned(), TokenValue::Pixels(value));
        }
        let spaces = match density {
            Density::Tight => [4, 6, 9, 12, 16, 22],
            Density::Cozy => [4, 7, 10, 14, 20, 28],
            Density::Open => [5, 9, 13, 18, 26, 34],
        };
        for (index, value) in spaces.into_iter().enumerate() {
            roles.insert(
                format!("aos.space.{}", index + 1),
                TokenValue::Pixels(value),
            );
        }
        let heights = match density {
            Density::Tight => [31, 34, 38],
            Density::Cozy => [31, 36, 42],
            Density::Open => [36, 42, 48],
        };
        for (key, value) in [
            "aos.control.height.compact",
            "aos.control.height.cozy",
            "aos.control.height.spacious",
        ]
        .into_iter()
        .zip(heights)
        {
            roles.insert(key.to_owned(), TokenValue::Pixels(value));
        }
        let base = u16::try_from(u32::from(scale.percent()) * 16 / 100).unwrap_or(16);
        for (key, multiplier) in [
            ("aos.type.caption", 0.75_f32),
            ("aos.type.body", 0.875),
            ("aos.type.control", 0.875),
            ("aos.type.title", 1.375),
            ("aos.type.display", 2.25),
        ] {
            roles.insert(
                key.to_owned(),
                TokenValue::TypeScale((f32::from(base) * multiplier).round() as u16),
            );
        }
        let motions = if reduced_motion {
            [0, 0, 0]
        } else {
            [90, 150, 240]
        };
        for (key, value) in ["aos.motion.fast", "aos.motion.normal", "aos.motion.layout"]
            .into_iter()
            .zip(motions)
        {
            roles.insert(key.to_owned(), TokenValue::Millis(value));
        }
        Self { roles }
    }

    /// Check that all required roles are present.
    pub fn is_complete(&self) -> bool {
        Self::REQUIRED_ROLES
            .iter()
            .all(|key| self.roles.contains_key(*key))
    }

    /// Resolve one role.
    pub fn get(&self, role: &str) -> Option<&TokenValue> {
        self.roles.get(role)
    }

    /// WCAG relative luminance for an opaque sRGB token.
    pub fn relative_luminance(color: [u8; 4]) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2])
    }

    /// WCAG contrast ratio, independent of foreground/background order.
    pub fn contrast_ratio(left: [u8; 4], right: [u8; 4]) -> f32 {
        let a = Self::relative_luminance(left);
        let b = Self::relative_luminance(right);
        let lighter = a.max(b);
        let darker = a.min(b);
        (lighter + 0.05) / (darker + 0.05)
    }

    /// Fail closed if required colors are absent, transparent, or too low contrast.
    pub fn validate_accessibility(&self) -> Result<(), &'static str> {
        let color = |role: &str| match self.roles.get(role) {
            Some(TokenValue::Color(color)) if color[3] == 255 => Ok(*color),
            Some(TokenValue::Color(_)) => Err("required accessibility token is transparent"),
            _ => Err("required accessibility token is absent"),
        };
        let canvas = color("aos.color.canvas")?;
        let raised = color("aos.color.canvas-raised")?;
        for background in [canvas, raised] {
            for role in [
                "aos.color.text",
                "aos.color.text-soft",
                "aos.color.text-dim",
            ] {
                let foreground = color(role)?;
                let ratio = Self::contrast_ratio(foreground, background);
                if ratio < 4.5 {
                    return Err("required token fails WCAG AA contrast");
                }
            }
            for role in [
                "aos.color.accent",
                "aos.color.success",
                "aos.color.warning",
                "aos.color.danger",
                "aos.color.info",
                "aos.color.focus",
            ] {
                let foreground = color(role)?;
                if Self::contrast_ratio(foreground, background) < 3.0 {
                    return Err(Box::leak(role.to_owned().into_boxed_str()));
                }
            }
        }
        Ok(())
    }
}

/// Theme selection plus accessibility settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Palette.
    pub name: ThemeName,
    /// Density.
    pub density: Density,
    /// Text scale.
    pub scale: TextScale,
    /// Whether movement is removed.
    pub reduced_motion: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: ThemeName::Dark,
            density: Density::Cozy,
            scale: TextScale::P100,
            reduced_motion: false,
        }
    }
}

/// Resolve tokens from a named config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Selected configuration.
    pub config: ThemeConfig,
    /// Resolved semantic roles.
    pub tokens: ThemeTokens,
}

impl Theme {
    /// Resolve a complete theme.
    pub fn resolve(config: ThemeConfig) -> Self {
        let tokens = ThemeTokens::resolve(
            config.name,
            config.density,
            config.scale,
            config.reduced_motion,
        );
        Self { config, tokens }
    }

    /// Stable digest used by deterministic display snapshots.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("theme is serializable");
        blake3::hash(&bytes).to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_resolves_every_role() {
        for name in [ThemeName::Dark, ThemeName::Light, ThemeName::Contrast] {
            for density in [Density::Tight, Density::Cozy, Density::Open] {
                let theme = Theme::resolve(ThemeConfig {
                    name,
                    density,
                    ..ThemeConfig::default()
                });
                assert!(theme.tokens.is_complete());
                assert_eq!(theme.tokens.roles.len(), ThemeTokens::REQUIRED_ROLES.len());
            }
        }
    }

    #[test]
    fn all_theme_combinations_meet_wcag_and_opaque_fallback() {
        for name in [ThemeName::Dark, ThemeName::Light, ThemeName::Contrast] {
            for density in [Density::Tight, Density::Cozy, Density::Open] {
                for scale in [
                    TextScale::P90,
                    TextScale::P100,
                    TextScale::P118,
                    TextScale::P200,
                ] {
                    let tokens = ThemeTokens::resolve(name, density, scale, true);
                    assert!(
                        tokens.validate_accessibility().is_ok(),
                        "{name:?} failed: {:?}",
                        tokens.validate_accessibility()
                    );
                }
            }
        }
    }
}
