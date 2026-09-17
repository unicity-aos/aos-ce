import Foundation
import Testing
@testable import AOSTrayCore

@MainActor @Suite struct NativeRuntimeInputSessionTests {
    private func frame(_ id: UUID) throws -> Data {
        try JSONSerialization.data(withJSONObject: ["topic": "astrid.v1.private.elicit.request", "principal": "alice",
            "source_id": "00000000-0000-0000-0000-000000000000",
            "payload": ["request_id": id.uuidString, "capsule_id": "example",
                "field": ["key": "token", "prompt": "Enter token", "field_type": "Secret"]]])
    }

    private func ack(_ id: UUID) throws -> Data {
        try JSONSerialization.data(withJSONObject: ["topic": "astrid.v1.private.elicit.result",
            "principal": "alice", "payload": ["request_id": id.uuidString, "status": "delivered"]])
    }

    @Test func nativeAnswerUsesPrivateExchangeExactlyOnce() async throws {
        let id = UUID()
        var calls = 0
        let session = NativeRuntimeInputSession(principal: "alice", collect: { request, _, _ in
            #expect(request.id == id)
            return .value("synthetic-session-input")
        }, dismiss: { _ in }, exchange: { bytes in
            calls += 1
            let object = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
            #expect(object["topic"] as? String == "astrid.v1.private.elicit.reply")
            return try ack(id)
        })
        #expect(try await session.handle(frame(id), timeout: .seconds(2)) == .delivered)
        #expect(calls == 1)
    }

    @Test func disconnectDismissesFormWithoutSendingAnAnswer() async throws {
        var ticket: UUID?
        var calls = 0
        let coordinator = try NativeInputCoordinator(capacity: 1,
            present: { value, _ in ticket = value }, dismiss: { _ in ticket = nil })
        let session = NativeRuntimeInputSession(principal: "alice", collect: { request, connection, timeout in
            await coordinator.collect(request, connection: connection, timeout: timeout)
        }, dismiss: { coordinator.disconnect($0) }, exchange: { _ in
            calls += 1
            return Data()
        })
        let bytes = try frame(UUID())
        let task = Task { try await session.handle(bytes, timeout: .seconds(2)) }
        for _ in 0..<100 where ticket == nil { await Task.yield() }
        try #require(ticket != nil)
        session.disconnect()
        do { _ = try await task.value; Issue.record("disconnected request succeeded") }
        catch { #expect(String(describing: error) == "invalidRequest") }
        #expect(ticket == nil)
        #expect(coordinator.pendingCount == 0)
        #expect(calls == 0)
    }

    @Test func transportFailureIsRedactedAndNeverRetried() async throws {
        struct LeakyError: Error, CustomStringConvertible {
            var description: String { "synthetic-transport-secret" }
        }
        var calls = 0
        var dismissed = 0
        let session = NativeRuntimeInputSession(principal: "alice",
            collect: { _, _, _ in .value("synthetic-input") },
            dismiss: { _ in dismissed += 1 }, exchange: { _ in calls += 1; throw LeakyError() })
        let bytes = try frame(UUID())
        for _ in 0..<2 {
            do { _ = try await session.handle(bytes, timeout: .seconds(1)); Issue.record("failure accepted") }
            catch { #expect(String(describing: error) == "invalidRequest") }
        }
        #expect(calls == 1)
        #expect(dismissed == 1)
    }
}
