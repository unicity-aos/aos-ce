import Foundation

public struct TrayStore: Equatable, Sendable {
    public var mode: LaunchMode
    public var connection: RuntimeConnection
    public var capsules: [CapsuleRecord]
    public var requests: [PermissionRequest]
    public var lifecycle: TrayLifecyclePolicy

    public init(
        mode: LaunchMode,
        connection: RuntimeConnection,
        capsules: [CapsuleRecord],
        requests: [PermissionRequest],
        lifecycle: TrayLifecyclePolicy = .accessoryShell
    ) {
        self.mode = mode
        self.connection = connection
        self.capsules = capsules
        self.requests = requests
        self.lifecycle = lifecycle
    }

    public static func disconnected() -> TrayStore {
        TrayStore(
            mode: .disconnected,
            connection: .disconnected,
            capsules: [],
            requests: []
        )
    }

    public static func demo(_ fixture: DemoFixture = .standard) -> TrayStore {
        TrayStore(
            mode: .demo,
            connection: .disconnected,
            capsules: fixture.capsules,
            requests: fixture.requests
        )
    }

    public static func make(mode: LaunchMode) -> TrayStore {
        switch mode {
        case .disconnected:
            return .disconnected()
        case .demo:
            return .demo()
        }
    }

    public var isDemo: Bool { mode == .demo }

    @discardableResult
    public mutating func applyDecision(
        requestID: String,
        decision: DecisionVerb
    ) -> Result<PermissionRequest, TrayError> {
        guard isDemo else {
            return .failure(.disconnected)
        }
        guard let index = requests.firstIndex(where: { $0.id == requestID }) else {
            return .failure(.unknownRequest(requestID))
        }
        let current = requests[index]
        guard current.state.isActionable else {
            return .failure(.notActionable(current.state))
        }
        guard current.accepts(decision) else {
            return .failure(.unsupportedDecision(decision))
        }

        var updated = current
        updated.appliedDecision = decision
        updated.state = decision == .deny ? .denied : .approved
        requests[index] = updated
        return .success(updated)
    }
}
