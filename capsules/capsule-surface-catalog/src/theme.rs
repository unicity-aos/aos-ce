//! `aos.theme/1` packs, bounded values, and fail-closed token fallback.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Stable theme contract identity.
pub const THEME_SCHEMA: &str = "aos.theme/1";
/// Number of complete themes shipped by this tranche.
pub const BUILT_IN_THEME_COUNT: usize = 2;
/// Canonical dark-first built-in theme identity.
pub const FIELDGLASS_THEME_ID: &str = "aos.builtin.fieldglass";
/// Canonical light-first built-in theme identity.
pub const PAPER_SIGNAL_THEME_ID: &str = "aos.builtin.paper-signal";

const SUPPORTED_TEXT_SCALES: [u8; 4] = [90, 100, 118, 200];
const APPROVED_LENGTHS: [u16; 14] = [0, 2, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192];
const APPROVED_SPACE: [u16; 15] = [0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 32, 40, 48];
const APPROVED_RADIUS: [u16; 7] = [0, 2, 4, 6, 8, 12, 16];
const APPROVED_DURATION: [u16; 8] = [0, 60, 90, 120, 150, 180, 240, 400];
const APPROVED_RATIO: [u16; 11] = [0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

/// Color environment represented by a theme environment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorEnvironment {
    /// Ordinary light environment.
    Light,
    /// Ordinary dark environment.
    Dark,
    /// High-contrast light environment.
    HighContrastLight,
    /// High-contrast dark environment.
    HighContrastDark,
}

/// Platform material family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Material {
    /// Opaque platform surface.
    Opaque,
    /// Translucent platform surface with a catalog-provided opaque fallback.
    Translucent,
    /// Explicit high-contrast surface.
    HighContrast,
}

/// Named spacing and control density.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// Compact spacing.
    Compact,
    /// Default spacing.
    Cozy,
    /// Spacious spacing.
    Spacious,
}

/// Bounded platform-material descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSpec {
    /// Platform material family.
    pub material: Material,
    /// System palette identity, not raw author CSS.
    pub system_palette: String,
    /// Bounded system display scale.
    pub display_scale_percent: u8,
    /// Safe-area insets in logical units.
    pub safe_area: SafeArea,
}

/// Logical safe-area inset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeArea {
    /// Top inset.
    pub top: u16,
    /// Leading inset.
    pub leading: u16,
    /// Trailing inset.
    pub trailing: u16,
    /// Bottom inset.
    pub bottom: u16,
}

/// Bounded user and workspace preferences supported by a pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// Pointer interaction is supported.
    pub pointer: bool,
    /// Direct keyboard interaction is supported.
    pub keyboard: bool,
    /// Supported bounded text scales.
    pub text_scale_percent: Vec<u8>,
    /// Reduced-motion presentation is supported.
    pub reduced_motion: bool,
    /// Bounded sound preference.
    pub sound: SoundDescriptor,
    /// Bounded haptic preference.
    pub haptic: HapticDescriptor,
}

/// Sound descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SoundDescriptor {
    /// No sound.
    None,
    /// Short interaction cue.
    Tap,
    /// Success cue.
    Success,
    /// Warning cue.
    Warning,
    /// Error cue.
    Error,
}

/// Haptic descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HapticDescriptor {
    /// No haptic.
    None,
    /// Light haptic.
    Light,
    /// Medium haptic.
    Medium,
    /// Strong haptic.
    Strong,
    /// Success haptic.
    Success,
    /// Warning haptic.
    Warning,
}

/// Typed, bounded semantic token value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TokenValue {
    /// RGBA color.
    Color([u8; 4]),
    /// Bounded logical or percentage length.
    Length {
        /// Numeric value.
        value: u16,
        /// Unit.
        unit: LengthUnit,
    },
    /// Finite ratio expressed in per-mille.
    Ratio(u16),
    /// Duration in milliseconds.
    DurationMs(u16),
    /// Percentage.
    Percent(u8),
    /// Bounded sound descriptor.
    Sound {
        /// Allowed cue.
        cue: SoundDescriptor,
        /// Bounded volume.
        volume_percent: u8,
    },
    /// Bounded haptic descriptor.
    Haptic(HapticDescriptor),
}

/// Unit for a length token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LengthUnit {
    /// Logical pixel.
    LogicalPixel,
    /// Percent of a declared semantic bound.
    Percent,
}

/// Complete theme pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemePack {
    /// Stable contract identity.
    pub schema: String,
    /// Stable theme identifier.
    pub id: String,
    /// Semantic version.
    pub version: String,
    /// Required environment metadata.
    pub environments: BTreeMap<ColorEnvironment, EnvironmentSpec>,
    /// Complete semantic token map for each environment.
    pub semantic_tokens: BTreeMap<ColorEnvironment, BTreeMap<String, TokenValue>>,
    /// Bounded density overrides for spacing roles.
    pub density_tokens: BTreeMap<Density, BTreeMap<String, TokenValue>>,
    /// Semantic aliases; targets must themselves be semantic roles.
    pub aliases: BTreeMap<String, String>,
    /// Modality, text, motion, sound, and haptic support.
    pub preferences: Preferences,
}

