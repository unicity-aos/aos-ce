import Foundation
import Darwin

/// Local-personal native-input first-run setup. Discovery is not acting,
/// approval, or secret authority. A completed receipt is not a live connection.
public enum NativeRuntimeSetupAction: Equatable, Sendable {
    case run(String)
    case unconfirmed
    case emptyDirectory
    case unsupported
    case failed
    case staleIdentifier(String)
    case invalidIdentifier
    case discoveryUnavailable
}

public enum NativeRuntimeSetupError: Equatable, Error, Sendable {
    case unconfirmed
    case unsupported
    case existing
    case failed
    case unavailable
    case invalidPrincipal
}

public struct NativeRuntimeSetupReceipt: Equatable, Sendable, Decodable {
    public let scope: String
    public let authority: String
    public let principal: String
    public let connectionPath: String
    public let restartRequired: Bool
    public let connected: Bool
}

public enum NativeRuntimeSetupCopy {
    public static let localPersonalExplanation =
        "This enrolls this Mac as a local-personal native-input device for a principal you own. The list is discovery only. It is not hosted acting, approval, or secret authority."
    public static let enrollConfirm =
        "Enroll this Mac as the aos-tray device for the selected principal."
    public static let routeConfirm =
        "Route native-input requests for that principal to this device. This does not grant other permissions."
    public static let restartNeededTitle = "Restart AOS Tray to connect"
    public static let restartNeeded =
        "Native input is set up for this machine. This session was not restarted and is not connected. Quit and reopen AOS Tray to use the existing enrollment."
    public static let demoUnavailable =
        "Demo mode cannot enroll a native-input device or claim hosted acting. Nothing was changed."
    public static let missingRuntime =
        "Native-input setup needs a local AOS installation. Nothing was enrolled."
    public static let unsupported =
        "This AOS runtime does not support local-personal native-input setup."
    public static let existing =
        "Native input is already enrolled on this machine. Existing connection, key, and routing were left unchanged."
    public static let failed = "Couldn’t set up native input. Nothing was overwritten."
    public static let unconfirmed = "Confirm device enrollment and operator routing before continuing. Nothing was enrolled."
    public static let cancel = "Setup cancelled. Nothing was enrolled."

    public static func message(for action: NativeRuntimeSetupAction) -> String? {
        switch action {
        case .run: return nil
        case .unconfirmed: return unconfirmed
        case .emptyDirectory: return PrincipalDiscoveryCopy.emptyDirectory
        case .unsupported: return unsupported
        case .failed: return PrincipalDiscoveryCopy.failed
        case .staleIdentifier: return PrincipalDiscoveryCopy.stale
        case .invalidIdentifier: return PrincipalDiscoveryCopy.invalid
        case .discoveryUnavailable: return unsupported
        }
    }

    public static func message(for error: NativeRuntimeSetupError) -> String {
        switch error {
        case .unconfirmed: return unconfirmed
        case .unsupported: return unsupported
        case .existing: return existing
        case .failed: return failed
        case .unavailable: return missingRuntime
        case .invalidPrincipal: return PrincipalDiscoveryCopy.invalid
        }
    }
}

/// Runs only an explicitly selected AOS executable's local-personal native-setup command.
public enum NativeRuntimeSetup {
    public static func defaultConnectionPath(home: String) -> String? {
        guard let home = realHome(home) else { return nil }
        return home + "/native-input/connection.json"
    }

    public static func adoptableConnectionPath(home: String) -> String? {
        guard let path = defaultConnectionPath(home: home),
              (try? NativeRuntimeConfiguration.load(path: path)) != nil else {
            return nil
        }
        return path
    }

    /// Existing session or default enrollment is left unchanged. Discovery is
    /// not authority to replace it.
    public static func existingEnrollment(home: String?, configPath: String?) -> Bool {
        if let configPath, configPath.hasPrefix("/") { return true }
        guard let home else { return false }
        return adoptableConnectionPath(home: home) != nil
    }

    public static func commandArguments(principal: String) -> [String] {
        ["native-setup", "--principal", principal, "--confirm-enroll", "--confirm-route", "--json"]
    }

    public static func evaluate(
        selected: String?,
        discovery: PrincipalDiscovery,
        confirmEnroll: Bool,
        confirmRoute: Bool
    ) -> NativeRuntimeSetupAction {
        switch PrincipalPicker.choose(selected: selected, from: discovery) {
        case .emptyDirectory:
            return .emptyDirectory
        case .staleIdentifier(let value):
            return .staleIdentifier(value)
        case .invalidIdentifier:
            return .invalidIdentifier
        case .discoveryUnavailable:
            return discovery == .unsupported ? .unsupported : .failed
        case .selected(let principal):
            guard confirmEnroll, confirmRoute else { return .unconfirmed }
            return .run(principal.id)
        }
    }

