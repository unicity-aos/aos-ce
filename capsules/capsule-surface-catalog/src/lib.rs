//! Host-independent semantic component catalog and theme-pack contracts.
//!
//! The catalog describes presentation only. It deliberately has no host
//! imports, file access, process access, network access, or capability API.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod a2ui;
pub mod catalog;
pub mod lab;
pub mod theme;
pub mod unknown;

pub use a2ui::{A2uiImportError, DeclaredLossMapping, map_a2ui_component};
pub use catalog::{
    CATALOG_SCHEMA, Catalog, CatalogError, CatalogPrimitive, ChildrenPolicy, Primitive,
    PrimitiveFamily, PrimitiveRecord, State,
};
pub use lab::{
    Breakpoint, COMPACT_BREAKPOINT_REM, DESKTOP_BREAKPOINT_REM, InputModality, LabDimension,
    LabError, LabInstance, RecipeFixture, Scenario, ThemeLab, instantiate, scenario_count,
    scenarios, verification_lab,
};
pub use theme::{
    BUILT_IN_THEME_COUNT, ColorEnvironment, Density, EnvironmentSpec, FIELDGLASS_THEME_ID,
    FallbackStage, Material, NeutralFallback, PAPER_SIGNAL_THEME_ID, Preferences, ResolvedToken,
    SafeArea, ThemeError, ThemePack, ThemeRegistry, TokenValue, required_theme_roles,
};
pub use unknown::{
    UnknownComponent, UnknownComponentDocument, UnknownComponentError, UnknownFallback,
    validate_unknown_component,
};