/// Theme validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeError {
    /// Contract identity was not exactly `aos.theme/1`.
    Schema,
    /// Theme id was absent, oversized, or outside the portable grammar.
    Id,
    /// Semantic version was absent or malformed.
    Version,
    /// One or more required environments were absent.
    Environment,
    /// Environment metadata was outside its bounded scale.
    EnvironmentValue,
    /// A required density was absent or incomplete.
    Density,
    /// A required semantic token was absent.
    Token,
    /// A token name or value was outside the bounded semantic API.
    TokenValue,
    /// Semantic aliases were cyclic or pointed outside semantic roles.
    Alias,
    /// Preference coverage was incomplete.
    Preference,
    /// Required contrast failed.
    Accessibility,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Schema => "theme schema must be aos.theme/1",
            Self::Id => "theme id is invalid",
            Self::Version => "theme semantic version is invalid",
            Self::Environment => "theme environment coverage is incomplete",
            Self::EnvironmentValue => "theme environment value is out of bounds",
            Self::Density => "theme density coverage is incomplete",
            Self::Token => "required semantic token is absent",
            Self::TokenValue => "semantic token value is hostile or out of bounds",
            Self::Alias => "semantic token alias is invalid",
            Self::Preference => "theme does not support the required preferences",
            Self::Accessibility => "theme fails required contrast",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ThemeError {}

/// Every stable semantic role in `aos.theme/1`.
pub fn required_theme_roles() -> &'static [&'static str] {
    &[
        "aos.color.background",
        "aos.color.surface",
        "aos.color.text",
        "aos.color.text-muted",
        "aos.color.border",
        "aos.color.focus",
        "aos.color.accent",
        "aos.color.on-accent",
        "aos.color.success",
        "aos.color.warning",
        "aos.color.danger",
        "aos.color.information",
        "aos.color.neutral",
        "aos.color.disabled",
        "aos.color.selected",
        "aos.color.overlay",
        "aos.elevation.level-0",
        "aos.elevation.level-1",
        "aos.elevation.level-2",
        "aos.elevation.level-3",
        "aos.space.1",
        "aos.space.2",
        "aos.space.3",
        "aos.space.4",
        "aos.space.5",
        "aos.space.6",
        "aos.radius.control",
        "aos.radius.surface",
        "aos.typography.caption",
        "aos.typography.body",
        "aos.typography.title",
        "aos.typography.display",
        "aos.motion.fast",
        "aos.motion.normal",
        "aos.motion.slow",
        "aos.sound.default",
        "aos.haptic.default",
    ]
}

/// Complete built-in theme registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeRegistry {
    packs: [ThemePack; BUILT_IN_THEME_COUNT],
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeRegistry {
    /// Construct and validate the two built-in packs.
    pub fn new() -> Self {
        let packs = [ThemePack::fieldglass(), ThemePack::paper_signal()];
        for pack in &packs {
            pack.validate()
                .expect("built-in theme packs are valid by construction");
        }
        Self { packs }
    }

    /// Built-in packs.
    pub fn packs(&self) -> &[ThemePack] {
        &self.packs
    }

    fn exact_match(
        &self,
        id: &str,
        version: [u64; 3],
        environment: ColorEnvironment,
        density: Density,
        role: &str,
    ) -> Option<ResolvedToken> {
        let pack = self
            .packs()
            .iter()
            .find(|pack| pack.id == id && parse_version(&pack.version) == Some(version))?;
        let value = pack.resolved_token(environment, density, role)?;
        Some(ResolvedToken {
            stage: FallbackStage::ExactTheme,
            theme_id: pack.id.clone(),
            theme_version: pack.version.clone(),
            value,
        })
    }

    fn compatible_match(
        &self,
        id: &str,
        version: [u64; 3],
        environment: ColorEnvironment,
        density: Density,
        role: &str,
    ) -> Option<ResolvedToken> {
        let mut compatible: Option<(&ThemePack, [u64; 3])> = None;
        for pack in self.packs() {
            let Some(found) = parse_version(&pack.version)
                .filter(|found| pack.id == id && found[0] == version[0] && *found <= version)
            else {
                continue;
            };
            if compatible.is_none_or(|(_, best)| found > best) {
                compatible = Some((pack, found));
            }
        }
        let (pack, found) = compatible?;
        let value = pack.resolved_token(environment, density, role)?;
        Some(ResolvedToken {
            stage: FallbackStage::CompatibleLatestMinor,
            theme_id: pack.id.clone(),
            theme_version: pack_version(found),
            value,
        })
    }

    fn built_in_contrast_match(
        &self,
        environment: ColorEnvironment,
        density: Density,
        role: &str,
    ) -> Option<ResolvedToken> {
        let id = FIELDGLASS_THEME_ID;
        let pack = self.packs().iter().find(|pack| pack.id == id)?;
        pack.resolved_token(environment, density, role)
            .map(|value| ResolvedToken {
                stage: FallbackStage::BuiltInContrast,
                theme_id: pack.id.clone(),
                theme_version: pack.version.clone(),
                value,
            })
    }

    /// Resolve one role with the exact, compatible, contrast, then neutral path.
    pub fn resolve(
        &self,
        id: &str,
        version: &str,
        environment: ColorEnvironment,
        density: Density,
        role: &str,
    ) -> Option<ResolvedToken> {
        let requested = parse_version(version)?;
        self.exact_match(id, requested, environment, density, role)
            .or_else(|| self.compatible_match(id, requested, environment, density, role))
            .or_else(|| self.built_in_contrast_match(environment, density, role))
            .or_else(|| {
                Some(ResolvedToken {
                    stage: FallbackStage::Neutral,
                    theme_id: "catalog-neutral".to_owned(),
                    theme_version: "1.0.0".to_owned(),
                    value: NeutralFallback::token(role, environment)?,
                })
            })
    }
}

