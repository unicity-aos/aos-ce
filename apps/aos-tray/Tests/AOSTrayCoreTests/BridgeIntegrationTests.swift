import Darwin
import Foundation
import Testing
@testable import AOSTrayCore

/// Opt-in cross-language test: real Rust proxy and Swift listener, scripted UI.
@Suite(.enabled(if: ProcessInfo.processInfo.environment["AOS_TRAY_TEST_BINARY"] != nil))
struct BridgeIntegrationTests {
    // Astrid ApprovalForm's exact wire enum, including persistent approval.
    @Test(arguments: [0, 1, 2, 3, -1], [false, true])
    func nativeDecisionReturnsToOriginalCall(selection: Int, modern: Bool) async throws {
        try await exercise(selection: selection, modern: modern, approval: true)
    }

    @Test(arguments: [0, 1, -1], [false, true])
    func booleanDecisionReturnsToOriginalCall(selection: Int, modern: Bool) async throws {
        try await exercise(selection: selection, modern: modern, approval: false)
    }

    @Test(arguments: [false, true], [false, true])
    func structuredConsentReachesTray(modern: Bool, approval: Bool) async throws {
        try await exercise(selection: 0, modern: modern, approval: approval, metadata: true)
    }

    @Test(arguments: [false, true])
    func durableAlwaysReachesTray(modern: Bool) async throws {
        try await exercise(selection: 2, modern: modern, approval: true, metadata: true, durable: true)
    }

