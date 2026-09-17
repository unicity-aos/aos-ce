import Darwin
import Foundation
import Testing
@testable import AOSTrayCore

@Suite
struct SocketEndpointTests {
    @Test func rejectsUnsafePaths() throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }

        #expect(SocketEndpoint.validateBindPath("relative.sock") == .failure(.invalidPath))
        #expect(SocketEndpoint.validateBindPath("/tmp/") == .failure(.invalidPath))
        #expect(SocketEndpoint.validateBindPath("/private/tmp/./aos.sock") == .failure(.invalidPath))
        #expect(SocketEndpoint.validateBindPath("/private/tmp/../tmp/aos.sock") == .failure(.invalidPath))
        #expect(SocketEndpoint.validateBindPath(dir.url.appendingPathComponent("missing/nested.sock").path) == .failure(.missingDirectory))
        let openDir = dir.url.appendingPathComponent("open", isDirectory: true)
        try FileManager.default.createDirectory(at: openDir, withIntermediateDirectories: false)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: openDir.path)
        #expect(SocketEndpoint.validateBindPath(openDir.appendingPathComponent("aos.sock").path) == .failure(.directoryNotPrivate))

        let longName = dir.socketPath(String(repeating: "a", count: max(SocketEndpoint.maxPathBytes, 120)))
        #expect(SocketEndpoint.validateBindPath(longName) == .failure(.pathTooLong))

        let link = dir.url.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: dir.url)
        let throughLink = link.appendingPathComponent("app.sock").path
        #expect(SocketEndpoint.validateBindPath(throughLink) == .failure(.symlinkComponent))

        let ok = dir.socketPath("ok.sock")
        #expect(try SocketEndpoint.validateBindPath(ok).get() == ok)

        FileManager.default.createFile(atPath: ok, contents: Data())
        #expect(SocketEndpoint.validateBindPath(ok) == .failure(.endpointExists))
    }

    @Test func unlinkOnlyOwnInode() throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("owned.sock")
        FileManager.default.createFile(atPath: path, contents: Data("keep".utf8))
        let inode = try #require(SocketEndpoint.inode(at: path))
        SocketEndpoint.unlinkIfOwned(path, expected: SocketInode(device: inode.device, inode: inode.inode &+ 1))
        #expect(FileManager.default.fileExists(atPath: path))
        SocketEndpoint.unlinkIfOwned(path, expected: inode)
        #expect(!FileManager.default.fileExists(atPath: path))
    }
}

@Suite(.serialized)
struct RuntimePromptBrokerTests {
    @Test func lateClickAfterCancelNeverApproves() async throws {
        let broker = RuntimePromptBroker()
        let request = ValidatedPresentationRequest(
            id: "same-id",
            message: "Runtime-supplied explanation",
            options: ["Allow once", "Deny this"],
            timeoutSeconds: 30
        )
        async let first = broker.decide(request)
        try await waitUntil { !broker.currentPrompts().isEmpty }
        let promptID = try #require(broker.currentPrompts().first?.id)
        #expect(broker.currentPrompts().first?.options == ["Allow once", "Deny this"])
        broker.cancelAll()
        #expect(await first == nil)
        #expect(broker.currentPrompts().isEmpty)
        broker.select(promptID: promptID, index: 0)
        #expect(broker.currentPrompts().isEmpty)
    }

    @Test func collidingRequestIDsStayIndependent() async throws {
        let broker = RuntimePromptBroker()
        let request = ValidatedPresentationRequest(
            id: "same-id",
            message: "Runtime-supplied explanation",
            options: ["A", "B"],
            timeoutSeconds: 30
        )
        async let first = broker.decide(request)
        async let second = broker.decide(request)
        try await waitUntil { broker.currentPrompts().count == 2 }
        let ids = broker.currentPrompts().map(\.id)
        #expect(Set(ids).count == 2)
        #expect(broker.currentPrompts().allSatisfy { $0.requestID == "same-id" })
        broker.select(promptID: ids[0], index: 1)
        broker.select(promptID: ids[1], index: 0)
        let values = [await first, await second]
        #expect(Set(values) == [0, 1])
    }

