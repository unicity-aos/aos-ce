# Adaptive Shell

`adaptive-shell` is the native Rust reference runner for the AOS Adaptive
Workspace. It opens on an activity, not a chat prompt, and keeps durable
activity/recipe intent separate from an ephemeral retained semantic surface.

The current tranche is intentionally headless. It provides:

- all 62 `aos.catalog/1` semantic component identities;
- bounded, deterministic activity, recipe, surface, and reviewed patch models;
- keyed reconciliation with stable node identity and focus;
- desktop master-plus-stack, grid, single, focus, and phone layout rules;
- Fieldglass dark, light, and high-contrast themes with density, text scale,
  and reduced-motion settings;
- a backend-neutral display list and deterministic snapshot runner;
- an honest `NativePortal::Unavailable` fixture state;
- the shell-chrome Activity Atlas with live deterministic previews, direct
  Master/Stack/Tab/Float/New-activity placement, keyboard equivalents,
  identity-preserving fixture transitions, and one-card phone presentation.

No browser, DOM, WebView, JavaScript, daemon, network, LLM, process, or
authority integration is present. The semantic catalog and layout rules are
independent of CPU architecture and of Linux Realm; they do not assume a
guest ISA. A future winit/GPU adapter can consume the display list without
entering the semantic model.

```text
cargo run -p adaptive-shell -- --headless --fixture desktop
cargo run -p adaptive-shell -- --headless --fixture phone --theme light --json
cargo run -p adaptive-shell -- --headless --fixture theme-lab --density open
```

Atlas pointer, Command-Space, and Super-Space invocations reduce to the same
shell action in tests.  A native compositor adapter must translate the platform's
canonical Super-Space event to `Command::ToggleAtlas(AtlasInvocation::SuperSpace)`;
the headless fixture does not attach a window backend.
