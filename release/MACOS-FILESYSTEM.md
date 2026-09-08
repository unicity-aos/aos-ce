# macOS filesystem installation

The AOS Darwin archive carries the signed, notarized AstridFS.app and its
co-versioned companion from the authenticated Astrid archive. App contents,
including the extension identity and signatures, are copied unchanged.

The common installer retains these under the immutable release directory and
uses `runtime/bin/macos/aos-filesystem.sh` to install and enable the container
at `/Applications/AOS.app`. The outer installation name is product-owned;
the filesystem remains Astrid. The wrapper honors `ASTRID_FSKIT_APP_DEST`
for an explicit alternative. No icon or Info.plist is patched after signing.
An existing `/Applications/AstridFS.app` is reused instead of creating a second
registration of the same provider. If both default paths exist, setup requests
an explicit selection; it does not remove either app. An existing destination
must pass the upstream signature/identity validator before replacement.

The website base installer and Oracle first-install bootstrap use this same
path. Already-installed AOS versions need an AOS upgrade to receive it.
macOS 26 and extension approval are required. A permission, signature, or
election failure returns a nonzero installer result with explicit retry
commands; the installed runtime is retained. No automatic sudo, Gatekeeper
bypass, or fallback unsigned signing is performed.

The managed app is not yet a general control UI. A future AOS command center
can own menu-bar/tray status, MCP elicitations and registry management without
changing the filesystem extension's identity or adding UI duties to it.

Fixture tests establish byte/mode preservation and installer dispatch, not
Apple notarization. Production signature and mount checks remain necessary.
