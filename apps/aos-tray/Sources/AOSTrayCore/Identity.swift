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
    public static let nativeConnectionOff = "OFF"
    public static let nativeConnectionLocalSocket = "LOCAL SOCKET"
    public static let inventoryUnavailable = "UNAVAILABLE"
    public static let runtimeSuppliedCaption = "Runtime-supplied message"
    public static let socketExplanation =
        "Local same-user socket. Inventory is unavailable. This is not human authenticity proof."
    public static let emptyRuntimePromptsText = "No runtime prompts."
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
    case overview
    case requests
    case capsules
}

public struct LaunchArguments: Equatable, Sendable {
    public var mode: LaunchMode
    public var snapshot: Bool
    public var help: Bool
    public var socketPath: String?
    public var aosBinary: String?
    public var aosHome: String?
    public var openOverview = false
    public var nativeInputConfig: String?

    public init(
        mode: LaunchMode = .disconnected,
        snapshot: Bool = false,
        help: Bool = false,
        socketPath: String? = nil
    ) {
        self.mode = mode
        self.snapshot = snapshot
        self.help = help
        self.socketPath = socketPath
        self.aosBinary = nil
        self.aosHome = nil
    }

    public static let helpText = """
    aos-tray — AOS macOS menu bar shell

    Usage:
      aos-tray
      aos-tray --demo
      aos-tray --snapshot
      aos-tray --demo --snapshot
      aos-tray --socket PATH
      aos-tray --aos-binary ABSOLUTE_PATH --aos-home ABSOLUTE_PATH
      aos-tray --overview
      aos-tray --native-input-config ABSOLUTE_PATH
      aos-tray --help

    Default mode is DISCONNECTED. No runtime inventory is invented.
    --demo loads an in-memory fixture with a persistent DEMO banner.
    Demo decisions modify the fixture only.
    --socket PATH listens on an explicit Unix socket for one newline JSON
    request/response per connection. Mutually exclusive with --demo and --snapshot.
    This process never stops the AOS runtime, mounts, agents, or MCP sessions.
    --native-input-config selects a private connection JSON file; no keys are
    discovered or paired. Incompatible with --demo and --snapshot.
    """

    public static func parse(_ arguments: [String]) -> Result<LaunchArguments, LaunchParseError> {
        var parsed = LaunchArguments()
        let args = Array(arguments.dropFirst())
        var index = 0
        while index < args.count {
            let argument = args[index]
            switch argument {
            case "--native-input-config":
                index += 1
                guard parsed.nativeInputConfig == nil, index < args.count, args[index].hasPrefix("/") else {
                    return .failure(.unknownArgument("--native-input-config requires one absolute path"))
                }
                parsed.nativeInputConfig = args[index]
            case "--overview": parsed.openOverview = true
            case "--aos-binary", "--aos-home":
                index += 1
                guard index < args.count, args[index].hasPrefix("/") else {
                    return .failure(.unknownArgument("\(argument) requires an absolute path"))
                }
                if argument == "--aos-binary" { parsed.aosBinary = args[index] }
                else { parsed.aosHome = args[index] }
            case "--demo":
                if parsed.socketPath != nil {
                    return .failure(.conflictingModes)
                }
                parsed.mode = .demo
            case "--snapshot":
                if parsed.socketPath != nil {
                    return .failure(.conflictingModes)
                }
                parsed.snapshot = true
            case "--socket":
                if parsed.mode == .demo || parsed.snapshot {
                    return .failure(.conflictingModes)
                }
                index += 1
                guard index < args.count else {
                    return .failure(.missingSocketPath)
                }
                let path = args[index]
                if path.hasPrefix("--") {
                    return .failure(.missingSocketPath)
                }
                parsed.socketPath = path
            case "--help", "-h":
                parsed.help = true
            default:
                return .failure(.unknownArgument(argument))
            }
            index += 1
        }
        if (parsed.aosBinary == nil) != (parsed.aosHome == nil) ||
            (parsed.nativeInputConfig != nil && (parsed.mode == .demo || parsed.snapshot)) ||
            (parsed.aosBinary != nil && (parsed.mode == .demo || parsed.snapshot)) {
            return .failure(.conflictingModes)
        }
        return .success(parsed)
    }
}

public enum LaunchParseError: Equatable, Error, Sendable {
    case unknownArgument(String)
    case missingSocketPath
    case conflictingModes

    public var message: String {
        switch self {
        case .unknownArgument(let value):
            return "unknown argument \(value)"
        case .missingSocketPath:
            return "--socket requires an explicit PATH"
        case .conflictingModes:
            return "--socket is mutually exclusive with --demo and --snapshot"
        }
    }
}