/// Stage used to resolve a token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackStage {
    /// Exact theme id and version.
    ExactTheme,
    /// Same major-version latest compatible minor.
    CompatibleLatestMinor,
    /// Built-in high-contrast theme.
    BuiltInContrast,
    /// Fail-closed neutral styling.
    Neutral,
}

/// A resolved token and its provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedToken {
    /// Fallback stage used.
    pub stage: FallbackStage,
    /// Theme that supplied the value.
    pub theme_id: String,
    /// Version that supplied the value.
    pub theme_version: String,
    /// Semantic value.
    pub value: TokenValue,
}

/// Neutral fail-closed styling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NeutralFallback;

impl NeutralFallback {
    /// Resolve a required role to a neutral accessible value.
    pub fn token(role: &str, environment: ColorEnvironment) -> Option<TokenValue> {
        let dark = matches!(
            environment,
            ColorEnvironment::Dark | ColorEnvironment::HighContrastDark
        );
        let high_contrast = matches!(
            environment,
            ColorEnvironment::HighContrastLight | ColorEnvironment::HighContrastDark
        );
        let background = if dark {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        };
        let foreground = if dark {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        };
        let muted = if high_contrast {
            foreground
        } else if dark {
            [224, 224, 224, 255]
        } else {
            [48, 48, 48, 255]
        };
        match role {
            "aos.color.background" => Some(TokenValue::Color(background)),
            "aos.color.surface" => Some(TokenValue::Color(background)),
            "aos.color.text" => Some(TokenValue::Color(foreground)),
            "aos.color.text-muted" => Some(TokenValue::Color(muted)),
            "aos.color.border" => Some(TokenValue::Color(foreground)),
            "aos.color.focus" => Some(TokenValue::Color(if dark {
                [120, 200, 255, 255]
            } else {
                [0, 70, 170, 255]
            })),
            "aos.color.accent" => Some(TokenValue::Color(if dark {
                [255, 215, 0, 255]
            } else {
                [0, 0, 139, 255]
            })),
            "aos.color.on-accent" => Some(TokenValue::Color(if dark {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            })),
            "aos.color.overlay" => Some(TokenValue::Color([0, 0, 0, 128])),
            "aos.typography.caption" => Some(TokenValue::Length {
                value: 12,
                unit: LengthUnit::LogicalPixel,
            }),
            "aos.typography.body" => Some(TokenValue::Length {
                value: 16,
                unit: LengthUnit::LogicalPixel,
            }),
            "aos.typography.title" => Some(TokenValue::Length {
                value: 20,
                unit: LengthUnit::LogicalPixel,
            }),
            "aos.typography.display" => Some(TokenValue::Length {
                value: 28,
                unit: LengthUnit::LogicalPixel,
            }),
            role if role.starts_with("aos.elevation.") => Some(TokenValue::Length {
                value: 0,
                unit: LengthUnit::LogicalPixel,
            }),
            role if role.starts_with("aos.space.") => Some(TokenValue::Length {
                value: 8,
                unit: LengthUnit::LogicalPixel,
            }),
            role if role.starts_with("aos.radius.") => Some(TokenValue::Length {
                value: 4,
                unit: LengthUnit::LogicalPixel,
            }),
            role if role.starts_with("aos.motion.") => Some(TokenValue::DurationMs(0)),
            "aos.color.success"
            | "aos.color.warning"
            | "aos.color.danger"
            | "aos.color.information"
            | "aos.color.neutral"
            | "aos.color.disabled"
            | "aos.color.selected" => Some(TokenValue::Color(if dark {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            })),
            "aos.sound.default" => Some(TokenValue::Sound {
                cue: SoundDescriptor::None,
                volume_percent: 0,
            }),
            "aos.haptic.default" => Some(TokenValue::Haptic(HapticDescriptor::None)),
            _ => None,
        }
    }

