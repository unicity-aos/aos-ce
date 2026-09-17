import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct RuntimePromptUpdatesTests {
    @Test func delayedNotificationsCannotRestoreCompletedRequests() async throws {
        let broker = RuntimePromptBroker()
        let notifications = CapturedPromptSnapshots()
        broker.onChange = { notifications.append($0) }
        let request = ValidatedPresentationRequest(id: "same-id", message: "Test",
            options: ["Allow", "Deny"], timeoutSeconds: 30)
        let first = Task { await broker.decide(request) }
        let second = Task { await broker.decide(request) }
        defer { first.cancel(); second.cancel(); broker.cancelAll() }
        try await until { broker.currentPrompts().count == 2 }
        let ids = broker.currentPrompts().map(\.id)
        await withTaskGroup(of: Void.self) { group in
            for id in ids {
                group.addTask { broker.select(promptID: id, index: 1) }
            }
        }
        #expect(await first.value == 1)
        #expect(await second.value == 1)
        try await until { notifications.values.count == 4 }

        // Deliver the terminal empty snapshot first, then deliberately replay
        // every older notification. This deterministically models a slow UI hop.
        var updates = RuntimePromptUpdates()
        var displayed: [RuntimePromptRow] = []
        let reverse = notifications.values.sorted { $0.generation > $1.generation }
        #expect(Set(reverse.map(\.generation)) == [1, 2, 3, 4])
        for snapshot in reverse {
            if updates.accept(snapshot) { displayed = snapshot.rows }
        }
        #expect(displayed == broker.currentPrompts())
        #expect(displayed.isEmpty)
        let duplicateAccepted = updates.accept(try #require(reverse.first))
        #expect(!duplicateAccepted)

        let next = Task { await broker.decide(request) }
        defer { next.cancel() }
        try await until { broker.currentPrompts().count == 1 }
        let latest = broker.currentSnapshot()
        let newAccepted = updates.accept(latest)
        #expect(newAccepted)
        #expect(!ids.contains(try #require(latest.rows.first).id))
        broker.cancelAll()
        #expect(await next.value == nil)
        let cancellationAccepted = updates.accept(broker.currentSnapshot())
        let staleAccepted = updates.accept(latest)
        #expect(cancellationAccepted)
        #expect(!staleAccepted)
    }

    private func until(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        throw WaitError.timedOut
    }
    private enum WaitError: Error { case timedOut }
}

private final class CapturedPromptSnapshots: @unchecked Sendable {
    private let lock = NSLock()
    private var snapshots: [RuntimePromptSnapshot] = []
    var values: [RuntimePromptSnapshot] { lock.withLock { snapshots } }
    func append(_ snapshot: RuntimePromptSnapshot) {
        lock.withLock { snapshots.append(snapshot) }
    }
}
