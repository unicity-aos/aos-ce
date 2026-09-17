import Darwin
import Foundation

public protocol PromptResponder: Sendable {
    func decide(_ request: ValidatedPresentationRequest) async -> Int?
}

public final class UnixPresentationServer: @unchecked Sendable {
    private static let initialReadTimeoutMs: Int32 = 5_000

    private let path: String
    private let responder: any PromptResponder
    private let acceptQueue = DispatchQueue(label: "ai.unicity.aos.tray.socket.accept")
    private let ioQueue = DispatchQueue(label: "ai.unicity.aos.tray.socket.io", attributes: .concurrent)
    private let stateLock = NSLock()
    private var listenFD: Int32 = -1
    private var clientFDs: Set<Int32> = []
    private var ownedInode: SocketInode?
    private var stopping = false

    public init(path: String, responder: any PromptResponder) {
        self.path = path
        self.responder = responder
    }

    public func start() throws {
        let validated = try SocketEndpoint.validateBindPath(path).get()
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else {
            throw SocketListenerError.bindFailed
        }
        var noSigPipe: Int32 = 1
        _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))

        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let copied = validated.withCString { cString -> Bool in
            withUnsafeMutableBytes(of: &address.sun_path) { raw in
                let count = strlen(cString)
                guard count < raw.count else { return false }
                raw.copyMemory(from: UnsafeRawBufferPointer(start: cString, count: count + 1))
                return true
            }
        }
        guard copied else {
            Darwin.close(fd)
            throw SocketEndpointError.pathTooLong
        }

        let bindStatus = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                Darwin.bind(fd, sockaddrPointer, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        if bindStatus != 0 {
            Darwin.close(fd)
            throw SocketListenerError.bindFailed
        }
        guard SocketEndpoint.chmodSocket(validated) else {
            _ = Darwin.unlink(validated)
            Darwin.close(fd)
            throw SocketListenerError.bindFailed
        }
        guard Darwin.listen(fd, 16) == 0, let inode = SocketEndpoint.inode(at: validated) else {
            _ = Darwin.unlink(validated)
            Darwin.close(fd)
            throw SocketListenerError.bindFailed
        }

        stateLock.lock()
        listenFD = fd
        ownedInode = inode
        stopping = false
        stateLock.unlock()

        acceptQueue.async { [weak self] in
            self?.acceptLoop(listenFD: fd)
        }
    }

    public func stop() {
        stateLock.lock()
        stopping = true
        let fd = listenFD
        listenFD = -1
        let clients = Array(clientFDs)
        clientFDs.removeAll()
        let inode = ownedInode
        ownedInode = nil
        stateLock.unlock()
        if fd >= 0 {
            Darwin.shutdown(fd, SHUT_RDWR)
            Darwin.close(fd)
        }
        for client in clients {
            Darwin.shutdown(client, SHUT_RDWR)
            Darwin.close(client)
        }
        if let inode {
            SocketEndpoint.unlinkIfOwned(path, expected: inode)
        }
    }

    deinit {
        stop()
    }

    private func acceptLoop(listenFD: Int32) {
        while true {
            stateLock.lock()
            let stopping = self.stopping
            stateLock.unlock()
            if stopping { return }

            let client = Darwin.accept(listenFD, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                return
            }
            var noSigPipe: Int32 = 1
            _ = setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))
            if !registerClient(client) {
                Darwin.close(client)
                return
            }
            Task {
                await self.serve(client)
            }
        }
    }

    private func registerClient(_ fd: Int32) -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        if stopping {
            return false
        }
        clientFDs.insert(fd)
        return true
    }

    private func closeIfOwned(_ fd: Int32) {
        stateLock.lock()
        let owned = clientFDs.remove(fd) != nil
        stateLock.unlock()
        if owned {
            Darwin.close(fd)
        }
    }

    private func serve(_ fd: Int32) async {
        defer { closeIfOwned(fd) }
        guard sameUserPeer(fd) else { return }
        switch await readFrameAsync(fd) {
        case .failure:
            return
        case .success(let frame):
            switch PresentationCodec.parseRequest(frame) {
            case .failure:
                return
            case .success(let request):
                let selected = await decision(for: request, fd: fd)
                let bounded = PresentationCodec.selection(selected, optionCount: request.options.count)
                if let payload = try? PresentationCodec.encodeResponse(
                    PresentationResponse(id: request.id, selected: bounded)
                ) {
                    _ = await writeAllAsync(fd, payload + Data([0x0A]))
                }
            }
        }
    }

    private func decision(for request: ValidatedPresentationRequest, fd: Int32) async -> Int? {
        let winner = FirstDecision()
        let decideTask = Task {
            winner.finish(.choice(await self.responder.decide(request)))
        }
        let timeoutTask = Task {
            do {
                try await Task.sleep(nanoseconds: UInt64(request.timeoutSeconds) * 1_000_000_000)
                winner.finish(.timeout)
            } catch {
                return
            }
        }
        let hangupTask = Task {
            await self.waitForHangup(fd)
            winner.finish(.hangup)
        }
        let event = await winner.value()
        decideTask.cancel()
        timeoutTask.cancel()
        hangupTask.cancel()
        switch event {
        case .choice(let selected):
            return selected
        case .timeout, .hangup:
            return nil
        }
    }

    private func waitForHangup(_ fd: Int32) async {
        let wait = HangupWait()
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                wait.attach(continuation)
                ioQueue.async {
                    var pollFD = pollfd(fd: fd, events: Int16(POLLHUP) | Int16(POLLIN), revents: 0)
                    while !wait.isCancelled {
                        let status = poll(&pollFD, 1, 200)
                        if status < 0 {
                            if errno == EINTR { continue }
                            break
                        }
                        if status == 0 { continue }
                        if pollFD.revents & (Int16(POLLHUP) | Int16(POLLERR) | Int16(POLLNVAL)) != 0 {
                            break
                        }
                        if pollFD.revents & Int16(POLLIN) != 0 {
                            var byte: UInt8 = 0
                            let n = Darwin.read(fd, &byte, 1)
                            if n <= 0 { break }
                        }
                    }
                    wait.finish()
                }
            }
        } onCancel: {
            wait.cancel()
        }
    }

    private func sameUserPeer(_ fd: Int32) -> Bool {
        var uid: uid_t = 0
        var gid: gid_t = 0
        guard getpeereid(fd, &uid, &gid) == 0 else { return false }
        return uid == getuid()
    }

    private func readFrameAsync(_ fd: Int32) async -> Result<Data, ProtocolError> {
        await withCheckedContinuation { continuation in
            ioQueue.async {
                continuation.resume(returning: self.readFrame(fd, timeoutMs: Self.initialReadTimeoutMs))
            }
        }
    }

    private func writeAllAsync(_ fd: Int32, _ data: Data) async -> Bool {
        await withCheckedContinuation { continuation in
            ioQueue.async {
                continuation.resume(returning: self.writeAll(fd, data))
            }
        }
    }

    private func readFrame(_ fd: Int32, timeoutMs: Int32) -> Result<Data, ProtocolError> {
        var buffer: [UInt8] = []
        buffer.reserveCapacity(256)
        let start = DispatchTime.now().uptimeNanoseconds
        let timeoutNs = UInt64(timeoutMs) * 1_000_000

        func remainingTimeoutMs() -> Int32 {
            let elapsed = DispatchTime.now().uptimeNanoseconds &- start
            if elapsed >= timeoutNs { return 0 }
            return Int32((timeoutNs &- elapsed) / 1_000_000)
        }

        while buffer.count <= PresentationLimits.maxFrameBytes {
            let remaining = remainingTimeoutMs()
            if remaining <= 0 {
                return .failure(.malformedFrame)
            }
            var pollFD = pollfd(fd: fd, events: Int16(POLLIN) | Int16(POLLHUP), revents: 0)
            let status = poll(&pollFD, 1, remaining)
            if status < 0 {
                if errno == EINTR { continue }
                return .failure(.malformedFrame)
            }
            if status == 0 {
                return .failure(.malformedFrame)
            }
            if pollFD.revents & (Int16(POLLERR) | Int16(POLLNVAL)) != 0 {
                return .failure(.malformedFrame)
            }
            if pollFD.revents & Int16(POLLIN) != 0 {
                var byte: UInt8 = 0
                let n = Darwin.read(fd, &byte, 1)
                if n < 0 {
                    if errno == EINTR { continue }
                    return .failure(.malformedFrame)
                }
                if n == 0 {
                    return .failure(.malformedFrame)
                }
                if byte == 0x0A {
                    if buffer.isEmpty { return .failure(.malformedFrame) }
                    return .success(Data(buffer))
                }
                buffer.append(byte)
                if buffer.count > PresentationLimits.maxFrameBytes {
                    return .failure(.frameTooLarge)
                }
                continue
            }
            if pollFD.revents & Int16(POLLHUP) != 0 {
                return .failure(.malformedFrame)
            }
        }
        return .failure(.frameTooLarge)
    }

    private func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        data.withUnsafeBytes { raw in
            var offset = 0
            let total = raw.count
            let base = raw.bindMemory(to: UInt8.self).baseAddress
            while offset < total {
                let n = Darwin.write(fd, base?.advanced(by: offset), total - offset)
                if n < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                if n == 0 { return false }
                offset += n
            }
            return true
        }
    }
}

