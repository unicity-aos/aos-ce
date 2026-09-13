import Foundation

/// Fixed AOS product strings. This is not a Distro theme or branding framework.
public enum ProductIdentity: Sendable {
    public static let name = "AOS"
    public static let bundleIdentifier = "ai.unicity.aos.tray"
    public static let statusItemTitle = "AOS"
    public static let disconnectedLabel = "DISCONNECTED"
    public static let demoBanner =
        "DEMO FIXTURE — not connected to AOS. Decisions change this fixture only."
    public static let disconnectedExplanation =
        "No runtime connection. Capsule inventory is unavailable. This tray does not invent live state."
    public static let demoExplanation =
        "Showing an in-memory fixture. Pregranted capsules are listed without a prompt. No live AOS home is read."
    public static let quitMenuTitle = "Quit AOS Tray"
    public static let quitAccessibility = "Quit AOS Tray. Does not stop the AOS runtime."
    public static let requestsMenuTitle = "Requests"
    public static let capsulesMenuTitle = "Capsules"
}

public struct PrincipalID: Hashable, Codable, Sendable, CustomStringConvertible {
    public var value: String

    public init(_ value: String) {
        self.value = value
    }

    public var description: String { value }
}

public struct CapsuleID: Hashable, Codable, Sendable, CustomStringConvertible {
    public var value: String

    public init(_ value: String) {
        self.value = value
    }

    public var description: String { value }
}

public struct Scope: Hashable, Codable, Sendable, CustomStringConvertible {
    public var value: String

    public init(_ value: String) {
        self.value = value
    }

    public var description: String { value }
}

public enum RuntimeConnection: String, Codable, Sendable {
    case disconnected
}

public enum LaunchMode: String, Codable, Sendable {
    case disconnected
    case demo
}

public enum GrantDisplayState: String, Codable, Sendable, CaseIterable {
    case pending
    case approved
    case denied
    case expired
    case unavailable

    public var label: String {
        switch self {
        case .pending: return "Pending"
        case .approved: return "Approved"
        case .denied: return "Denied"
        case .expired: return "Expired"
        case .unavailable: return "Unavailable"
        }
    }

    public var isActionable: Bool { self == .pending }
}

/// Host and MCP decision verbs already used by AOS. Tray never invents a fifth duration.
public enum DecisionVerb: String, Codable, Sendable, CaseIterable {
    case approve
    case approveOnce = "approve_once"
    case approveSession = "approve_session"
    case approveAlways = "approve_always"
    case deny

    public var label: String {
        switch self {
        case .approve: return "Approve"
        case .approveOnce: return "Approve Once"
        case .approveSession: return "Approve for Session"
        case .approveAlways: return "Always Approve"
        case .deny: return "Deny"
        }
    }
}

public enum ActivationPolicyKind: String, Codable, Sendable {
    case accessory
}

public struct TrayLifecyclePolicy: Equatable, Codable, Sendable {
    public var activationPolicy: ActivationPolicyKind
    public var showsDockIcon: Bool
    public var closeWindowHides: Bool
    public var terminateAfterLastWindowClosed: Bool
    public var quitStopsRuntime: Bool
    public var autoLaunchAtLogin: Bool
    public var readsLiveHome: Bool
    public var collectsCredentials: Bool
    public var performsNetworking: Bool
    public var installsApps: Bool
    public var startsDaemons: Bool

    public static let accessoryShell = TrayLifecyclePolicy(
        activationPolicy: .accessory,
        showsDockIcon: false,
        closeWindowHides: true,
        terminateAfterLastWindowClosed: false,
        quitStopsRuntime: false,
        autoLaunchAtLogin: false,
        readsLiveHome: false,
        collectsCredentials: false,
        performsNetworking: false,
        installsApps: false,
        startsDaemons: false
    )
}

public enum PanelSection: String, Codable, Sendable {
    case requests
    case capsules
}

public struct LaunchArguments: Equatable, Sendable {
    public var mode: LaunchMode
    public var snapshot: Bool
    public var help: Bool

    public init(mode: LaunchMode = .disconnected, snapshot: Bool = false, help: Bool = false) {
        self.mode = mode
        self.snapshot = snapshot
        self.help = help
    }

    public static let helpText = """
    aos-tray — AOS macOS menu bar shell

    Usage:
      aos-tray
      aos-tray --demo
      aos-tray --snapshot
      aos-tray --demo --snapshot
      aos-tray --help

    Default mode is DISCONNECTED. No runtime inventory is invented.
    --demo loads an in-memory fixture with a persistent DEMO banner.
    Demo decisions modify the fixture only.
    This process never stops the AOS runtime, mounts, agents, or MCP sessions.
    """

    public static func parse(_ arguments: [String]) -> Result<LaunchArguments, LaunchParseError> {
        var parsed = LaunchArguments()
        for argument in arguments.dropFirst() {
            switch argument {
            case "--demo":
                parsed.mode = .demo
            case "--snapshot":
                parsed.snapshot = true
            case "--help", "-h":
                parsed.help = true
            default:
                return .failure(.unknownArgument(argument))
            }
        }
        return .success(parsed)
    }
}

public enum LaunchParseError: Equatable, Error, Sendable {
    case unknownArgument(String)

    public var message: String {
        switch self {
        case .unknownArgument(let value):
            return "unknown argument \(value)"
        }
    }
}