    @Test func alreadyCancelledDecideDoesNotHang() async throws {
        let broker = RuntimePromptBroker()
        let request = ValidatedPresentationRequest(
            id: "cancel-race",
            message: "Runtime-supplied explanation",
            options: ["Allow", "Deny"],
            timeoutSeconds: 30
        )
        let task = Task { await broker.decide(request) }
        task.cancel()
        try await Task.sleep(nanoseconds: 20_000_000)
        let value = try await waitForValue(timeout: 1) { await task.value }
        #expect(value == nil)
        #expect(broker.currentPrompts().isEmpty)
        broker.select(promptID: "not-registered", index: 0)
        #expect(broker.currentPrompts().isEmpty)
    }

    @Test func waitingDecideHonorsTaskCancellation() async throws {
        let broker = RuntimePromptBroker()
        let request = ValidatedPresentationRequest(
            id: "waiting-cancel",
            message: "Runtime-supplied explanation",
            options: ["Allow", "Deny"],
            timeoutSeconds: 30
        )
        let task = Task { await broker.decide(request) }
        try await waitUntil { !broker.currentPrompts().isEmpty }
        #expect(broker.currentPrompts().first?.options == ["Allow", "Deny"])
        task.cancel()
        let value = try await waitForValue(timeout: 1) { await task.value }
        #expect(value == nil)
        #expect(broker.currentPrompts().isEmpty)
    }
}

@Suite(.serialized)
struct UnixPresentationServerTests {
    @Test func acceptSelectsProvidedIndex() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("accept.sock")
        let server = UnixPresentationServer(path: path, responder: ScriptedResponder { _ in 0 })
        try server.start()
        defer { server.stop() }

        #expect(SocketEndpoint.inode(at: path) != nil)
        try assertMode(path, 0o600)