    /// Build the complete neutral map.
    pub fn tokens(environment: ColorEnvironment) -> BTreeMap<String, TokenValue> {
        required_theme_roles()
            .iter()
            .filter_map(|role| Some(((*role).to_owned(), Self::token(role, environment)?)))
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Palette {
    background: [u8; 4],
    surface: [u8; 4],
    text: [u8; 4],
    text_muted: [u8; 4],
    border: [u8; 4],
    focus: [u8; 4],
    accent: [u8; 4],
    on_accent: [u8; 4],
    success: [u8; 4],
    warning: [u8; 4],
    danger: [u8; 4],
    information: [u8; 4],
    neutral: [u8; 4],
    disabled: [u8; 4],
    selected: [u8; 4],
    overlay: [u8; 4],
}

impl ThemePack {
    /// Complete dark-first built-in theme.
    pub fn fieldglass() -> Self {
        Self::pack(FIELDGLASS_THEME_ID, false)
    }

    /// Complete light-first built-in theme.
    pub fn paper_signal() -> Self {
        Self::pack(PAPER_SIGNAL_THEME_ID, true)
    }

    fn pack(id: &str, light: bool) -> Self {
        let environments = [
            ColorEnvironment::Light,
            ColorEnvironment::Dark,
            ColorEnvironment::HighContrastLight,
            ColorEnvironment::HighContrastDark,
        ]
        .into_iter()
        .map(|environment| {
            (
                environment,
                EnvironmentSpec {
                    material: if matches!(
                        environment,
                        ColorEnvironment::HighContrastLight | ColorEnvironment::HighContrastDark
                    ) {
                        Material::HighContrast
                    } else {
                        Material::Opaque
                    },
                    system_palette: if light {
                        "neutral-light"
                    } else {
                        "neutral-dark"
                    }
                    .to_owned(),
                    display_scale_percent: 100,
                    safe_area: SafeArea {
                        top: 16,
                        leading: 0,
                        trailing: 0,
                        bottom: 24,
                    },
                },
            )
        })
        .collect();
        let semantic_tokens = [
            ColorEnvironment::Light,
            ColorEnvironment::Dark,
            ColorEnvironment::HighContrastLight,
            ColorEnvironment::HighContrastDark,
        ]
        .into_iter()
        .map(|environment| (environment, semantic_tokens(id, environment)))
        .collect();
        let density_tokens = [Density::Compact, Density::Cozy, Density::Spacious]
            .into_iter()
            .map(|density| (density, density_tokens(density)))
            .collect();
        Self {
            schema: THEME_SCHEMA.to_owned(),
            id: id.to_owned(),
            version: "1.0.0".to_owned(),
            environments,
            semantic_tokens,
            density_tokens,
            aliases: BTreeMap::new(),
            preferences: Preferences {
                pointer: true,
                keyboard: true,
                text_scale_percent: SUPPORTED_TEXT_SCALES.to_vec(),
                reduced_motion: true,
                sound: SoundDescriptor::Tap,
                haptic: HapticDescriptor::Light,
            },
        }
    }

    /// Resolve a role after applying density and motion preferences.
    pub fn resolved_tokens(
        &self,
        environment: ColorEnvironment,
        density: Density,
        reduced_motion: bool,
    ) -> Option<BTreeMap<String, TokenValue>> {
        let mut tokens = self.semantic_tokens.get(&environment)?.clone();
        let overrides = self.density_tokens.get(&density)?;
        for (role, value) in overrides {
            tokens.insert(role.clone(), *value);
        }
        if reduced_motion {
            for role in ["aos.motion.fast", "aos.motion.normal", "aos.motion.slow"] {
                tokens.insert(role.to_owned(), TokenValue::DurationMs(0));
            }
        }
        Some(tokens)
    }

    fn resolved_token(
        &self,
        environment: ColorEnvironment,
        density: Density,
        role: &str,
    ) -> Option<TokenValue> {
        if let Some(alias) = self.aliases.get(role)
            && let Some(value) = self.resolved_token(environment, density, alias)
        {
            return Some(value);
        }
        if let Some(value) = self
            .density_tokens
            .get(&density)
            .and_then(|tokens| tokens.get(role))
        {
            return Some(*value);
        }
        self.semantic_tokens
            .get(&environment)
            .and_then(|tokens| tokens.get(role))
            .copied()
    }

    /// Validate exact schema, complete environment/density coverage, and bounds.
    pub fn validate(&self) -> Result<(), ThemeError> {
        if self.schema != THEME_SCHEMA {
            return Err(ThemeError::Schema);
        }
        if !valid_theme_id(&self.id) {
            return Err(ThemeError::Id);
        }
        if parse_version(&self.version).is_none() {
            return Err(ThemeError::Version);
        }
        if self.environments.len() != 4
            || self.semantic_tokens.len() != 4
            || !all_environments(&self.environments)
            || !all_environments(&self.semantic_tokens)
        {
            return Err(ThemeError::Environment);
        }
        for (environment, spec) in &self.environments {
            if !matches!(spec.display_scale_percent, 80..=200)
                || !valid_safe_area(&spec.safe_area)
                || spec.system_palette.is_empty()
                || spec.system_palette.len() > 64
            {
                return Err(ThemeError::EnvironmentValue);
            }
            let tokens = self
                .semantic_tokens
                .get(environment)
                .ok_or(ThemeError::Token)?;
            if tokens.len() != required_theme_roles().len()
                || !required_theme_roles()
                    .iter()
                    .all(|role| tokens.contains_key(*role))
            {
                return Err(ThemeError::Token);
            }
            for (role, value) in tokens {
                validate_token(role, *value).map_err(|_| ThemeError::TokenValue)?;
            }
            self.validate_accessibility(*environment)
                .map_err(|_| ThemeError::Accessibility)?;
        }
        if self.density_tokens.len() != 3
            || !matches!(
                self.density_tokens.keys().collect::<Vec<_>>().as_slice(),
                [Density::Compact, Density::Cozy, Density::Spacious]
            )
        {
            return Err(ThemeError::Density);
        }
        for (density, overrides) in &self.density_tokens {
            let resolved = self
                .resolved_tokens(ColorEnvironment::Light, *density, false)
                .ok_or(ThemeError::Density)?;
            if !required_theme_roles()
                .iter()
                .all(|role| resolved.contains_key(*role))
            {
                return Err(ThemeError::Token);
            }
            for (role, value) in overrides {
                validate_token(role, *value).map_err(|_| ThemeError::TokenValue)?;
            }
        }
        for source in self.aliases.keys() {
            if !required_theme_roles().contains(&source.as_str()) {
                return Err(ThemeError::Alias);
            }
            let mut visited = BTreeSet::from([source.clone()]);
            let mut target = source.clone();
            while let Some(next) = self.aliases.get(&target) {
                if !required_theme_roles().contains(&next.as_str()) || !visited.insert(next.clone())
                {
                    return Err(ThemeError::Alias);
                }
                target = next.clone();
            }
        }
        if !self.preferences.pointer
            || !self.preferences.keyboard
            || !self.preferences.reduced_motion
            || !SUPPORTED_TEXT_SCALES
                .iter()
                .all(|scale| self.preferences.text_scale_percent.contains(scale))
        {
            return Err(ThemeError::Preference);
        }
        Ok(())
    }

    /// WCAG contrast check for all required non-overlay color roles.
    pub fn validate_accessibility(&self, environment: ColorEnvironment) -> Result<(), ThemeError> {
        let tokens = self
            .semantic_tokens
            .get(&environment)
            .ok_or(ThemeError::Token)?;
        let color = |role: &str| match tokens.get(role) {
            Some(TokenValue::Color(color)) => Ok(*color),
            _ => Err(ThemeError::Token),
        };
        let background = color("aos.color.background")?;
        if background[3] != 255 {
            return Err(ThemeError::Accessibility);
        }
        for role in ["aos.color.text", "aos.color.text-muted"] {
            if contrast(color(role)?, background) < 4.5 {
                return Err(ThemeError::Accessibility);
            }
        }
        for role in [
            "aos.color.focus",
            "aos.color.accent",
            "aos.color.success",
            "aos.color.warning",
            "aos.color.danger",
            "aos.color.information",
            "aos.color.neutral",
            "aos.color.disabled",
            "aos.color.selected",
        ] {
            if contrast(color(role)?, background) < 3.0 {
                return Err(ThemeError::Accessibility);
            }
        }
        if contrast(color("aos.color.on-accent")?, color("aos.color.accent")?) < 4.5 {
            return Err(ThemeError::Accessibility);
        }
        Ok(())
    }
}

fn semantic_tokens(id: &str, environment: ColorEnvironment) -> BTreeMap<String, TokenValue> {
    let dark = matches!(
        environment,
        ColorEnvironment::Dark | ColorEnvironment::HighContrastDark
    );
    let high_contrast = matches!(
        environment,
        ColorEnvironment::HighContrastLight | ColorEnvironment::HighContrastDark
    );
    let palette = palette(id, dark, high_contrast);
    let color = |value: [u8; 4]| TokenValue::Color(value);
    let length = |value: u16| TokenValue::Length {
        value,
        unit: LengthUnit::LogicalPixel,
    };
    BTreeMap::from([
        ("aos.color.background".to_owned(), color(palette.background)),
        ("aos.color.surface".to_owned(), color(palette.surface)),
        ("aos.color.text".to_owned(), color(palette.text)),
        ("aos.color.text-muted".to_owned(), color(palette.text_muted)),
        ("aos.color.border".to_owned(), color(palette.border)),
        ("aos.color.focus".to_owned(), color(palette.focus)),
        ("aos.color.accent".to_owned(), color(palette.accent)),
        ("aos.color.on-accent".to_owned(), color(palette.on_accent)),
        ("aos.color.success".to_owned(), color(palette.success)),
        ("aos.color.warning".to_owned(), color(palette.warning)),
        ("aos.color.danger".to_owned(), color(palette.danger)),
        (
            "aos.color.information".to_owned(),
            color(palette.information),
        ),
        ("aos.color.neutral".to_owned(), color(palette.neutral)),
        ("aos.color.disabled".to_owned(), color(palette.disabled)),
        ("aos.color.selected".to_owned(), color(palette.selected)),
        ("aos.color.overlay".to_owned(), color(palette.overlay)),
        ("aos.elevation.level-0".to_owned(), length(0)),
        ("aos.elevation.level-1".to_owned(), length(2)),
        ("aos.elevation.level-2".to_owned(), length(6)),
        ("aos.elevation.level-3".to_owned(), length(12)),
        ("aos.space.1".to_owned(), length(4)),
        ("aos.space.2".to_owned(), length(8)),
        ("aos.space.3".to_owned(), length(12)),
        ("aos.space.4".to_owned(), length(16)),
        ("aos.space.5".to_owned(), length(24)),
        ("aos.space.6".to_owned(), length(32)),
        ("aos.radius.control".to_owned(), length(8)),
        ("aos.radius.surface".to_owned(), length(12)),
        ("aos.typography.caption".to_owned(), length(12)),
        ("aos.typography.body".to_owned(), length(16)),
        ("aos.typography.title".to_owned(), length(20)),
        ("aos.typography.display".to_owned(), length(28)),
        ("aos.motion.fast".to_owned(), TokenValue::DurationMs(90)),
        ("aos.motion.normal".to_owned(), TokenValue::DurationMs(150)),
        ("aos.motion.slow".to_owned(), TokenValue::DurationMs(240)),
        (
            "aos.sound.default".to_owned(),
            TokenValue::Sound {
                cue: SoundDescriptor::Tap,
                volume_percent: 70,
            },
        ),
        (
            "aos.haptic.default".to_owned(),
            TokenValue::Haptic(HapticDescriptor::Light),
        ),
    ])
}

fn palette(id: &str, dark: bool, high_contrast: bool) -> Palette {
    let transparent_black = [0, 0, 0, 64];
    if id == FIELDGLASS_THEME_ID {
        if dark && high_contrast {
            return Palette {
                background: [0, 0, 0, 255],
                surface: [0, 0, 0, 255],
                text: [255, 255, 255, 255],
                text_muted: [235, 235, 235, 255],
                border: [255, 255, 255, 255],
                focus: [120, 200, 255, 255],
                accent: [255, 215, 0, 255],
                on_accent: [0, 0, 0, 255],
                success: [110, 235, 180, 255],
                warning: [255, 200, 80, 255],
                danger: [255, 130, 150, 255],
                information: [140, 200, 255, 255],
                neutral: [240, 240, 240, 255],
                disabled: [180, 180, 180, 255],
                selected: [255, 225, 120, 255],
                overlay: [0, 0, 0, 128],
            };
        }
        if !dark && high_contrast {
            return Palette {
                background: [255, 255, 255, 255],
                surface: [255, 255, 255, 255],
                text: [0, 0, 0, 255],
                text_muted: [25, 25, 25, 255],
                border: [0, 0, 0, 255],
                focus: [0, 70, 170, 255],
                accent: [0, 0, 139, 255],
                on_accent: [255, 255, 255, 255],
                success: [0, 95, 65, 255],
                warning: [125, 75, 0, 255],
                danger: [145, 20, 40, 255],
                information: [0, 70, 155, 255],
                neutral: [25, 25, 25, 255],
                disabled: [90, 90, 95, 255],
                selected: [0, 0, 120, 255],
                overlay: transparent_black,
            };
        }
        if dark {
            return Palette {
                background: [11, 12, 15, 255],
                surface: [21, 22, 28, 255],
                text: [237, 238, 243, 255],
                text_muted: [195, 196, 206, 255],
                border: [82, 84, 96, 255],
                focus: [120, 190, 255, 255],
                accent: [171, 139, 255, 255],
                on_accent: [14, 10, 28, 255],
                success: [100, 220, 165, 255],
                warning: [245, 190, 95, 255],
                danger: [245, 125, 145, 255],
                information: [130, 190, 255, 255],
                neutral: [205, 206, 214, 255],
                disabled: [165, 165, 175, 255],
                selected: [205, 180, 255, 255],
                overlay: [0, 0, 0, 128],
            };
        }
        return Palette {
            background: [255, 255, 255, 255],
            surface: [248, 249, 252, 255],
            text: [22, 23, 28, 255],
            text_muted: [64, 65, 75, 255],
            border: [115, 116, 128, 255],
            focus: [0, 90, 190, 255],
            accent: [60, 40, 180, 255],
            on_accent: [255, 255, 255, 255],
            success: [15, 110, 78, 255],
            warning: [145, 87, 4, 255],
            danger: [170, 32, 52, 255],
            information: [8, 80, 155, 255],
            neutral: [65, 66, 76, 255],
            disabled: [110, 110, 118, 255],
            selected: [50, 30, 160, 255],
            overlay: transparent_black,
        };
    }
    if dark && high_contrast {
        return Palette {
            background: [0, 0, 0, 255],
            surface: [0, 0, 0, 255],
            text: [255, 255, 255, 255],
            text_muted: [235, 235, 235, 255],
            border: [255, 255, 255, 255],
            focus: [170, 240, 220, 255],
            accent: [140, 255, 210, 255],
            on_accent: [0, 0, 0, 255],
            success: [140, 255, 210, 255],
            warning: [255, 205, 90, 255],
            danger: [255, 140, 155, 255],
            information: [160, 210, 255, 255],
            neutral: [240, 240, 240, 255],
            disabled: [180, 180, 180, 255],
            selected: [180, 255, 230, 255],
            overlay: [0, 0, 0, 128],
        };
    }
    if !dark && high_contrast {
        return Palette {
            background: [255, 255, 255, 255],
            surface: [255, 255, 255, 255],
            text: [0, 0, 0, 255],
            text_muted: [25, 25, 25, 255],
            border: [0, 0, 0, 255],
            focus: [0, 90, 70, 255],
            accent: [0, 90, 70, 255],
            on_accent: [255, 255, 255, 255],
            success: [0, 100, 75, 255],
            warning: [120, 70, 0, 255],
            danger: [150, 15, 35, 255],
            information: [0, 65, 145, 255],
            neutral: [25, 25, 25, 255],
            disabled: [90, 90, 95, 255],
            selected: [0, 80, 60, 255],
            overlay: transparent_black,
        };
    }
    if dark {
        return Palette {
            background: [14, 15, 18, 255],
            surface: [23, 25, 28, 255],
            text: [240, 241, 244, 255],
            text_muted: [200, 201, 207, 255],
            border: [88, 90, 98, 255],
            focus: [170, 240, 220, 255],
            accent: [120, 225, 190, 255],
            on_accent: [3, 26, 20, 255],
            success: [115, 230, 190, 255],
            warning: [250, 195, 100, 255],
            danger: [250, 130, 148, 255],
            information: [140, 195, 255, 255],
            neutral: [208, 209, 215, 255],
            disabled: [168, 168, 176, 255],
            selected: [170, 250, 220, 255],
            overlay: [0, 0, 0, 128],
        };
    }
    Palette {
        background: [252, 251, 248, 255],
        surface: [255, 255, 255, 255],
        text: [22, 27, 25, 255],
        text_muted: [64, 70, 67, 255],
        border: [112, 118, 114, 255],
        focus: [0, 105, 82, 255],
        accent: [0, 110, 86, 255],
        on_accent: [255, 255, 255, 255],
        success: [10, 110, 85, 255],
        warning: [140, 85, 0, 255],
        danger: [170, 30, 50, 255],
        information: [10, 75, 150, 255],
        neutral: [64, 70, 67, 255],
        disabled: [112, 112, 118, 255],
        selected: [0, 90, 70, 255],
        overlay: transparent_black,
    }
}

fn density_tokens(density: Density) -> BTreeMap<String, TokenValue> {
    let spaces: [u16; 6] = match density {
        Density::Compact => [2, 4, 6, 10, 14, 20],
        Density::Cozy => [4, 8, 12, 16, 24, 32],
        Density::Spacious => [8, 12, 18, 24, 32, 40],
    };
    spaces
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            (
                format!("aos.space.{}", index + 1),
                TokenValue::Length {
                    value,
                    unit: LengthUnit::LogicalPixel,
                },
            )
        })
        .collect()
}

