import Foundation

public enum InventoryIdentity {
    public static func capsule(_ principal: PrincipalID, _ capsule: CapsuleID, _ scope: Scope) -> String {
        [principal.value, capsule.value, scope.value].joined(separator: "\u{1f}")
    }
}

public struct CapsuleRecord: Equatable, Identifiable, Codable, Sendable {
    public var id: String { InventoryIdentity.capsule(principal, capsule, scope) }
    public var principal: PrincipalID
    public var capsule: CapsuleID
    public var scope: Scope
    public var state: GrantDisplayState
    public var pregranted: Bool

    public init(
        principal: PrincipalID,
        capsule: CapsuleID,
        scope: Scope,
        state: GrantDisplayState,
        pregranted: Bool
    ) {
        self.principal = principal
        self.capsule = capsule
        self.scope = scope
        self.state = state
        self.pregranted = pregranted
    }

    public var prompt: Bool { !pregranted && state == .pending }
}

public struct PermissionRequest: Equatable, Identifiable, Codable, Sendable {
    public var id: String
    public var principal: PrincipalID
    public var capsule: CapsuleID
    public var scope: Scope
    public var reason: String
    public var state: GrantDisplayState
    public var supportedDecisions: [DecisionVerb]
    public var appliedDecision: DecisionVerb?

    public init(
        id: String,
        principal: PrincipalID,
        capsule: CapsuleID,
        scope: Scope,
        reason: String,
        state: GrantDisplayState,
        supportedDecisions: [DecisionVerb],
        appliedDecision: DecisionVerb? = nil
    ) {
        self.id = id
        self.principal = principal
        self.capsule = capsule
        self.scope = scope
        self.reason = reason
        self.state = state
        self.supportedDecisions = Self.unique(supportedDecisions)
        self.appliedDecision = appliedDecision
    }

    public var prompt: Bool { state == .pending }

    public var visibleDecisions: [DecisionVerb] {
        state.isActionable ? supportedDecisions : []
    }

    public func accepts(_ decision: DecisionVerb) -> Bool {
        supportedDecisions.contains(decision)
    }

    private static func unique(_ decisions: [DecisionVerb]) -> [DecisionVerb] {
        var seen = Set<DecisionVerb>()
        var ordered: [DecisionVerb] = []
        for decision in decisions {
            if seen.insert(decision).inserted {
                ordered.append(decision)
            }
        }
        return ordered
    }
}

public struct DemoFixture: Equatable, Sendable {
    public var capsules: [CapsuleRecord]
    public var requests: [PermissionRequest]

    public init(capsules: [CapsuleRecord], requests: [PermissionRequest]) {
        self.capsules = capsules
        self.requests = requests
    }

    /// Deterministic in-memory CE-shaped fixture. Not a live home or Distro read.
    public static let standard = DemoFixture(
        capsules: [
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-fs"),
                scope: Scope("fs.read"),
                state: .approved,
                pregranted: true
            ),
            CapsuleRecord(
                principal: PrincipalID("other-principal"),
                capsule: CapsuleID("aos-fs"),
                scope: Scope("fs.read"),
                state: .approved,
                pregranted: true
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-fs"),
                scope: Scope("fs.write"),
                state: .pending,
                pregranted: false
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-shell"),
                scope: Scope("shell.exec"),
                state: .approved,
                pregranted: true
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-http"),
                scope: Scope("http.fetch"),
                state: .pending,
                pregranted: false
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-memory"),
                scope: Scope("memory.write"),
                state: .pending,
                pregranted: false
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-skills"),
                scope: Scope("skills.load"),
                state: .denied,
                pregranted: false
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-session"),
                scope: Scope("session.attach"),
                state: .expired,
                pregranted: false
            ),
            CapsuleRecord(
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-mcp"),
                scope: Scope("mcp.serve"),
                state: .unavailable,
                pregranted: false
            ),
        ],
        requests: [
            PermissionRequest(
                id: "req-http-1",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-http"),
                scope: Scope("http.fetch"),
                reason: "Fetch a fixture URL",
                state: .pending,
                supportedDecisions: [.approveOnce, .deny]
            ),
            PermissionRequest(
                id: "req-http-2",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-http"),
                scope: Scope("http.fetch"),
                reason: "Fetch a second fixture URL",
                state: .pending,
                supportedDecisions: [.approveOnce, .deny]
            ),
            PermissionRequest(
                id: "req-memory-1",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-memory"),
                scope: Scope("memory.write"),
                reason: "Persist a fixture note",
                state: .pending,
                supportedDecisions: [.approve, .approveSession, .deny]
            ),
            PermissionRequest(
                id: "req-skills-1",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-skills"),
                scope: Scope("skills.load"),
                reason: "Load a fixture skill",
                state: .denied,
                supportedDecisions: [.approveOnce, .deny]
            ),
            PermissionRequest(
                id: "req-session-1",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-session"),
                scope: Scope("session.attach"),
                reason: "Attach a fixture session",
                state: .expired,
                supportedDecisions: [.approveSession, .deny]
            ),
            PermissionRequest(
                id: "req-mcp-1",
                principal: PrincipalID("demo-principal"),
                capsule: CapsuleID("aos-mcp"),
                scope: Scope("mcp.serve"),
                reason: "Serve fixture MCP",
                state: .unavailable,
                supportedDecisions: [.approve, .deny]
            ),
        ]
    )
}

public enum TrayError: Equatable, Error, Sendable {
    case disconnected
    case unknownRequest(String)
    case unsupportedDecision(DecisionVerb)
    case notActionable(GrantDisplayState)

    public var message: String {
        switch self {
        case .disconnected:
            return "No runtime connection. Decisions are not forwarded."
        case .unknownRequest(let id):
            return "Unknown request \(id)."
        case .unsupportedDecision(let verb):
            return "Decision \(verb.rawValue) is not supported for this request."
        case .notActionable(let state):
            return "Request is \(state.rawValue) and cannot accept a decision."
        }
    }
}
