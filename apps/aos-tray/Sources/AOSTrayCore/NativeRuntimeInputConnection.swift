import Foundation

/// Reads the authenticated socket continuously, including while a form is open.
/// The caller owns authentication and explicit private-responder activation.
/// EOF closes forms; concurrent replies are correlated by request UUID.
@MainActor
public final class NativeRuntimeInputConnection {
    private struct Header: Decodable { let topic: String }
    private struct Envelope: Decodable {
        let topic: String
        let payload: Payload?
        struct Payload: Decodable { let request_id: UUID? }
    }
    private struct PendingReply {
        let continuation: CheckedContinuation<Data, any Error>
        let deadline: Task<Void, Never>
    }
    private let socket: NativeRuntimeSocket
    private let principal: String
    private let capacity: Int
    private let inputTimeout: Duration
    private let ioTimeout: TimeInterval
    private let collect: NativeRuntimeInputSession.Collect
    private let dismiss: (UUID) -> Void
    private let delivered: (UUID, NativeRuntimeInputCodec.Delivery) -> Void
    private var session: NativeRuntimeInputSession?
    private var forms: [UUID: Task<Void, Never>] = [:]
    private var replies: [UUID: PendingReply] = [:]
    private var running = false
    private var closed = false

    public init(socket: NativeRuntimeSocket, principal: String, capacity: Int,
                inputTimeout: Duration, ioTimeout: TimeInterval,
                collect: @escaping NativeRuntimeInputSession.Collect,
                dismiss: @escaping (UUID) -> Void,
                delivered: @escaping (UUID, NativeRuntimeInputCodec.Delivery) -> Void) throws {
        guard capacity > 0, inputTimeout > .zero, ioTimeout.isFinite, ioTimeout > 0 else {
            throw NativeInputError.invalidRequest
        }
        self.socket = socket
        self.principal = principal
        self.capacity = capacity
        self.inputTimeout = inputTimeout
        self.ioTimeout = ioTimeout
        self.collect = collect
        self.dismiss = dismiss
        self.delivered = delivered
    }

    /// One owner task must await this method, and cancel/close it on shutdown.
    /// readTimeout is an explicit connection liveness policy, not a form timeout.
    public func run(readTimeout: TimeInterval) async throws {
        guard !running, !closed else { throw NativeInputError.invalidRequest }
        running = true
        session = NativeRuntimeInputSession(principal: principal, collect: collect, dismiss: dismiss,
            exchange: { [weak self] bytes in
                guard let self else { throw NativeInputError.invalidRequest }
                return try await self.exchange(bytes)
            })
        defer { close() }
        while !closed && !Task.isCancelled {
            let frame = try await socket.readFrame(timeout: readTimeout)
            try receive(frame)
        }
    }

    public func close() {
        guard !closed else { return }
        closed = true
        socket.close()
        session?.disconnect()
        session = nil
        let tasks = forms.values
        forms.removeAll()
        for task in tasks { task.cancel() }
        let pending = replies.values
        replies.removeAll()
        for reply in pending {
            reply.deadline.cancel()
            reply.continuation.resume(throwing: NativeInputError.invalidRequest)
        }
    }

    private func receive(_ frame: Data) throws {
        let header: Header
        do { header = try JSONDecoder().decode(Header.self, from: frame) }
        catch { throw NativeInputError.invalidRequest }
        if header.topic == "astrid.v1.private.elicit.result" {
            guard let envelope = try? JSONDecoder().decode(Envelope.self, from: frame) else {
                throw NativeInputError.invalidRequest
            }
            guard let id = envelope.payload?.request_id, let pending = replies.removeValue(forKey: id) else { return }
            pending.deadline.cancel()
            pending.continuation.resume(returning: frame)
        } else if header.topic == "astrid.v1.private.elicit.request" {
            // Only explicitly native-owned requests open this form. Legacy
            // elicitation stays with its existing responder, even for secrets.
            let request = try NativeRuntimeInputCodec.request(frame, principal: principal)
            guard forms[request.id] == nil, forms.count < capacity, let session else {
                throw NativeInputError.invalidRequest
            }
            forms[request.id] = Task { [weak self] in
                guard let self else { return }
                defer { forms.removeValue(forKey: request.id) }
                do {
                    let result = try await session.handle(frame, timeout: inputTimeout)
                    if !closed { delivered(request.id, result) }
                } catch { close() }
            }
        }
    }

    private func exchange(_ bytes: Data) async throws -> Data {
        let envelope: Envelope
        do { envelope = try JSONDecoder().decode(Envelope.self, from: bytes) }
        catch { throw NativeInputError.invalidAnswer }
        guard !closed, let id = envelope.payload?.request_id, replies[id] == nil else {
            throw NativeInputError.invalidRequest
        }
        return try await withCheckedThrowingContinuation { continuation in
            let deadline = Task { [weak self, ioTimeout] in
                do { try await Task.sleep(for: .seconds(ioTimeout)) } catch { return }
                self?.close()
            }
            replies[id] = PendingReply(continuation: continuation, deadline: deadline)
            Task { [weak self, socket, ioTimeout] in
                do { try await socket.writeFrame(bytes, timeout: ioTimeout) }
                catch { self?.close() }
            }
        }
    }
}
