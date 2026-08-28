//! Command-line entry point for the deterministic native shell runner.

use adaptive_shell::fixtures::{FixtureKind, render_fixture};
use adaptive_shell::theme::{Density, TextScale, ThemeConfig, ThemeName};
use std::env;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse(env::args().skip(1))?;
    if options.help {
        print_help();
        return Ok(());
    }
    if !options.headless {
        eprintln!(
            "native window backend is intentionally deferred; run with --headless for the runnable Rust shell"
        );
        return Ok(());
    }
    let snapshot = render_fixture(options.fixture, options.theme)?;
    if options.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        println!("AOS Adaptive Workspace · native Rust headless runner");
        println!(
            "fixture={} viewport={}x{}",
            snapshot.fixture, snapshot.viewport.width, snapshot.viewport.height
        );
        println!(
            "theme={} density={} scale={} reduced_motion={}",
            snapshot.theme, snapshot.density, snapshot.scale_percent, snapshot.reduced_motion
        );
        println!(
            "activity={} recipe_revision={} surfaces={}/{}",
            snapshot.activity_id,
            snapshot.recipe_revision,
            snapshot.visible_surface_count,
            snapshot.surface_count
        );
        println!(
            "display_commands={} semantic_digest={} display_digest={}",
            snapshot.display.commands, snapshot.semantic_digest, snapshot.display.digest
        );
        println!(
            "native_portal={:?} clock_ms={}",
            snapshot.native_portal, snapshot.clock_ms
        );
    }
    Ok(())
}

#[derive(Debug)]
struct Options {
    fixture: FixtureKind,
    headless: bool,
    json: bool,
    help: bool,
    theme: ThemeConfig,
}

impl Options {
    fn parse<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut fixture = FixtureKind::Desktop;
        let mut headless = false;
        let mut json = false;
        let mut help = false;
        let mut theme = ThemeConfig::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--fixture" => {
                    let value = args
                        .next()
                        .ok_or("--fixture needs desktop, phone, or theme-lab")?;
                    fixture = FixtureKind::parse(&value).ok_or_else(|| {
                        format!("unknown fixture `{value}` (expected desktop|phone|theme-lab)")
                    })?;
                }
                "--headless" => headless = true,
                "--json" => json = true,
                "--help" | "-h" => help = true,
                "--theme" => {
                    let value = args
                        .next()
                        .ok_or("--theme needs dark, light, or contrast")?;
                    theme.name = match value.as_str() {
                        "dark" => ThemeName::Dark,
                        "light" => ThemeName::Light,
                        "contrast" | "high-contrast" => ThemeName::Contrast,
                        _ => {
                            return Err(format!(
                                "unknown theme `{value}` (expected dark|light|contrast)"
                            ));
                        }
                    };
                }
                "--density" => {
                    let value = args.next().ok_or("--density needs tight, cozy, or open")?;
                    theme.density = match value.as_str() {
                        "tight" => Density::Tight,
                        "cozy" => Density::Cozy,
                        "open" => Density::Open,
                        _ => {
                            return Err(format!(
                                "unknown density `{value}` (expected tight|cozy|open)"
                            ));
                        }
                    };
                }
                "--scale" => {
                    let value = args.next().ok_or("--scale needs 90, 100, 118, or 200")?;
                    theme.scale = match value.as_str() {
                        "90" | "90%" => TextScale::P90,
                        "100" | "100%" => TextScale::P100,
                        "118" | "118%" => TextScale::P118,
                        "200" | "200%" => TextScale::P200,
                        _ => {
                            return Err(format!(
                                "unknown scale `{value}` (expected 90|100|118|200)"
                            ));
                        }
                    };
                }
                "--reduced-motion" => theme.reduced_motion = true,
                value if value.starts_with('-') => return Err(format!("unknown option `{value}`")),
                value => return Err(format!("unexpected argument `{value}`")),
            }
        }
        Ok(Self {
            fixture,
            headless,
            json,
            help,
            theme,
        })
    }
}

fn print_help() {
    println!(
        "AOS Adaptive Workspace native Rust runner\n\nUsage: adaptive-shell --headless [options]\n\nOptions:\n  --fixture desktop|phone|theme-lab\n  --theme dark|light|contrast\n  --density tight|cozy|open\n  --scale 90|100|118|200\n  --reduced-motion\n  --json\n  --help\n\nThe current tranche is headless by design; no browser, daemon, network, or native portal is assumed. Native compositor adapters must bind canonical Super-Space to the same Atlas action as Command-Space."
    );
}
