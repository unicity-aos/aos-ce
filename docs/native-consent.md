# AOS native consent

## Pending approvals in the terminal-console branch

The sections below also contain the original integration plan. This branch now
implements a session-local pending-result adapter for modern `input_required`
responses when `aos mcp serve` uses a local Unix presentation socket. It does not
change protocol negotiation or claim background support for older clients.

The initial tool result says `awaiting_approval` and names `aos_approval_status`.
The agent should tell the user to review the request in the AOS app or
`aos console`, then check status after the user responds—not continuously poll.
Decisions remain on that local surface. The status tool accepts only an opaque
approval ID: it cannot approve, accept secrets, or repeat an operation.

The bridge passes the local decision to Astrid using its original opaque
continuation and retains the original runtime result. `completed` means an
outcome exists, not that the action succeeded; callers must inspect that outcome.
Duplicate status reads do not execute the action again. Original output schemas
remain advertised alongside the bridge's pending-result shape.

Limits are explicit:

- Local decision time is capped at 240 seconds, below Astrid's five-minute
  continuation lifetime. Late decisions do not resume expired requests.
- Results belong to this MCP connection, not durable storage. After disconnect,
  an unknown approval ID is not evidence that the action never happened. Do not
  automatically repeat it.
- Completed outcomes already read may be evicted when the bounded table fills.
  Observed expired/unavailable entries are reclaimable after their presenter
  worker exits; a still-blocked presenter cannot be evicted to spawn more workers.
  Unread outcomes and active calls are not evicted to make room. An exhausted
  table refuses new calls before starting them.
- Retained outcomes are bounded to 1 MiB. Larger outcomes produce an explicit
  retrieval error, never a retry of the action.
- Older blocking elicitation, platform-only presenters, private input and HTTP
  are unchanged. A local pending result does not guarantee a particular agent
  host will proactively display a message.

Regression coverage includes an actual `aos mcp serve` subprocess with a
controlled runtime and secure local socket. That proves the bridge exchange;
it is not a live Codex/Claude/Grok or real capsule sign-off.

## Current boundary

`apps/aos-tray` is an AOS-specific macOS menu-bar shell. It is not the AstridFS
container app. It must not alter the signed filesystem bundle, start a runtime on
launch, stop agents on quit, or read an installed home merely to show its window.

The initial shell is disconnected. `--demo` supplies labeled, in-memory fixtures;
fixture decisions are not runtime grants. Build and model tests do not establish
that a real approval, secret entry or installed-capsule query has worked.

## Reuse the product interaction bridge

AOS already owns `aos mcp serve --interaction native` and a constrained
`Presenter` interface in `crates/unicity-aos-bootstrap/src/mcp/interaction.rs`.
Connect the native UI there rather than subscribing a second client to answer
MCP-owned requests directly on Astrid's approval bus.

The existing broker has two different lifecycles:

- A missing capsule grant drops the original invocation. Grant, then retry is
  intentional: there is no suspended operation to resume.
- A capability approval may suspend an operation. The broker must restore its
  result subscription before forwarding the decision. Answering the bus outside
  that path can leave the original tool result undelivered.

Native replies must return to the runtime that issued the JSON-RPC elicitation,
not to host stdout. Keep identifiers and result ownership intact.

## Protocol delivery is separate from permission policy

RMCP exposes negotiated protocol versions and capabilities. Astrid implements
legacy `elicitation/create` and modern multi-round-trip `input_required` flows.
The AOS native presenter currently handles the former; integrating the latter is
still required. Modern form capability alone does not imply generic background
jobs, automatic resumption or support in every host.

Native mode chooses the native presenter independently of host form rendering.
Missing host support must not become a user denial when the native route owns the
request. Preserve existing client mode; do not silently downgrade a configured
native route to collecting input through the agent host. Old-client blocking is
an accepted compatibility limit. HTTP does not currently traverse the stdio
presenter and needs a separately demonstrated adapter before claiming parity.

## UI and authority

