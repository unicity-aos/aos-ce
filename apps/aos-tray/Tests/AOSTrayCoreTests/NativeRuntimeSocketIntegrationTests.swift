import CryptoKit
import Foundation
import Testing
@testable import AOSTrayCore

/// Actual Rust NativeUplink + registry, Swift socket/authentication + form adapter.
/// Input is scripted here; this is not a visible-window or WASM guest test.
@MainActor
@Suite(.enabled(if: ProcessInfo.processInfo.environment["ASTRID_NATIVE_INPUT_FIXTURE"] != nil))
struct NativeRuntimeSocketIntegrationTests {
    @Test(arguments: [false, true]) func signedRuntimeRoundTrip(wrongKey: Bool) async throws {
        let binary = try #require(ProcessInfo.processInfo.environment["ASTRID_NATIVE_INPUT_FIXTURE"])
        let root = URL(fileURLWithPath: "/private/tmp/ani-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        let output = root.appendingPathComponent("fixture-output")
        FileManager.default.createFile(atPath: output.path, contents: Data())
        let handle = try FileHandle(forWritingTo: output)
        defer { try? handle.close() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = [root.path]
        process.standardOutput = handle
        process.standardError = handle
        try process.run()
        defer { if process.isRunning { process.terminate(); process.waitUntilExit() } }
        let deadline = ContinuousClock.now.advanced(by: .seconds(20))
        var path: String?
        while process.isRunning && ContinuousClock.now < deadline {
            let log = try String(contentsOf: output, encoding: .utf8)
            if let line = log.split(separator: "\n").first(where: { $0.hasPrefix("ready:") }) {
                path = String(line.dropFirst("ready:".count)); break
            }
            try await Task.sleep(for: .milliseconds(20))
        }
        let key = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: wrongKey ? 8 : 7, count: 32))
        if wrongKey {
            let socket = try await NativeRuntimeSocket.connect(path: #require(path), timeout: 2)
            defer { socket.close() }
            do {
                try await socket.authenticate(principal: "native-input-test", token: Data(repeating: 0xab, count: 32),
                                              signingKey: key, timeout: 2)
                Issue.record("unregistered signing key accepted")
            } catch { #expect(String(describing: error) == "authenticationFailed") }
            do {
                try await socket.writeFrame(Data("no-fallback".utf8), timeout: 1)
                Issue.record("rejected connection remained usable")
            } catch {}
            return
        }
        let configPath = root.appendingPathComponent("connection.json")
        let keyPath = root.appendingPathComponent("tray.ed25519")
        let tokenPath = root.appendingPathComponent("system.token")
        try key.rawRepresentation.write(to: keyPath)
        try Data(String(repeating: "ab", count: 32).utf8).write(to: tokenPath)
        let configData = try JSONSerialization.data(withJSONObject: [
            "socketPath": try #require(path), "principal": "native-input-test",
            "privateKeyPath": keyPath.path, "tokenPath": tokenPath.path, "capacity": 1,
            "inputTimeoutSeconds": 2, "ioTimeoutSeconds": 2, "readTimeoutSeconds": 3])
        try configData.write(to: configPath)
        for file in [configPath, keyPath, tokenPath] {
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        }
        let config = try NativeRuntimeConfiguration.load(path: configPath.path)
        let socket = try await config.connect()
        defer { socket.close() }
        var presented = 0
        var deliveries = 0
        let connection = try NativeRuntimeInputConnection(socket: socket, principal: "native-input-test",
            capacity: 1, inputTimeout: .seconds(2), ioTimeout: 2, collect: { request, _, _ in
            #expect(request.capsule == "fixture")
            #expect(request.key == "token")
            #expect(request.kind == .secret)
            presented += 1
            return .value("swift-runtime-synthetic")
        }, dismiss: { _ in }, delivered: { _, result in
            #expect(result == .delivered)
            deliveries += 1
            socket.close()
        })
        do { try await connection.run(readTimeout: 3) }
        catch { /* Closing after the acknowledgement ends the read loop. */ }
        #expect(presented == 1)
        #expect(deliveries == 1)
        connection.close()
        socket.close()
        let completion = ContinuousClock.now.advanced(by: .seconds(3))
        while process.isRunning && ContinuousClock.now < completion { try await Task.sleep(for: .milliseconds(20)) }
        try #require(!process.isRunning, "fixture did not finish after disconnect")
        #expect(process.terminationStatus == 0)
        let log = try String(contentsOf: output, encoding: .utf8)
        #expect(log.contains("completed:private-waiter-delivered"))
        #expect(!log.contains("swift-runtime-synthetic"))
    }
}
