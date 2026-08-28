//! Catalog-owned Theme Lab verification corpus.

use crate::catalog::{Catalog, Primitive, PrimitiveRecord, State};
use crate::theme::{ColorEnvironment, Density, ThemePack};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Exclusive upper bound of the phone breakpoint, in rem.
pub const COMPACT_BREAKPOINT_REM: f64 = 45.0;
/// Exclusive lower bound of the desktop breakpoint, in rem.
pub const DESKTOP_BREAKPOINT_REM: f64 = 72.0;

/// Lab breakpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Breakpoint {
    /// Compact phone viewport.
    Phone,
    /// Intermediate compact breakpoint.
    Compact,
    /// Desktop viewport.
    Desktop,
}

impl Breakpoint {
    /// Resolve the contract breakpoint from a viewport width in rem.
    pub const fn from_viewport_rem(width_rem: f64) -> Self {
        if width_rem < COMPACT_BREAKPOINT_REM {
            Self::Phone
        } else if width_rem < DESKTOP_BREAKPOINT_REM {
            Self::Compact
        } else {
            Self::Desktop
        }
    }
}

/// Lab input modality.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputModality {
    /// Pointer presentation.
    Pointer,
    /// Keyboard-originated presentation.
    Keyboard,
}

/// One complete documented Lab scenario.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Color and contrast environment.
    pub environment: ColorEnvironment,
    /// Density.
    pub density: Density,
    /// Viewport breakpoint.
    pub breakpoint: Breakpoint,
    /// Input modality.
    pub modality: InputModality,
    /// Text scale.
    pub text_scale_percent: u8,
    /// Reduced-motion preference.
    pub reduced_motion: bool,
}

/// Lab dimension documentation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabDimension {
    /// Stable dimension name.
    pub name: String,
    /// Documented values.
    pub values: Vec<String>,
}

/// Catalog-owned verification surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeLab {
    /// Exact catalog.
    pub catalog: Catalog,
    /// Two complete themes.
    pub themes: [ThemePack; 2],
}

/// One semantic Lab instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabInstance {
    /// Primitive under test.
    pub component_id: Primitive,
    /// State under test.
    pub state: State,
    /// Scenario under test.
    pub scenario: Scenario,
    /// Theme used.
    pub theme_id: String,
    /// Theme version used.
    pub theme_version: String,
    /// Primary semantic token used for state presentation.
    pub semantic_token_role: &'static str,
}

/// Fixture recipe identity remains unchanged when presentation changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeFixture {
    /// Stable recipe id.
    pub recipe_id: String,
    /// Stable recipe revision.
    pub revision: u64,
    /// Stable semantic content digest.
    pub semantic_digest: String,
    /// Theme reference attached to the recipe.
    pub theme_id: String,
}

/// Lab construction or instantiation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabError {
    /// State was not documented for the primitive.
    StateUnsupported,
    /// Scenario value was outside the documented dimension.
    ScenarioUnsupported,
    /// Theme lacked coverage for the scenario.
    ThemeIncomplete,
}

impl fmt::Display for LabError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::StateUnsupported => "state is not documented for this primitive",
            Self::ScenarioUnsupported => "scenario is outside the documented Lab matrix",
            Self::ThemeIncomplete => "theme does not cover this scenario",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for LabError {}

impl ThemeLab {
    /// Construct the verification corpus.
    pub fn new() -> Self {
        let registry = crate::theme::ThemeRegistry::new();
        let themes = registry.packs();
        Self {
            catalog: Catalog::v1(),
            themes: [themes[0].clone(), themes[1].clone()],
        }
    }