private enum DecisionEvent {
    case choice(Int?)
    case timeout
    case hangup
}

private final class FirstDecision: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<DecisionEvent, Never>?
    private var event: DecisionEvent?

    func finish(_ event: DecisionEvent) {
        lock.lock()
        guard self.event == nil else {
            lock.unlock()
            return
        }
        self.event = event
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume(returning: event)
    }

    func value() async -> DecisionEvent {
        await withCheckedContinuation { continuation in
            lock.lock()
            if let event = self.event {
                lock.unlock()
                continuation.resume(returning: event)
            } else {
                self.continuation = continuation
                lock.unlock()
            }
        }
    }
}

public enum SocketListenerError: Equatable, Error, Sendable {
    case bindFailed

    public var message: String {
        switch self {
        case .bindFailed:
            return "failed to bind Unix socket"
        }
    }
}

private final class HangupWait: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false
    private var finished = false
    private var continuation: CheckedContinuation<Void, Never>?

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }

    func attach(_ continuation: CheckedContinuation<Void, Never>) {
        lock.lock()
        if finished || cancelled {
            lock.unlock()
            continuation.resume()
            return
        }
        self.continuation = continuation
        lock.unlock()
    }

    func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
        finish()
    }

    func finish() {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        finished = true
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume()
    }
}