        let response = try await UnixTestClient.transact(
            path: path,
            payload: requestJSON(id: "correlation-id", message: "Runtime-supplied explanation", options: ["Allow", "Deny"], timeout: 30)
        )
        #expect(response["id"] as? String == "correlation-id")
        #expect((response["selected"] as? NSNumber)?.intValue == 0)
        #expect((response["version"] as? NSNumber)?.intValue == 1)
    }

    @Test func independentConnectionsDoNotShareDecisions() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("multi.sock")
        let gate = ConnectionGate(count: 2)
        let server = UnixPresentationServer(path: path, responder: ScriptedResponder { request in
            await gate.arrive()
            return request.message == "one" ? 0 : 1
        })
        try server.start()
        defer { server.stop() }
        #expect(SocketEndpoint.inode(at: path) != nil)

        async let first = UnixTestClient.transactAsync(
            path: path,
            payload: requestJSON(id: "same-id", message: "one", options: ["Allow", "Deny"], timeout: 30)
        )
        async let second = UnixTestClient.transactAsync(
            path: path,
            payload: requestJSON(id: "same-id", message: "two", options: ["Allow", "Deny"], timeout: 30)
        )
        let responses = [try await first, try await second]
        let selected = Set(responses.compactMap { ($0["selected"] as? NSNumber)?.intValue })
        #expect(selected == [0, 1])
        #expect(responses.allSatisfy { $0["id"] as? String == "same-id" })
    }

    @Test func brokerTimeoutWritesNullAndRemovesPrompt() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("broker-timeout.sock")
        let broker = RuntimePromptBroker()
        let server = UnixPresentationServer(path: path, responder: broker)
        try server.start()
        defer {
            broker.cancelAll()
            server.stop()
        }
        #expect(SocketEndpoint.inode(at: path) != nil)

        async let response = UnixTestClient.transactAsync(
            path: path,
            payload: requestJSON(
                id: "broker-timeout",
                message: "Runtime-supplied explanation",
                options: ["Allow", "Deny"],
                timeout: 1
            ),
            waitSeconds: 4
        )
        try await waitUntil { !broker.currentPrompts().isEmpty }
        #expect(broker.currentPrompts().first?.requestID == "broker-timeout")
        #expect(broker.currentPrompts().first?.options == ["Allow", "Deny"])
        let body = try await response
        #expect(body["id"] as? String == "broker-timeout")
        #expect(body["selected"] is NSNull)
        try await waitUntil(timeout: 1) { broker.currentPrompts().isEmpty }
        #expect(broker.currentPrompts().isEmpty)
    }

    @Test func timeoutWritesNullAndNeverApproves() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("timeout.sock")
        let server = UnixPresentationServer(
            path: path,
            responder: ScriptedResponder { _ in
                try? await Task.sleep(nanoseconds: 30_000_000_000)
                return 0
            }
        )
        try server.start()
        defer { server.stop() }
        #expect(SocketEndpoint.inode(at: path) != nil)

        let response = try await UnixTestClient.transact(
            path: path,
            payload: requestJSON(id: "t", message: "timeout please", options: ["Allow"], timeout: 1),
            waitSeconds: 4
        )
        #expect(response["id"] as? String == "t")
        #expect(response["selected"] is NSNull)
    }

    @Test func malformedFrameDoesNotWriteApproval() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("bad.sock")
        let server = UnixPresentationServer(path: path, responder: ScriptedResponder { _ in 0 })
        try server.start()
        defer { server.stop() }
        #expect(SocketEndpoint.inode(at: path) != nil)

        let payload = try await UnixTestClient.transactRaw(path: path, payload: "{not-json}\n", waitSeconds: 1)
        #expect(payload == nil || payload?.isEmpty == true)
    }

    @Test func existingEndpointIsRefusedWithoutUnlink() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("exists.sock")
        let first = UnixPresentationServer(path: path, responder: ScriptedResponder { _ in 0 })
        try first.start()
        defer { first.stop() }
        #expect(SocketEndpoint.inode(at: path) != nil)
        let inode = try #require(SocketEndpoint.inode(at: path))

        let second = UnixPresentationServer(path: path, responder: ScriptedResponder { _ in 1 })
        do {
            try second.start()
            Issue.record("second listener should not bind an existing endpoint")
            second.stop()
        } catch let error as SocketEndpointError {
            #expect(error == .endpointExists)
        }
        #expect(SocketEndpoint.inode(at: path) == inode)

        let response = try await UnixTestClient.transact(
            path: path,
            payload: requestJSON(id: "still-first", message: "hello", options: ["Allow"], timeout: 30)
        )
        #expect((response["selected"] as? NSNumber)?.intValue == 0)
    }

    @Test func stopRemovesOnlyOwnedEndpoint() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("cleanup.sock")
        let server = UnixPresentationServer(path: path, responder: ScriptedResponder { _ in nil })
        try server.start()
        #expect(SocketEndpoint.inode(at: path) != nil)
        server.stop()
        #expect(SocketEndpoint.inode(at: path) == nil)
        #expect(!FileManager.default.fileExists(atPath: path))
    }

    @Test func silentClientStopCleansUpWithoutWaitingForReadTimeout() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("silent.sock")
        let approved = ApprovalFlag()
        let server = UnixPresentationServer(
            path: path,
            responder: ScriptedResponder { _ in
                approved.mark()
                return 0
            }
        )
        try server.start()
        #expect(SocketEndpoint.inode(at: path) != nil)

        let fd = try UnixTestClient.connect(path)
        defer { Darwin.close(fd) }
        try await Task.sleep(nanoseconds: 100_000_000)

        let start = DispatchTime.now().uptimeNanoseconds
        server.stop()
        let elapsedNs = DispatchTime.now().uptimeNanoseconds &- start
        #expect(elapsedNs < 1_000_000_000)
        #expect(SocketEndpoint.inode(at: path) == nil)
        #expect(!FileManager.default.fileExists(atPath: path))
        #expect(approved.value == false)

        var pollFD = pollfd(fd: fd, events: Int16(POLLIN) | Int16(POLLHUP), revents: 0)
        let status = poll(&pollFD, 1, 1000)
        #expect(status > 0)
        var byte: UInt8 = 0
        let n = Darwin.read(fd, &byte, 1)
        #expect(n <= 0)
    }

    @Test func disconnectCancelsWithoutApproval() async throws {
        let dir = try PrivateSocketDir.make()
        defer { dir.remove() }
        let path = dir.socketPath("hangup.sock")
        let approved = ApprovalFlag()
        let server = UnixPresentationServer(
            path: path,
            responder: ScriptedResponder { _ in
                do {
                    try await Task.sleep(nanoseconds: 2_000_000_000)
                    approved.mark()
                    return 0
                } catch {
                    return nil
                }
            }
        )
        try server.start()
        defer { server.stop() }
        #expect(SocketEndpoint.inode(at: path) != nil)

        let fd = try UnixTestClient.connect(path)
        try UnixTestClient.send(fd, requestJSON(id: "h", message: "hangup", options: ["Allow"], timeout: 30) + "\n")
        Darwin.close(fd)
        try await Task.sleep(nanoseconds: 400_000_000)
        #expect(approved.value == false)
    }
}

