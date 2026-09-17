import Foundation
import Testing
@testable import AOSTrayCore

@MainActor
@Suite struct NativeInputCoordinatorTests {
    private func request(id: UUID = UUID()) throws -> NativeInputRequest {
        try NativeInputRequest.decode(JSONSerialization.data(withJSONObject: [
            "id": id.uuidString, "principal": "codex-code", "capsule": "demo",
            "key": "token", "prompt": "Enter token", "kind": "secret"
        ]))
    }

    private func settle(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        Issue.record("coordinator did not reach expected state")
        throw NativeInputError.invalidRequest
    }

    @Test func serialPresentationAndInvalidAnswer() async throws {
        var shown: [UUID] = []
        var dismissed: [UUID] = []
        let coordinator = try NativeInputCoordinator(capacity: 2,
            present: { ticket, _ in shown.append(ticket) }, dismiss: { dismissed.append($0) })
        defer { coordinator.cancelAll() }
        let connection = UUID()
        let firstRequest = try request()
        let secondRequest = try request()
        let first = Task { await coordinator.collect(firstRequest, connection: connection, timeout: .seconds(5)) }
        try await settle { shown.count == 1 }
        let second = Task { await coordinator.collect(secondRequest, connection: connection, timeout: .seconds(5)) }
        try await settle { coordinator.pendingCount == 2 }
        #expect(shown.count == 1)
        coordinator.submit(ticket: shown[0], answer: .value(""))
        #expect(coordinator.pendingCount == 2)
        coordinator.submit(ticket: shown[0], answer: .value("synthetic"))
        #expect(await first.value == .value("synthetic"))
        #expect(shown.count == 2)
        #expect(dismissed == [shown[0]])
        coordinator.submit(ticket: shown[1], answer: .cancelled)
        #expect(await second.value == .cancelled)
        #expect(coordinator.pendingCount == 0)
    }

    @Test func disconnectDoesNotFlashQueuedRequestsOrCancelOtherConnections() async throws {
        var shown: [UUID] = []
        let coordinator = try NativeInputCoordinator(capacity: 3,
            present: { ticket, _ in shown.append(ticket) }, dismiss: { _ in })
        defer { coordinator.cancelAll() }
        let one = UUID(), two = UUID()
        let a = try request(), b = try request(), c = try request()
        let first = Task { await coordinator.collect(a, connection: one, timeout: .seconds(5)) }
        try await settle { coordinator.pendingCount == 1 }
        let queued = Task { await coordinator.collect(b, connection: one, timeout: .seconds(5)) }
        try await settle { coordinator.pendingCount == 2 }
        let other = Task { await coordinator.collect(c, connection: two, timeout: .seconds(5)) }
        try await settle { coordinator.pendingCount == 3 }
        coordinator.disconnect(one)
        #expect(await first.value == .cancelled)
        #expect(await queued.value == .cancelled)
        #expect(shown.count == 2)
        #expect(coordinator.pendingCount == 1)
        coordinator.submit(ticket: shown[1], answer: .value("other"))
        #expect(await other.value == .value("other"))
    }

    @Test func queuedDeadlineIsNotRestartedAtPresentation() async throws {
        var shown = 0
        let coordinator = try NativeInputCoordinator(capacity: 2,
            present: { _, _ in shown += 1 }, dismiss: { _ in })
        defer { coordinator.cancelAll() }
        let connection = UUID(), a = try request(), b = try request()
        let first = Task { await coordinator.collect(a, connection: connection, timeout: .seconds(5)) }
        try await settle { shown == 1 }
        let queued = Task { await coordinator.collect(b, connection: connection, timeout: .milliseconds(20)) }
        #expect(await queued.value == .cancelled)
        #expect(shown == 1)
        coordinator.cancelAll()
        #expect(await first.value == .cancelled)
    }

    @Test func cancellationAndReusedRequestIDRejectLateWindowReply() async throws {
        var shown: [UUID] = []
        let coordinator = try NativeInputCoordinator(capacity: 1,
            present: { ticket, _ in shown.append(ticket) }, dismiss: { _ in })
        defer { coordinator.cancelAll() }
        let connection = UUID(), req = try request()
        let first = Task { await coordinator.collect(req, connection: connection, timeout: .seconds(5)) }
        try await settle { shown.count == 1 }
        first.cancel()
        #expect(await first.value == .cancelled)
        let second = Task { await coordinator.collect(req, connection: connection, timeout: .seconds(5)) }
        try await settle { shown.count == 2 }
        coordinator.submit(ticket: shown[0], answer: .value("stale"))
        #expect(coordinator.pendingCount == 1)
        coordinator.submit(ticket: shown[1], answer: .value("fresh"))
        #expect(await second.value == .value("fresh"))
    }

    @Test func duplicateCapacityAndPreCancelledTasksDoNotPresent() async throws {
        var shown = 0
        let coordinator = try NativeInputCoordinator(capacity: 1,
            present: { _, _ in shown += 1 }, dismiss: { _ in })
        defer { coordinator.cancelAll() }
        let connection = UUID(), req = try request()
        let first = Task { await coordinator.collect(req, connection: connection, timeout: .seconds(5)) }
        try await settle { shown == 1 }
        #expect(await coordinator.collect(req, connection: connection, timeout: .seconds(1)) == .cancelled)
        let second = try request()
        #expect(await coordinator.collect(second, connection: connection, timeout: .seconds(1)) == .cancelled)
        coordinator.cancelAll()
        #expect(await first.value == .cancelled)
        let cancelled = Task { await coordinator.collect(req, connection: connection, timeout: .seconds(1)) }
        cancelled.cancel()
        #expect(await cancelled.value == .cancelled)
        #expect(shown == 1)
    }
}
