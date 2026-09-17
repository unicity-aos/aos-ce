import Foundation

/// One already-authenticated runtime connection. The owner supplies bounded
/// request/reply I/O and calls disconnect on EOF. There is no MCP fallback and
/// no automatic retry of secret-bearing replies.
@MainActor
public final class NativeRuntimeInputSession {
    public typealias Collect = (NativeInputRequest, UUID, Duration) async -> NativeInputAnswer
    public typealias Exchange = (Data) async throws -> Data
    private let connection = UUID()
    private let principal: String
    private let collect: Collect
    private let dismiss: (UUID) -> Void
    private let exchange: Exchange
    private var connected = true
    private var pending: Set<UUID> = []

    public init(principal: String, collect: @escaping Collect,
                dismiss: @escaping (UUID) -> Void, exchange: @escaping Exchange) {
        self.principal = principal
        self.collect = collect
        self.dismiss = dismiss
        self.exchange = exchange
    }

    /// Decode, show the native form, and deliver exactly once. A returned
    /// acknowledgement reports delivery only, never storage success.
    public func handle(_ frame: Data, timeout: Duration) async throws -> NativeRuntimeInputCodec.Delivery {
        guard connected, !Task.isCancelled, timeout > .zero else { throw NativeInputError.invalidRequest }
        let request = try NativeRuntimeInputCodec.request(frame, principal: principal)
        guard pending.insert(request.id).inserted else { throw NativeInputError.invalidRequest }
        defer { pending.remove(request.id) }
        let answer = await collect(request, connection, timeout)
        guard connected, !Task.isCancelled else { throw NativeInputError.invalidRequest }
        let reply = try NativeRuntimeInputCodec.privateReply(answer, to: request)
        do {
            let result = try await exchange(reply)
            guard connected, !Task.isCancelled else { throw NativeInputError.invalidRequest }
            return try NativeRuntimeInputCodec.delivery(result, for: request)
        } catch {
            // Transport errors can include frames. Do not propagate arbitrary
            // peer diagnostics into the UI, logs, or host tool response.
            disconnect()
            throw NativeInputError.invalidRequest
        }
    }

    public func disconnect() {
        guard connected else { return }
        connected = false
        dismiss(connection)
    }
}
