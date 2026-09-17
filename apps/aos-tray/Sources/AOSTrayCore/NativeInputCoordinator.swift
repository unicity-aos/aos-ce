import Foundation

/// Owns pending native forms, not their answers. One window is visible at a
/// time; queued requests retain their original deadline. A transport must call
/// disconnect when its authenticated runtime connection closes.
@MainActor
public final class NativeInputCoordinator {
    public typealias Present = (UUID, NativeInputRequest) -> Void
    public typealias Dismiss = (UUID) -> Void

    private struct Entry {
        let request: NativeInputRequest
        let connection: UUID
        let continuation: CheckedContinuation<NativeInputAnswer, Never>
        let expiry: Task<Void, Never>
    }

    private let capacity: Int
    private let present: Present
    private let dismiss: Dismiss
    private var entries: [UUID: Entry] = [:]
    private var order: [UUID] = []
    public private(set) var active: UUID?
    public var pendingCount: Int { entries.count }

    public init(capacity: Int, present: @escaping Present, dismiss: @escaping Dismiss) throws {
        guard capacity > 0 else { throw NativeInputError.invalidRequest }
        self.capacity = capacity
        self.present = present
        self.dismiss = dismiss
    }

    public func collect(_ request: NativeInputRequest, connection: UUID,
                        timeout: Duration) async -> NativeInputAnswer {
        guard !Task.isCancelled, timeout > .zero,
              (try? request.validate()) != nil, entries.count < capacity,
              !entries.values.contains(where: {
                  $0.connection == connection && $0.request.id == request.id
              }) else { return .cancelled }
        // A fresh ticket prevents a late window callback from answering a new
        // request even if a peer reuses the same public request UUID.
        let ticket = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                guard !Task.isCancelled else {
                    continuation.resume(returning: .cancelled)
                    return
                }
                let expiry = Task { [weak self] in
                    do { try await Task.sleep(for: timeout) }
                    catch { return }
                    self?.finish(ticket, answer: .cancelled)
                }
                entries[ticket] = Entry(request: request, connection: connection,
                    continuation: continuation, expiry: expiry)
                order.append(ticket)
                showNext()
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.finish(ticket, answer: .cancelled) }
        }
    }

    public func submit(ticket: UUID, answer: NativeInputAnswer) {
        guard active == ticket, let entry = entries[ticket],
              (try? entry.request.validateAnswer(answer)) != nil else { return }
        finish(ticket, answer: answer)
    }

    public func disconnect(_ connection: UUID) {
        cancel(entries.filter { $0.value.connection == connection }.map(\.key))
    }

    public func cancelAll() { cancel(Array(entries.keys)) }

    private func cancel(_ tickets: [UUID]) {
        // Remove queued requests before closing the active window; otherwise
        // closing it could briefly present another request from a dead peer.
        for ticket in tickets where ticket != active { finish(ticket, answer: .cancelled) }
        if let active, tickets.contains(active) { finish(active, answer: .cancelled) }
    }

    private func finish(_ ticket: UUID, answer: NativeInputAnswer) {
        guard let entry = entries.removeValue(forKey: ticket) else { return }
        order.removeAll { $0 == ticket }
        entry.expiry.cancel()
        if active == ticket {
            active = nil
            dismiss(ticket)
        }
        entry.continuation.resume(returning: answer)
        showNext()
    }

    private func showNext() {
        guard active == nil, let next = order.first, let entry = entries[next] else { return }
        active = next
        present(next, entry.request)
    }
}
