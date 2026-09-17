import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct NativeRuntimeConfigurationTests {
    @Test func missingSessionTokenIsTransientButInvalidCredentialsAreNot() async throws {
        let root = URL(fileURLWithPath: "/private/tmp/ani-reconnect-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let key = root.appendingPathComponent("key")
        let token = root.appendingPathComponent("token")
        try Data(repeating: 7, count: 32).write(to: key)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: key.path)
        let config = NativeRuntimeConfiguration(socketPath: root.path + "/socket", principal: "default",
            privateKeyPath: key.path, tokenPath: token.path, capacity: 4,
            inputTimeoutSeconds: 90, ioTimeoutSeconds: 5, readTimeoutSeconds: 300)
        do {
            _ = try await config.connect()
            Issue.record("Missing token connected")
        } catch NativeRuntimeSocketError.unavailable {} // Runtime may be stopped.
        catch { Issue.record("Missing session token was classified as permanent: \(error)") }

        try Data(repeating: 120, count: 64).write(to: token) // Not hexadecimal.
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: token.path)
        do {
            _ = try await config.connect()
            Issue.record("Malformed token connected")
        } catch NativeRuntimeConfigurationError.unavailableCredential {}
        catch { Issue.record("Malformed token was not a permanent credential failure") }

        try FileManager.default.removeItem(at: key)
        do {
            _ = try await config.connect()
            Issue.record("Missing device key connected")
        } catch NativeRuntimeConfigurationError.unavailableCredential {}
        catch { Issue.record("Missing device key was not a permanent credential failure") }
    }

    @Test func privateFilesAreBoundedAndRejectRedirects() throws {
        let root = URL(fileURLWithPath: "/private/tmp/ani-credential-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let key = root.appendingPathComponent("key")
        try Data(repeating: 7, count: 32).write(to: key)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: key.path)
        #expect(try PrivateRuntimeFile.read(key.path, maximum: 32).count == 32)
        #expect(throws: (any Error).self) { try PrivateRuntimeFile.read(key.path, maximum: 31) }
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: key.path)
        #expect(throws: (any Error).self) { try PrivateRuntimeFile.read(key.path, maximum: 32) }
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: key.path)
        let link = root.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: key)
        #expect(throws: (any Error).self) { try PrivateRuntimeFile.read(link.path, maximum: 32) }
        let directoryLink = root.appendingPathComponent("directory-link")
        try FileManager.default.createSymbolicLink(at: directoryLink, withDestinationURL: root)
        #expect(throws: (any Error).self) { try PrivateRuntimeFile.read(directoryLink.appendingPathComponent("key").path, maximum: 32) }
        for path in ["relative", root.path + "/../key", root.path + "//key", root.path + "/./key"] {
            #expect(throws: (any Error).self) { try PrivateRuntimeFile.read(path, maximum: 32) }
        }
    }

    @Test func configuredInputCannotRunInDemoOrSnapshotMode() throws {
        for args in [["--demo", "--native-input-config", "/config"],
                     ["--native-input-config", "/config", "--snapshot"],
                     ["--native-input-config", "relative"],
                     ["--native-input-config", "/a", "--native-input-config", "/b"]] {
            if case .success = LaunchArguments.parse(["aos-tray"] + args) { Issue.record("invalid launch accepted") }
        }
        let parsed = try LaunchArguments.parse(["aos-tray", "--native-input-config", "/config"]).get()
        #expect(parsed.nativeInputConfig == "/config")
    }

    @Test func configurationValidatesLimitsWithoutReadingCredentials() throws {
        let root = URL(fileURLWithPath: "/private/tmp/ani-config-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("connection.json")
        var object: [String: Any] = ["socketPath": root.path + "/system.sock", "principal": "codex-code",
            "privateKeyPath": root.path + "/tray.ed25519", "tokenPath": root.path + "/system.token",
            "capacity": 8, "inputTimeoutSeconds": 120, "ioTimeoutSeconds": 5, "readTimeoutSeconds": 3600]
        func write() throws {
            try JSONSerialization.data(withJSONObject: object).write(to: file)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        }
        try write()
        let config = try NativeRuntimeConfiguration.load(path: file.path)
        #expect(config.principal == "codex-code")
        for capacity in [0, -1] {
            object["capacity"] = capacity
            try write()
            #expect(throws: NativeRuntimeConfigurationError.invalidConfiguration) {
                try NativeRuntimeConfiguration.load(path: file.path)
            }
        }
    }
}