- Existing distro grants remain prompt-free. Display their origin.
- Display principal, capsule and actual requested scope; label a capsule/agent's
  explanation as its supplied reason, not authoritative policy.
- Show only decisions supported by that request. Do not label a temporary grant
  permanent, and do not offer an approval lifetime the backend cannot enforce.
- Keep distinct request IDs distinct. Visual grouping must not discard requests
  or turn approval of one invocation into approval of another.
- Include principal and scope in inventory row identity.
- Distinguish user denial from cancellation, expiry and unavailable UI. None
  authorizes execution. Late and duplicate answers must not revive a request.
- A native UI is not proof of human presence. Existing same-principal uplink
  authentication does not exclude other host processes with equivalent keys.

Credentials are not part of the initial shell. Later native credential input or
browser authorization must deliver data to the intended secret-store scope and
return status only to MCP. Keep the current presenter's refusal of password,
free-form and URL input until a dedicated path is implemented and verified.

## Bounded next integration

1. Replace the current non-secret presenter with the tray connection, preserving
   request ownership and cancellation. Test against a disposable runtime.
2. Handle actual modern `input_required`/`requestState` exchanges and resume only
   their suspended logical invocation; do not replay a completed side effect.
3. Add real inventory and a separate credential path, with explicit resolver
   authentication and custody. Do not infer those guarantees from the shell.

The first real demonstration must show a pregranted action without a prompt,
a new permission decided in the app, and the resulting tool outcome. It must use
the actual connection, not demo state. Signing, installation, auto-start and
cross-platform UI are not implemented by this shell.

## Development presenter connection

The terminal pending adapter reports the blocked tool name and directions for
the selected presentation surface. Console directions include a shell-quoted
`AOS_HOME` so a second installation is not mistaken for the requesting runtime.
If the MCP server runs remotely, the user must first connect to that machine
using their existing SSH configuration; AOS does not infer an SSH destination.
Custom presenter sockets do not advertise an unrelated terminal console.
Neither tool arguments nor private answers are copied into these directions.
The adapter advertises its own continuation support on the server-facing call;
coding hosts need not supply modern per-call metadata. The host receives ordinary
tool results, while opaque continuation state stays inside the adapter.

On macOS, the MCP entry point requests launch of the installed
`~/Applications/AOS Command Center.app` without blocking MCP stdio. Launch request
is not confirmation that the application opened or displayed an approval. Host
agents still need to surface pending results; an MCP result is not a guaranteed
Codex, Claude, or Grok notification.

The next integration uses an explicitly selected Unix socket, not a second
approval-bus subscriber. `aos mcp serve --interaction native --interaction-socket
PATH` sends a constrained presentation request to `aos-tray --socket PATH`.
These options are under implementation; they are not installed defaults.

Each connection carries one newline-delimited JSON request and response:

```json
{"version":1,"id":"correlation-id","message":"Runtime-supplied explanation","options":[{"label":"Allow"},{"label":"Deny"}],"timeoutSeconds":120}
{"version":1,"id":"correlation-id","selected":0}
```

`selected: null` means cancellation. The client maps an index back to its already
validated decision value; the app does not manufacture grants. A mismatched ID,
invalid selection, malformed frame, disconnect or timeout never means approval.
An explicitly selected socket must not fall back to another presenter.

Frames are bounded to 16,384 bytes, messages to 4,096 UTF-8 bytes, IDs to 128
bytes, choices to one through four and deadlines to one through 300 seconds.
The initial client deadline is 120 seconds. The app must expire abandoned prompts
and keep simultaneous connections independent.

The endpoint lives in an existing user-owned private directory (0700), with socket
mode 0600 and same-user peer checks. Reject symlink path components and preexisting
endpoints; cleanup must remove only the listener's own endpoint. This establishes
a same-user local connection, not proof that a human or trusted application sent
the request. The UI labels the supplied explanation and does not invent principal,
capsule or scope identities that are absent from this presentation contract.

This connection does not carry credentials or free-form input. Inventory remains
unconnected until its real query path is implemented. Disposable-socket tests and
an actual UI decision are separate evidence requirements.
