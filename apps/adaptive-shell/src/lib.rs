//! A native, renderer-neutral reference shell for the AOS Adaptive Workspace.
//!
//! This crate deliberately stops at the user-space semantic and headless
//! rendering boundary.  It does not start a daemon, spawn a process, talk to
//! Astrid, or pretend that a native portal is available.  A future windowing
//! backend can consume [`render::DisplayList`] without changing the semantic
//! model in this crate.

#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs)]

pub mod activity;
pub mod components;
pub mod fixtures;
pub mod input;
pub mod layout;
pub mod reconcile;
pub mod render;
pub mod theme;

pub use activity::{
    Activity, ActivityRegistry, OpaqueOwnerRef, Patch, PatchError, PatchOp, PatchOutcome, Recipe,
    RecipeStore, Surface, SurfaceId,
};
pub use components::{ComponentKind, NodeId, SemanticNode, StateSet};
pub use fixtures::{Fixture, FixtureKind, Snapshot};
pub use input::{Command, ShellState};
pub use layout::{LayoutMode, LayoutPlan, LayoutPolicy, Viewport};
pub use theme::{Density, Theme, ThemeConfig, ThemeName};
