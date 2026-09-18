# AOS 2026.9.3

AOS 2026.9.3 is a usability and upgrade patch following 2026.9.2. It makes the
native Command Center discoverable from ordinary AOS start, restart, and MCP
host journeys; adds private-input and multi-round interaction handling; and
repairs legacy private-tree permissions during upgrades from every published
pre-2026.9 AOS release.

The release also binds approvals and MCP results to the requesting identity,
improves first-run and upgrade progress, refreshes the authenticated Community
capsule selection, and makes filesystem mounting available through the macOS
Command Center. The native Command Center remains a macOS feature in this
release; Linux and Windows continue to use their existing CLI/runtime surfaces.

The final release commit must pin the exact published Astrid 2026.9.3 tag,
source commit, release metadata assets, and BLAKE3 digests. Those values are not
guessed from an untagged candidate. GNU and musl Linux on x86_64 and ARM64 plus
macOS on Intel and Apple Silicon remain the intended runtime matrix. Windows
archives remain outside this release.

Before tagging, exercise the composed candidate through:

- fresh Community installation and restart;
- upgrades from published AOS 2026.1.1, 2026.1.2, 2026.1.3, and 2026.9.2;
- generated Codex, Claude, and Grok host launchers;
- MCP initialization, discovery, elicitation, and a real tool invocation;
- clean stop with durable state reduced to the volume contract; and
- authenticated GNU, musl, and Darwin package selection.

The pre-tag journey may use the approved disposable QA signing identity while
preserving the production verification logic. Production signing identities
and published artifact bytes are verified by the separately authorized release
workflow. Merge of the preparation PR does not authorize tagging, publication,
or channel promotion.
