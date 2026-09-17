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
        guard home.hasPrefix("/"), !home.contains("\0"), !home.hasSuffix("/") else { return nil }
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
        guard binary.hasPrefix("/"), home.hasPrefix("/"), timeout > 0, timeout.isFinite else {
            throw NativeRuntimeSetupError.unavailable
        }
        guard OwnedPrincipal.isValidID(principal), principal != "anonymous" else {
            throw NativeRuntimeSetupError.invalidPrincipal
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
        environment["AOS_HOME"] = home
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
        return try decodeReceipt(out)
    }

    static func decodeReceipt(_ bytes: Data) throws -> NativeRuntimeSetupReceipt {
        guard let object = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            throw NativeRuntimeSetupError.failed
        }
        for (key, value) in object {
            let haystack = "\(key) \(value)".lowercased()
            if haystack.contains("token") || haystack.contains("secret") || haystack.contains("privatekey")
                || haystack.contains("astrid_pair") {
                throw NativeRuntimeSetupError.failed
            }
        }
        let receipt = try JSONDecoder().decode(NativeRuntimeSetupReceipt.self, from: bytes)
        guard receipt.scope == "local-personal", receipt.authority == "setup",
              receipt.restartRequired, receipt.connected == false,
              OwnedPrincipal.isValidID(receipt.principal), receipt.principal != "anonymous",
              receipt.connectionPath.hasPrefix("/") else {
            throw NativeRuntimeSetupError.failed
        }
        return receipt
    }
}
