import Foundation

public struct RuntimePromptSnapshot: Sendable {
    public let generation: UInt64
    public let rows: [RuntimePromptRow]
}

/// Apply on the UI actor. Older asynchronous notifications cannot restore a
/// completed prompt, even when callbacks arrive in a different order.
public struct RuntimePromptUpdates {
    private var generation: UInt64 = 0

    public init() {}

    public mutating func accept(_ snapshot: RuntimePromptSnapshot) -> Bool {
        guard snapshot.generation > generation else { return false }
        generation = snapshot.generation
        return true
    }
}

public struct RuntimePromptRow: Equatable, Codable, Sendable, Identifiable {
    public var id: String
    public var requestID: String
    public var message: String
    public var options: [String]
    public var consent: ConsentPresentation?

    public init(id: String, requestID: String, message: String, options: [String],
                consent: ConsentPresentation? = nil) {
        self.id = id
        self.requestID = requestID
        self.message = message
        self.options = options
        self.consent = consent
    }
}

/// Same-user socket prompts, keyed by connection identity rather than request id.
public final class RuntimePromptBroker: PromptResponder, @unchecked Sendable {
    private struct Entry {
        var request: ValidatedPresentationRequest
        var continuation: CheckedContinuation<Int?, Never>
    }

    private enum Slot {
        case reserved
        case waiting(Entry)
        case finished
    }

    private let lock = NSLock()
    private var order: [UUID] = []
    private var slots: [UUID: Slot] = [:]
    private var generation: UInt64 = 0
    public var onChange: (@Sendable (RuntimePromptSnapshot) -> Void)?

    public init() {}

    public func currentPrompts() -> [RuntimePromptRow] {
        currentSnapshot().rows
    }

    public func currentSnapshot() -> RuntimePromptSnapshot {
        lock.lock()
        let snapshot = RuntimePromptSnapshot(generation: generation, rows: snapshotLocked())
        lock.unlock()
        return snapshot
    }

    public func decide(_ request: ValidatedPresentationRequest) async -> Int? {
        let id = UUID()
        reserve(id)
        return await withTaskGroup(of: Void.self) { group in
            group.addTask {
                await self.expire(id, afterSeconds: request.timeoutSeconds)
            }
            let value = await withTaskCancellationHandler {
                await self.waitForChoice(id, request)
            } onCancel: {
                self.complete(id, nil)
            }
            group.cancelAll()
            return value
        }
    }

    public func select(promptID: String, index: Int) {
        guard let id = UUID(uuidString: promptID) else { return }
        lock.lock()
        guard case .waiting(let entry) = slots[id] else {
            lock.unlock()
            return
        }
        let optionCount = entry.request.options.count
        lock.unlock()
        complete(id, PresentationCodec.selection(index, optionCount: optionCount))
    }

    public func cancelAll() {
        lock.lock()
        let ids = Array(slots.keys)
        lock.unlock()
        for id in ids {
            complete(id, nil)
        }
    }

    private func reserve(_ id: UUID) {
        lock.lock()
        slots[id] = .reserved
        lock.unlock()
    }

    private func waitForChoice(_ id: UUID, _ request: ValidatedPresentationRequest) async -> Int? {
        await withCheckedContinuation { continuation in
            lock.lock()
            switch slots[id] {
            case .reserved:
                slots[id] = .waiting(Entry(request: request, continuation: continuation))
                order.append(id)
                let snapshot = changedSnapshotLocked()
                lock.unlock()
                onChange?(snapshot)
            case .waiting:
                lock.unlock()
                continuation.resume(returning: nil)
            case .finished, .none:
                slots[id] = nil
                lock.unlock()
                continuation.resume(returning: nil)
            }
        }
    }

    private func expire(_ id: UUID, afterSeconds timeoutSeconds: Int) async {
        let nanos = UInt64(max(timeoutSeconds, 0)) * 1_000_000_000
        do {
            try await Task.sleep(nanoseconds: nanos)
        } catch {
            // Cancelled because a decision already landed; still complete so a
            // waiting continuation cannot remain parked after the parent task dies.
        }
        complete(id, nil)
    }

    private func complete(_ id: UUID, _ value: Int?) {
        lock.lock()
        switch slots[id] {
        case .none:
            lock.unlock()
        case .reserved:
            slots[id] = .finished
            lock.unlock()
        case .finished:
            lock.unlock()
        case .waiting(let entry):
            slots[id] = nil
            order.removeAll { $0 == id }
            let snapshot = changedSnapshotLocked()
            lock.unlock()
            entry.continuation.resume(returning: value)
            onChange?(snapshot)
        }
    }

    private func changedSnapshotLocked() -> RuntimePromptSnapshot {
        generation += 1
        return RuntimePromptSnapshot(generation: generation, rows: snapshotLocked())
    }

    private func snapshotLocked() -> [RuntimePromptRow] {
        order.compactMap { id in
            guard case .waiting(let entry) = slots[id] else { return nil }
            return RuntimePromptRow(
                id: id.uuidString,
                requestID: entry.request.id,
                message: entry.request.message,
                options: entry.request.options,
                consent: entry.request.consent
            )
        }
    }
}
