import Foundation

/// Advisory runtime display data, never a grant or a decision target.
public struct ConsentPresentation: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Sendable {
        case capsuleAccess = "capsule_access"
        case actionApproval = "action_approval"
        case ingress

        public var title: String {
            switch self {
            case .capsuleAccess: "Capsule access"
            case .actionApproval: "Action approval"
            case .ingress: "Connection trust"
            }
        }
    }

    public enum Lifetime: String, Codable, Sendable {
        case none, session, durable
        case untilRuntimeRestart = "until_runtime_restart"

        public var explanation: String? {
            switch self {
            case .none: nil
            case .session: "Remembered for this session"
            case .untilRuntimeRestart: "Remembered until the runtime restarts"
            case .durable: "Saved across runtime restarts"
            }
        }
    }

    public let version: Int
    public let kind: Kind
    public let action: String?
    public let resource: String?
    public let reason: String?
    public let principal: String?
    public let capsule: String?
    public let tool: String?
    /// AOS binds these to the original form values before indexing them.
    public let lifetimes: [Lifetime]

    public func isValid(optionCount: Int) -> Bool {
        version == 1 && lifetimes.count == optionCount &&
        [action, resource, reason, principal, capsule, tool].allSatisfy { value in
            guard let value else { return true }
            return !value.isEmpty && value.utf8.count <= PresentationLimits.maxMessageBytes
        }
    }
}
