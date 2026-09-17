import Foundation
import Testing
@testable import AOSTrayCore

/// Opt-in real packaged WASM guest against a separately prepared disposable daemon.
/// Uses synthetic input through the production Swift connection, not a visible popup.
@MainActor
@Suite(.enabled(if: ProcessInfo.processInfo.environment["ASTRID_NATIVE_GUEST_ROOT"] != nil))
struct NativeGuestJourneyTests {
    @Test func cancelCollectAndRestart() async throws {
        let root = try #require(ProcessInfo.processInfo.environment["ASTRID_NATIVE_GUEST_ROOT"])
        let binary = try #require(ProcessInfo.processInfo.environment["ASTRID_NATIVE_GUEST_CLI"])
        try #require(URL(fileURLWithPath: root).deletingLastPathComponent().path == "/private/tmp")
        try #require(URL(fileURLWithPath: root).lastPathComponent.hasPrefix("ani-guest."))
        let config = try NativeRuntimeConfiguration.load(path: root + "/connection.json")
        let socket = try await config.connect()
        var presentations = 0
        var acknowledgements = 0
        let connection = try NativeRuntimeInputConnection(socket: socket, principal: "default",
            capacity: 4, inputTimeout: .seconds(10), ioTimeout: 5, collect: { request, _, _ in
                #expect(request.capsule == "native-input-probe")
                #expect(request.key == "token")
                #expect(request.kind == .secret)
                presentations += 1
                return presentations == 1 ? .cancelled : .value("native-guest-synthetic-only")
            }, dismiss: { _ in }, delivered: { _, result in
                #expect(result == .delivered)
                acknowledgements += 1
            })
        let reader = Task { try await connection.run(readTimeout: 60) }
        defer { connection.close(); reader.cancel() }
        let cancelled = try await cli(binary, root, ["capsule", "run", "native-input-probe", "collect"])
        #expect(cancelled.status != 0)
        #expect(cancelled.output.contains("input-not-completed"))
        let absent = try await cli(binary, root, ["capsule", "run", "native-input-probe", "stored"])
        #expect(absent.status != 0)
        #expect(absent.output.contains("secret-absent"))
        let provided = try await cli(binary, root, ["capsule", "run", "native-input-probe", "collect"])
        #expect(provided.status == 0)
        #expect(provided.output.contains("secret-present"))
        #expect(!provided.output.contains("native-guest-synthetic-only"))
        #expect(presentations == 2)
        #expect(acknowledgements == 2)
        connection.close()
        reader.cancel()
        _ = await reader.result
        let stopped = try await cli(binary, root, ["stop"])
        try #require(stopped.status == 0, "disposable runtime must stop cleanly")
        let entries = try FileManager.default.contentsOfDirectory(atPath: root + "/runtime")
        #expect(entries == ["astrid.volume"])
        let started = try await cli(binary, root, ["start"])
        try #require(started.status == 0, "disposable runtime must restart")
        let reconnected = try await config.connect()
        var resumedPresentations = 0
        var formPresentations: [String] = []
        var cancelForms = false
        let resumedConnection = try NativeRuntimeInputConnection(socket: reconnected,
            principal: "default", capacity: 4, inputTimeout: .seconds(10), ioTimeout: 5,
            collect: { request, _, _ in
                #expect(request.capsule == "native-input-probe")
                if request.key != "token" {
                    formPresentations.append(request.key)
                    if cancelForms { return .cancelled }
                    switch request.key {
                    case "text":
                        #expect(request.kind == .text)
                        return .value("native-text")
                    case "empty-text":
                        #expect(request.kind == .text)
                        return .value("")
                    case "select":
                        #expect(request.kind == .select)
                        #expect(request.options == ["one", "two"])
                        return .value("two")
                    case "array":
                        #expect(request.kind == .array)
                        return .values(["one,two", "three"])
                    case "empty-array":
                        #expect(request.kind == .array)
                        return .values([])
                    default:
                        Issue.record("unexpected form key")
                        return .cancelled
                    }
                }
                #expect(request.kind == .secret)
                resumedPresentations += 1
                return .value("native-guest-restarted-synthetic")
            }, dismiss: { _ in }, delivered: { _, result in
                #expect(result == .delivered)
            })
        let resumedReader = Task { try await resumedConnection.run(readTimeout: 60) }
        defer { resumedConnection.close(); resumedReader.cancel() }
        let persisted = try await cli(binary, root, ["capsule", "run", "native-input-probe", "stored"])
        #expect(persisted.status == 0)
        #expect(persisted.output.contains("secret-present"))
        #expect(!persisted.output.contains("native-guest-synthetic-only"))
        let recollected = try await cli(binary, root,
            ["capsule", "run", "native-input-probe", "collect"])
        #expect(recollected.status == 0)
        #expect(recollected.output.contains("secret-present"))
        #expect(!recollected.output.contains("native-guest-restarted-synthetic"))
        #expect(resumedPresentations == 1)
        let formCommands = ["text", "empty-text", "select", "array", "empty-array"]
        for command in formCommands {
            let result = try await cli(binary, root,
                ["capsule", "run", "native-input-probe", command])
            #expect(result.status == 0)
            #expect(result.output.contains("input-matched"))
        }
        #expect(formPresentations == formCommands)
        cancelForms = true
        for command in formCommands {
            let result = try await cli(binary, root,
                ["capsule", "run", "native-input-probe", command])
            #expect(result.status != 0)
            #expect(result.output.contains("input-not-completed"))
        }
        #expect(formPresentations == formCommands + formCommands)
        resumedConnection.close()
        resumedReader.cancel()
        _ = await resumedReader.result
    }

    private func cli(_ binary: String, _ root: String, _ args: [String]) async throws
        -> (status: Int32, output: String) {
        let output = URL(fileURLWithPath: root).appendingPathComponent("cli-\(UUID().uuidString).log")
        FileManager.default.createFile(atPath: output.path, contents: Data(),
                                       attributes: [.posixPermissions: 0o600])
        let file = try FileHandle(forWritingTo: output)
        defer { try? file.close() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.currentDirectoryURL = URL(fileURLWithPath: root)
        process.arguments = args
        var environment = ProcessInfo.processInfo.environment
        environment["ASTRID_HOME"] = root + "/runtime"
        environment["ASTRID_PRINCIPAL"] = "default"
        environment.removeValue(forKey: "ASTRID_RUN_DIR")
        process.environment = environment
        process.standardOutput = file
        process.standardError = file
        try process.run()
        defer { if process.isRunning { process.terminate(); process.waitUntilExit() } }
        let deadline = ContinuousClock.now.advanced(by: .seconds(30))
        while process.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        try #require(!process.isRunning, "guest command timed out: \(args)")
        return (process.terminationStatus, try String(contentsOf: output, encoding: .utf8))
    }
}