    private func exercise(selection: Int, modern: Bool, approval: Bool, metadata: Bool = false, durable: Bool = false) async throws {
        let binary = try #require(ProcessInfo.processInfo.environment["AOS_TRAY_TEST_BINARY"])
        let root = URL(fileURLWithPath: "/private/tmp/at-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(
            at: root, withIntermediateDirectories: false,
            attributes: [.posixPermissions: 0o700]
        )
        defer { try? FileManager.default.removeItem(at: root) }
        let socket = root.appendingPathComponent("ui.sock").path
        let server = UnixPresentationServer(
            path: socket, responder: BridgeDecision(selection: selection, approval: approval, metadata: metadata, durable: durable)
        )
        try server.start()
        defer { server.stop() }

        let runtime = root.appendingPathComponent("runtime.sh")
        let legacyScript = #"""
        #!/bin/sh
        IFS= read -r call || exit 90
        printf 'call-read\n' >&2
        printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"elicitation/create","params":{"mode":"form","message":"Integration decision","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}'
        IFS= read -r decision || exit 91
        printf 'decision-read\n' >&2
        printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "$decision"
        """#
        let modernScript = #"""
        #!/bin/sh
        IFS= read -r call || exit 90
        printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required","requestState":"opaque-integration-state","inputRequests":{"permission":{"method":"elicitation/create","params":{"mode":"form","message":"Integration decision","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
        IFS= read -r decision || exit 91
        printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "$decision"
        """#
        let booleanSchema = #""grant":{"type":"boolean"}"#
        let approvalSchema = #""choice":{"type":"string","enum":["approve_once","approve_session","approve_always","deny"]}"#
        let baseScript = modern ? modernScript : legacyScript
        var script = approval ? baseScript.replacingOccurrences(of: booleanSchema, with: approvalSchema)
            .replacingOccurrences(of: #""required":["grant"]"#, with: #""required":["choice"]"#) : baseScript
        if approval { try #require(script.contains("approve_always")) }
        if metadata {
            let choices: [[String: Any]] = approval ? [
                ["value": "deny", "lifetime": "none"],
                ["value": "approve_always", "lifetime": durable ? "durable" : "until_runtime_restart"],
                ["value": "approve_session", "lifetime": "session"],
                ["value": "approve_once", "lifetime": "none"],
            ] : [["value": false, "lifetime": "none"], ["value": true, "lifetime": "durable"]]
            let extensionValue: [String: Any] = ["org.astrid/consent": [
                "version": 1, "kind": approval ? "action_approval" : "capsule_access",
                "principal": "codex-code", "choices": choices,
            ]]
            let encoded = String(decoding: try JSONSerialization.data(withJSONObject: extensionValue), as: UTF8.self)
            script = script.replacingOccurrences(of: #""mode":"form""#, with: #""mode":"form","_meta":"# + encoded)
        }
        try Data(script.utf8).write(to: runtime)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: runtime.path)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = ["mcp", "serve", "--interaction", "native", "--interaction-socket", socket]
        var environment = ProcessInfo.processInfo.environment
        environment["AOS_HOME"] = root.appendingPathComponent("home").path
        environment["UNICITY_AOS_RUNTIME_BIN"] = runtime.path
        process.environment = environment
        let input = Pipe()
        let outputURL = root.appendingPathComponent("host-output")
        FileManager.default.createFile(atPath: outputURL.path, contents: Data())
        let output = try FileHandle(forWritingTo: outputURL)
        defer { try? output.close() }
        let errors = Pipe()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = errors
        try process.run()
        defer {
            try? input.fileHandleForWriting.close()
            if process.isRunning { process.terminate(); process.waitUntilExit() }
        }
        try input.fileHandleForWriting.write(contentsOf: Data(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"test.read\"}}\n".utf8
        ))
        let deadline = ContinuousClock.now.advanced(by: .seconds(30))
        var bytes = Data()
        while ContinuousClock.now < deadline {
            bytes = try Data(contentsOf: outputURL)
            if bytes.contains(0x0A) || !process.isRunning { break }
            try await Task.sleep(for: .milliseconds(20))
        }
        let timedOut = !bytes.contains(0x0A)
        try input.fileHandleForWriting.close()
        if timedOut { process.terminate() }
        process.waitUntilExit()
        let stderr = String(decoding: errors.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        try #require(!timedOut, "Rust bridge did not finish within 30 seconds: \(stderr)")
        #expect(process.terminationStatus == 0, "\(stderr)")
        let reply = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        #expect((reply["id"] as? NSNumber)?.intValue == 1)
        let original = try #require(reply["result"] as? [String: Any])
        let result: [String: Any]
        if modern {
            #expect(original["method"] as? String == "tools/call")
            let params = try #require(original["params"] as? [String: Any])
            #expect(params["name"] as? String == "test.read")
            #expect(params["requestState"] as? String == "opaque-integration-state")
            let responses = try #require(params["inputResponses"] as? [String: Any])
            result = try #require(responses["permission"] as? [String: Any])
        } else {
            #expect(original["id"] as? String == "permission")
            result = try #require(original["result"] as? [String: Any])
        }
        #expect(result["action"] as? String == (selection < 0 ? "cancel" : "accept"))
        if selection >= 0 {
            let content = try #require(result["content"] as? [String: Any])
            if approval {
                let choices = ["approve_once", "approve_session", "approve_always", "deny"]
                #expect(content["choice"] as? String == choices[selection])
            } else {
                #expect(content["grant"] as? Bool == (selection == 0))
            }
        } else {
            #expect(result["content"] == nil)
        }
    }
}

private struct BridgeDecision: PromptResponder {
    let selection: Int
    let approval: Bool
    let metadata: Bool
    let durable: Bool
    func decide(_ request: ValidatedPresentationRequest) async -> Int? {
        #expect(request.message == "Integration decision")
        #expect(request.options == (approval
            ? ["Approve Once", "Approve for Session", metadata && !durable ? "Until runtime restart" : "Always Approve", "Deny"]
            : ["Grant", "Deny"]))
        if metadata {
            #expect(request.consent?.principal == "codex-code")
            #expect(request.consent?.kind == (approval ? .actionApproval : .capsuleAccess))
            #expect(request.consent?.lifetimes == (approval
                ? [.none, .session, durable ? .durable : .untilRuntimeRestart, .none] : [.durable, .none]))
        } else {
            #expect(request.consent == nil)
        }
        return selection < 0 ? nil : selection
    }
}