fn validate_token(role: &str, value: TokenValue) -> Result<(), ThemeError> {
    if !valid_role(role) || !required_theme_roles().contains(&role) {
        return Err(ThemeError::TokenValue);
    }
    match value {
        TokenValue::Color(color) => {
            if role == "aos.color.overlay" {
                if color[3] == 0 {
                    return Err(ThemeError::TokenValue);
                }
            } else if color[3] != 255 {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::Length { value, unit } => {
            let approved = if role.starts_with("aos.space.") {
                APPROVED_SPACE.contains(&value)
            } else if role.starts_with("aos.radius.") {
                APPROVED_RADIUS.contains(&value)
            } else if role.starts_with("aos.typography.") {
                matches!(value, 8..=64)
            } else {
                APPROVED_LENGTHS.contains(&value)
            };
            if !approved {
                return Err(ThemeError::TokenValue);
            }
            if unit != LengthUnit::LogicalPixel
                && !(role.starts_with("aos.typography.") && unit == LengthUnit::Percent)
            {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::Ratio(value) => {
            if !APPROVED_RATIO.contains(&value) {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::DurationMs(value) => {
            if !APPROVED_DURATION.contains(&value) {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::Percent(value) => {
            if !matches!(value, 0..=100) || value % 10 != 0 {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::Sound {
            cue,
            volume_percent,
        } => {
            if !matches!(volume_percent, 0..=100) || volume_percent % 5 != 0 {
                return Err(ThemeError::TokenValue);
            }
            if volume_percent == 0 && cue != SoundDescriptor::None {
                return Err(ThemeError::TokenValue);
            }
        }
        TokenValue::Haptic(_) => {}
    }
    Ok(())
}

fn valid_theme_id(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '.')
        })
    {
        return false;
    }
    value.split('.').all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('-')
            && !segment.ends_with('-')
            && !segment.contains("--")
    })
}

fn valid_role(role: &str) -> bool {
    role.starts_with("aos.")
        && role.len() <= 96
        && !role.contains("..")
        && role.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '-')
        })
}

fn all_environments<Value>(map: &BTreeMap<ColorEnvironment, Value>) -> bool {
    [
        ColorEnvironment::Light,
        ColorEnvironment::Dark,
        ColorEnvironment::HighContrastLight,
        ColorEnvironment::HighContrastDark,
    ]
    .into_iter()
    .all(|environment| map.contains_key(&environment))
}

fn valid_safe_area(area: &SafeArea) -> bool {
    area.top <= 64 && area.leading <= 64 && area.trailing <= 64 && area.bottom <= 64
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

fn pack_version(version: [u64; 3]) -> String {
    format!("{}.{}.{}", version[0], version[1], version[2])
}

fn relative_luminance(color: [u8; 4]) -> f32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.212_6 * channel(color[0]) + 0.715_2 * channel(color[1]) + 0.072_2 * channel(color[2])
}

fn contrast(left: [u8; 4], right: [u8; 4]) -> f32 {
    let first = relative_luminance(left);
    let second = relative_luminance(right);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_two_complete_themes() {
        assert_eq!(
            [FIELDGLASS_THEME_ID, PAPER_SIGNAL_THEME_ID],
            ["aos.builtin.fieldglass", "aos.builtin.paper-signal"]
        );
        for pack in [ThemePack::fieldglass(), ThemePack::paper_signal()] {
            pack.validate().expect("theme is complete");
            for environment in [
                ColorEnvironment::Light,
                ColorEnvironment::Dark,
                ColorEnvironment::HighContrastLight,
                ColorEnvironment::HighContrastDark,
            ] {
                for density in [Density::Compact, Density::Cozy, Density::Spacious] {
                    let tokens = pack
                        .resolved_tokens(environment, density, false)
                        .expect("theme tokens");
                    assert_eq!(tokens.len(), required_theme_roles().len());
                }
            }
        }
        let registry = ThemeRegistry::new();
        assert_eq!(registry.packs().len(), BUILT_IN_THEME_COUNT);
    }

    #[test]
    fn hostile_tokens_fail_closed() {
        let mut pack = ThemePack::paper_signal();
        pack.aliases
            .insert("aos.color.accent".to_owned(), "aos.color.text".to_owned());
        assert!(pack.validate().is_ok());

        let mut transparent_required_color = ThemePack::paper_signal();
        transparent_required_color
            .semantic_tokens
            .get_mut(&ColorEnvironment::Light)
            .unwrap()
            .insert(
                "aos.color.text".to_owned(),
                TokenValue::Color([0, 0, 0, 128]),
            );
        assert!(matches!(
            transparent_required_color.validate(),
            Err(ThemeError::TokenValue)
        ));

        let mut infinite = ThemePack::paper_signal();
        infinite
            .density_tokens
            .get_mut(&Density::Cozy)
            .unwrap()
            .insert("aos.space.1".to_owned(), TokenValue::Ratio(u16::MAX));
        assert!(matches!(infinite.validate(), Err(ThemeError::TokenValue)));

        let json = serde_json::json!({
            "schema": THEME_SCHEMA,
            "id": "hostile",
            "version": "1.0.0",
            "environments": {},
            "semanticTokens": {},
            "densityTokens": {},
            "aliases": {},
            "preferences": {},
            "rawFont": "Public Sans"
        });
        assert!(serde_json::from_value::<ThemePack>(json).is_err());
    }

    #[test]
    fn missing_token_falls_back_through_stages() {
        let registry = ThemeRegistry::new();
        for environment in [
            ColorEnvironment::Light,
            ColorEnvironment::Dark,
            ColorEnvironment::HighContrastLight,
            ColorEnvironment::HighContrastDark,
        ] {
            let fallback = registry
                .resolve(
                    "absent",
                    "1.0.0",
                    environment,
                    Density::Cozy,
                    "aos.color.text",
                )
                .expect("built-in contrast fallback");
            assert_eq!(fallback.stage, FallbackStage::BuiltInContrast);
            assert_eq!(fallback.theme_id, FIELDGLASS_THEME_ID);
        }
        let exact = registry
            .resolve(
                FIELDGLASS_THEME_ID,
                "1.0.0",
                ColorEnvironment::Light,
                Density::Cozy,
                "aos.color.text",
            )
            .unwrap();
        assert_eq!(exact.stage, FallbackStage::ExactTheme);
        let compatible = registry
            .resolve(
                FIELDGLASS_THEME_ID,
                "1.4.0",
                ColorEnvironment::Light,
                Density::Cozy,
                "aos.color.text",
            )
            .unwrap();
        assert_eq!(compatible.stage, FallbackStage::CompatibleLatestMinor);
        let neutral = registry
            .resolve(
                "absent",
                "1.0.0",
                ColorEnvironment::Dark,
                Density::Compact,
                "aos.color.text",
            )
            .unwrap();
        assert_eq!(neutral.stage, FallbackStage::BuiltInContrast);
    }
}