    /// Documented dimensions.
    pub fn dimensions(&self) -> Vec<LabDimension> {
        vec![
            LabDimension {
                name: "environment".to_owned(),
                values: ["light", "dark", "high-contrast-light", "high-contrast-dark"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            LabDimension {
                name: "density".to_owned(),
                values: ["compact", "cozy", "spacious"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            LabDimension {
                name: "breakpoint".to_owned(),
                values: ["phone", "compact", "desktop"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            LabDimension {
                name: "modality".to_owned(),
                values: ["pointer", "keyboard"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            LabDimension {
                name: "text-scale".to_owned(),
                values: ["90", "100", "118", "200"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            LabDimension {
                name: "reduced-motion".to_owned(),
                values: ["false", "true"]
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
        ]
    }

    /// Switch only the theme reference on a fixture recipe.
    pub fn switch_theme(&self, recipe: &RecipeFixture, theme: &ThemePack) -> RecipeFixture {
        RecipeFixture {
            recipe_id: recipe.recipe_id.clone(),
            revision: recipe.revision,
            semantic_digest: recipe.semantic_digest.clone(),
            theme_id: format!("{}@{}", theme.id, theme.version),
        }
    }
}

impl Default for ThemeLab {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct the canonical verification surface.
pub fn verification_lab() -> ThemeLab {
    ThemeLab::new()
}

/// Canonical fixture recipe used by the independence proof.
pub fn canonical_recipe_fixture() -> RecipeFixture {
    RecipeFixture {
        recipe_id: "fixture/workspace-recipe".to_owned(),
        revision: 1,
        semantic_digest: "fixture-semantic-content".to_owned(),
        theme_id: "aos.builtin.fieldglass@1.0.0".to_owned(),
    }
}

/// Number of complete documented scenarios.
pub const fn scenario_count() -> usize {
    4 * 3 * 3 * 2 * 4 * 2
}

/// Enumerate every documented scenario.
pub fn scenarios() -> Vec<Scenario> {
    (0..scenario_count())
        .map(|index| scenario(index).expect("index is bounded"))
        .collect()
}

/// Return a scenario by canonical cross-product index.
pub fn scenario(index: usize) -> Option<Scenario> {
    if index >= scenario_count() {
        return None;
    }
    let environments = [
        ColorEnvironment::Light,
        ColorEnvironment::Dark,
        ColorEnvironment::HighContrastLight,
        ColorEnvironment::HighContrastDark,
    ];
    let densities = [Density::Compact, Density::Cozy, Density::Spacious];
    let breakpoints = [Breakpoint::Phone, Breakpoint::Compact, Breakpoint::Desktop];
    let modalities = [InputModality::Pointer, InputModality::Keyboard];
    let scales = [90, 100, 118, 200];
    let mut remaining = index;
    let mut next = |bound: usize| {
        let value = remaining % bound;
        remaining /= bound;
        value
    };
    Some(Scenario {
        environment: environments[next(environments.len())],
        density: densities[next(densities.len())],
        breakpoint: breakpoints[next(breakpoints.len())],
        modality: modalities[next(modalities.len())],
        text_scale_percent: scales[next(scales.len())],
        reduced_motion: next(2) == 1,
    })
}

/// Instantiate one semantic primitive in one state, scenario, and theme.
pub fn instantiate(
    record: &PrimitiveRecord,
    state: State,
    scenario: Scenario,
    theme: &ThemePack,
) -> Result<LabInstance, LabError> {
    if !record.states.contains(&state) {
        return Err(LabError::StateUnsupported);
    }
    if !scenario_supported(&scenario) {
        return Err(LabError::ScenarioUnsupported);
    }
    let resolved = theme
        .resolved_tokens(
            scenario.environment,
            scenario.density,
            scenario.reduced_motion,
        )
        .ok_or(LabError::ThemeIncomplete)?;
    let role = token_role(record.id, state);
    if !resolved.contains_key(role) {
        return Err(LabError::ThemeIncomplete);
    }
    Ok(LabInstance {
        component_id: record.id,
        state,
        scenario,
        theme_id: theme.id.clone(),
        theme_version: theme.version.clone(),
        semantic_token_role: role,
    })
}

fn scenario_supported(scenario: &Scenario) -> bool {
    matches!(
        scenario.environment,
        ColorEnvironment::Light
            | ColorEnvironment::Dark
            | ColorEnvironment::HighContrastLight
            | ColorEnvironment::HighContrastDark
    ) && matches!(
        scenario.density,
        Density::Compact | Density::Cozy | Density::Spacious
    ) && matches!(
        scenario.breakpoint,
        Breakpoint::Phone | Breakpoint::Compact | Breakpoint::Desktop
    ) && matches!(
        scenario.modality,
        InputModality::Pointer | InputModality::Keyboard
    ) && matches!(scenario.text_scale_percent, 90 | 100 | 118 | 200)
}

fn token_role(id: Primitive, state: State) -> &'static str {
    if state == State::Disabled {
        return "aos.color.disabled";
    }
    match state {
        State::Error => "aos.color.danger",
        State::Success => "aos.color.success",
        State::Warning => "aos.color.warning",
        State::FocusVisible => "aos.color.focus",
        State::Selected => "aos.color.selected",
        State::Loading | State::Empty => "aos.color.text-muted",
        _ => match id.family() {
            crate::catalog::PrimitiveFamily::Layout => "aos.color.surface",
            crate::catalog::PrimitiveFamily::Content => "aos.color.text",
            crate::catalog::PrimitiveFamily::Input => "aos.color.accent",
            crate::catalog::PrimitiveFamily::Data => "aos.color.text",
            crate::catalog::PrimitiveFamily::Navigation => "aos.color.accent",
            crate::catalog::PrimitiveFamily::Feedback => "aos.color.information",
            crate::catalog::PrimitiveFamily::Permission => "aos.color.focus",
            crate::catalog::PrimitiveFamily::Media => "aos.color.surface",
            crate::catalog::PrimitiveFamily::Canvas => "aos.color.surface",
            crate::catalog::PrimitiveFamily::Terminal => "aos.color.surface",
            crate::catalog::PrimitiveFamily::NativePortal => "aos.color.border",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{BUILT_IN_THEME_COUNT, required_theme_roles};
    use std::collections::BTreeSet;

    #[test]
    fn documents_the_complete_matrix() {
        let lab = ThemeLab::new();
        assert_eq!(lab.catalog.records.len(), 62);
        assert_eq!(lab.themes.len(), BUILT_IN_THEME_COUNT);
        assert_eq!(scenario_count(), 576);
        assert_eq!(scenarios().len(), 576);
        assert_eq!(
            lab.dimensions()
                .into_iter()
                .map(|dimension| dimension.values.len())
                .product::<usize>(),
            576
        );
        assert!(scenario(575).is_some());
        assert!(scenario(768).is_none());
        assert_eq!(
            Breakpoint::from_viewport_rem(44.999_999_999_999),
            Breakpoint::Phone
        );
        assert_eq!(Breakpoint::from_viewport_rem(45.0), Breakpoint::Compact);
        assert_eq!(
            Breakpoint::from_viewport_rem(71.999_999_999_999),
            Breakpoint::Compact
        );
        assert_eq!(Breakpoint::from_viewport_rem(72.0), Breakpoint::Desktop);
    }

    #[test]
    fn instantiates_every_record_state_scenario_and_theme() {
        let lab = ThemeLab::new();
        let scenarios = scenarios();
        let mut observed_records = BTreeSet::new();
        let mut observed_states = BTreeSet::new();
        for theme in &lab.themes {
            for record in &lab.catalog.records {
                for state in record.states.iter().copied() {
                    for scenario in scenarios.iter().copied() {
                        let instance = instantiate(record, state, scenario, theme)
                            .expect("Lab instance is complete");
                        assert_eq!(instance.component_id, record.id);
                        assert_eq!(instance.state, state);
                        assert!(required_theme_roles().contains(&instance.semantic_token_role));
                        observed_records.insert(record.id);
                        observed_states.insert(state);
                    }
                }
            }
        }
        assert_eq!(observed_records.len(), 62);
        assert!(observed_states.contains(&State::FocusVisible));
        assert!(observed_states.contains(&State::Disabled));
    }

    #[test]
    fn theme_change_does_not_change_recipe_identity() {
        let lab = ThemeLab::new();
        let recipe = canonical_recipe_fixture();
        let changed = lab.switch_theme(&recipe, &lab.themes[1]);
        assert_eq!(recipe.recipe_id, changed.recipe_id);
        assert_eq!(recipe.revision, changed.revision);
        assert_eq!(recipe.semantic_digest, changed.semantic_digest);
        assert_ne!(recipe.theme_id, changed.theme_id);
    }
}