    public static func run(
        binary: String,
        home: String,
        principal: String,
        confirmEnroll: Bool,
        confirmRoute: Bool
    ) async throws -> NativeRuntimeSetupReceipt {
        try await Task.detached {
            try runBlocking(
                binary: binary, home: home, principal: principal,
                confirmEnroll: confirmEnroll, confirmRoute: confirmRoute
            )
        }.value
    }

    static func runBlocking(
        binary: String,
        home: String,
        principal: String,
        confirmEnroll: Bool,
        confirmRoute: Bool,
        timeout: TimeInterval = 60
    ) throws -> NativeRuntimeSetupReceipt {
        guard confirmEnroll, confirmRoute else { throw NativeRuntimeSetupError.unconfirmed }
        guard binary.hasPrefix("/"), timeout > 0, timeout.isFinite else {
            throw NativeRuntimeSetupError.unavailable
        }
        guard OwnedPrincipal.isValidID(principal), principal != "anonymous" else {
            throw NativeRuntimeSetupError.invalidPrincipal
        }
        guard let canonicalHome = realHome(home),
              let expectedConnection = defaultConnectionPath(home: canonicalHome) else {
            throw NativeRuntimeSetupError.failed
        }
        let stdout = Pipe()
        let stderr = Pipe()
        defer {
            try? stdout.fileHandleForReading.close()
            try? stdout.fileHandleForWriting.close()
            try? stderr.fileHandleForReading.close()
            try? stderr.fileHandleForWriting.close()
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = commandArguments(principal: principal)
        var environment = ProcessInfo.processInfo.environment
        environment["AOS_HOME"] = canonicalHome
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = stdout
        process.standardError = stderr
        try process.run()
        defer {
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
        }
        try stdout.fileHandleForWriting.close()
        try stderr.fileHandleForWriting.close()
        let started = ProcessInfo.processInfo.systemUptime
        while process.isRunning {
            guard ProcessInfo.processInfo.systemUptime - started < timeout else {
                throw NativeRuntimeSetupError.failed
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        let out = stdout.fileHandleForReading.readDataToEndOfFile()
        let err = stderr.fileHandleForReading.readDataToEndOfFile()
        guard out.count <= 16_384, err.count <= 16_384 else { throw NativeRuntimeSetupError.failed }
        let stderrText = String(data: err, encoding: .utf8) ?? ""
        if process.terminationStatus == 2 {
            if stderrText.contains("not supported") { throw NativeRuntimeSetupError.unsupported }
            throw NativeRuntimeSetupError.invalidPrincipal
        }
        guard process.terminationStatus == 0 else {
            if stderrText.contains("already exists") { throw NativeRuntimeSetupError.existing }
            throw NativeRuntimeSetupError.failed
        }
        return try decodeReceipt(out, principal: principal, connectionPath: expectedConnection)
    }

    static func realHome(_ home: String) -> String? {
        guard home.hasPrefix("/"), !home.contains("\0"), !home.hasSuffix("/") else { return nil }
        let parts = home.split(separator: "/", omittingEmptySubsequences: true)
        guard !parts.isEmpty, parts.allSatisfy({ $0 != "." && $0 != ".." && !$0.isEmpty }) else {
            return nil
        }

        var current = home
        var missing: [String] = []
        for _ in 0...parts.count {
            var info = stat()
            if current.withCString({ lstat($0, &info) }) == 0 {
                var resolved = [CChar](repeating: 0, count: Int(PATH_MAX))
                guard current.withCString({ realpath($0, &resolved) }) != nil else { return nil }
                let canonical = String(cString: resolved)
                var canonInfo = stat()
                guard canonical.withCString({ stat($0, &canonInfo) }) == 0,
                      (canonInfo.st_mode & S_IFMT) == S_IFDIR else {
                    return nil
                }
                return missing.reversed().reduce(canonical) { $0 + "/" + $1 }
            }
            guard errno == ENOENT else { return nil }
            let url = URL(fileURLWithPath: current, isDirectory: true)
            let name = url.lastPathComponent
            guard !name.isEmpty, current != "/" else { return nil }
            missing.append(name)
            let parent = url.deletingLastPathComponent().path
            if parent == current { return nil }
            current = parent
        }
        return nil
    }

    static func decodeReceipt(_ bytes: Data, principal: String, connectionPath: String) throws -> NativeRuntimeSetupReceipt {
        guard let object = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            throw NativeRuntimeSetupError.failed
        }
        let allowed: Set<String> = [
            "scope", "authority", "principal", "connectionPath", "restartRequired", "connected",
        ]
        guard Set(object.keys) == allowed else { throw NativeRuntimeSetupError.failed }
        let receipt = try JSONDecoder().decode(NativeRuntimeSetupReceipt.self, from: bytes)
        guard receipt.scope == "local-personal", receipt.authority == "setup",
              receipt.restartRequired, receipt.connected == false,
              receipt.principal == principal,
              receipt.connectionPath == connectionPath else {
            throw NativeRuntimeSetupError.failed
        }
        return receipt
    }
}