private final class ApprovalFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var marked = false
    func mark() {
        lock.lock()
        marked = true
        lock.unlock()
    }
    var value: Bool {
        lock.lock()
        defer { lock.unlock() }
        return marked
    }
}

private final class ScriptedResponder: PromptResponder, @unchecked Sendable {
    let handler: @Sendable (ValidatedPresentationRequest) async -> Int?

    init(_ handler: @escaping @Sendable (ValidatedPresentationRequest) async -> Int?) {
        self.handler = handler
    }

    func decide(_ request: ValidatedPresentationRequest) async -> Int? {
        await handler(request)
    }
}

private actor ConnectionGate {
    private let needed: Int
    private var arrived = 0
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(count: Int) {
        needed = count
    }

    func arrive() async {
        arrived += 1
        if arrived >= needed {
            let waiters = self.waiters
            self.waiters = []
            waiters.forEach { $0.resume() }
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }
}

private struct PrivateSocketDir {
    var url: URL

    static func make() throws -> PrivateSocketDir {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        let url = base.appendingPathComponent("aos-tray-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: url.path)
        return PrivateSocketDir(url: url)
    }

    func socketPath(_ name: String) -> String {
        url.appendingPathComponent(name).path
    }

    func remove() {
        try? FileManager.default.removeItem(at: url)
    }
}

private enum UnixTestClient {
    static func transact(path: String, payload: String, waitSeconds: Int = 5) async throws -> [String: Any] {
        try await transactAsync(path: path, payload: payload, waitSeconds: waitSeconds)
    }

    static func transactAsync(path: String, payload: String, waitSeconds: Int = 5) async throws -> [String: Any] {
        let line = try await Task.detached {
            try transactLine(path: path, payload: payload, waitSeconds: waitSeconds)
        }.value
        let data = try #require(line.data(using: .utf8))
        return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    static func transactLine(path: String, payload: String, waitSeconds: Int) throws -> String {
        let body = payload.hasSuffix("\n") ? payload : payload + "\n"
        guard let line = try transactRawBlocking(path: path, payload: body, waitSeconds: waitSeconds) else {
            throw WaitTimeout()
        }
        return line
    }

    static func transactRaw(path: String, payload: String, waitSeconds: Int) async throws -> String? {
        try await Task.detached {
            try transactRawBlocking(path: path, payload: payload, waitSeconds: waitSeconds)
        }.value
    }

    static func transactRawBlocking(path: String, payload: String, waitSeconds: Int) throws -> String? {
        let fd = try connect(path)
        defer { Darwin.close(fd) }
        try send(fd, payload)
        return readLine(fd, waitSeconds: waitSeconds)
    }

    static func connect(_ path: String) throws -> Int32 {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SocketListenerError.bindFailed }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let copied = path.withCString { cString -> Bool in
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
        let status = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                Darwin.connect(fd, sockaddrPointer, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard status == 0 else {
            Darwin.close(fd)
            throw SocketListenerError.bindFailed
        }
        return fd
    }

    static func send(_ fd: Int32, _ text: String) throws {
        let data = Data(text.utf8)
        let ok = data.withUnsafeBytes { raw -> Bool in
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
        if !ok { throw SocketListenerError.bindFailed }
    }

    static func readLine(_ fd: Int32, waitSeconds: Int) -> String? {
        var buffer: [UInt8] = []
        let deadline = Date().addingTimeInterval(TimeInterval(waitSeconds))
        while Date() < deadline {
            var pollFD = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            let remaining = max(1, Int32(deadline.timeIntervalSinceNow * 1000))
            let status = poll(&pollFD, 1, remaining)
            if status <= 0 { continue }
            var byte: UInt8 = 0
            let n = Darwin.read(fd, &byte, 1)
            if n < 0 {
                if errno == EINTR { continue }
                return nil
            }
            if n == 0 { return buffer.isEmpty ? nil : String(bytes: buffer, encoding: .utf8) }
            if byte == 0x0A {
                return String(bytes: buffer, encoding: .utf8)
            }
            buffer.append(byte)
        }
        return nil
    }
}

private func requestJSON(id: String, message: String, options: [String], timeout: Int) -> String {
    let labels = options.map { "{\"label\":\"\($0)\"}" }.joined(separator: ",")
    return "{\"version\":1,\"id\":\"\(id)\",\"message\":\"\(message)\",\"options\":[\(labels)],\"timeoutSeconds\":\(timeout)}"
}

private func assertMode(_ path: String, _ mode: mode_t) throws {
    var info = stat()
    guard lstat(path, &info) == 0 else {
        Issue.record("missing socket at \(path)")
        return
    }
    #expect(info.st_mode & 0o777 == mode)
}

private struct WaitTimeout: Error {}

private final class DeadlineWait: @unchecked Sendable {
    private let lock = NSLock()
    private var resumed = false
    private let condition: @Sendable () -> Bool
    private let deadline: Date
    private let queue = DispatchQueue.global(qos: .userInitiated)

    init(timeout: TimeInterval, condition: @escaping @Sendable () -> Bool) {
        self.deadline = Date().addingTimeInterval(timeout)
        self.condition = condition
    }

    func wait(_ continuation: CheckedContinuation<Void, Error>) {
        tick(continuation)
    }

    private func tick(_ continuation: CheckedContinuation<Void, Error>) {
        if condition() {
            resume(.success(()), continuation)
            return
        }
        if Date() >= deadline {
            resume(.failure(WaitTimeout()), continuation)
            return
        }
        queue.asyncAfter(deadline: .now() + .milliseconds(20)) { [self] in
            self.tick(continuation)
        }
    }

    private func resume(_ result: Result<Void, Error>, _ continuation: CheckedContinuation<Void, Error>) {
        lock.lock()
        defer { lock.unlock() }
        guard !resumed else { return }
        resumed = true
        continuation.resume(with: result)
    }
}

private func waitUntil(
    timeout: TimeInterval = 2,
    _ condition: @escaping @Sendable () -> Bool
) async throws {
    if condition() { return }
    let waiter = DeadlineWait(timeout: timeout, condition: condition)
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
        waiter.wait(continuation)
    }
}

private func waitForValue<T: Sendable>(
    timeout: TimeInterval,
    _ body: @escaping @Sendable () async -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { await body() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
            throw WaitTimeout()
        }
        guard let value = try await group.next() else {
            throw WaitTimeout()
        }
        group.cancelAll()
        while let _ = try? await group.next() {}
        return value
    }
}
