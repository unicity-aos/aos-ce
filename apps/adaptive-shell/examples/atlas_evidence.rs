//! Generate deterministic headless Atlas evidence.

use adaptive_shell::fixtures::{Fixture, FixtureKind};
use adaptive_shell::input::Command;
use adaptive_shell::theme::{Density, TextScale, ThemeConfig, ThemeName};
use adaptive_shell::{PlacementOrigin, PlacementTarget};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut desktop = Fixture::new(FixtureKind::Desktop, ThemeConfig::default())?;
    desktop.apply(Command::ToggleAtlas(
        adaptive_shell::AtlasInvocation::Pointer,
    ))?;
    desktop.apply(Command::BeginAtlasPlacement {
        activity_id: "cats".to_owned(),
        origin: PlacementOrigin::Drag,
    })?;
    desktop.apply(Command::HoverAtlasPlacement(PlacementTarget::Float))?;
    desktop.apply(Command::CommitAtlasPlacement)?;
    let desktop_commit = desktop.state.atlas.last_commit.clone().expect("commit");
    println!(
        "{}",
        json!({
            "scenario": "desktop-connected-placement",
            "viewport": [1440, 1000],
            "theme": "dark",
            "invocation": "pointer",
            "commit": desktop_commit,
            "display": desktop.display_list().summary(),
            "snapshot": desktop.snapshot(),
        })
    );

    let mut phone = Fixture::new(
        FixtureKind::Phone,
        ThemeConfig {
            name: ThemeName::Contrast,
            density: Density::Open,
            scale: TextScale::P200,
            reduced_motion: true,
        },
    )?;
    phone.apply(Command::ToggleAtlas(
        adaptive_shell::AtlasInvocation::SuperSpace,
    ))?;
    phone.apply(Command::FocusAtlasTile("native".to_owned()))?;
    phone.apply(Command::BeginAtlasPlacement {
        activity_id: "native".to_owned(),
        origin: PlacementOrigin::Keyboard,
    })?;
    phone.apply(Command::CommitAtlasPlacement)?;
    let phone_commit = phone.state.atlas.last_commit.clone().expect("commit");
    println!(
        "{}",
        json!({
            "scenario": "phone-one-card-new-activity",
            "viewport": [390, 844],
            "theme": "high-contrast-open-200-reduced-motion",
            "invocation": "super-space",
            "commit": phone_commit,
            "display": phone.display_list().summary(),
            "snapshot": phone.snapshot(),
        })
    );
    Ok(())
}
