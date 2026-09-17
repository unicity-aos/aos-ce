import Darwin
import Foundation

public enum NativeRuntimeSocketError: Error { case unavailable, invalidFrame, authenticationFailed }

/// Length-prefixed local IPC. The owner must pin the runtime endpoint and
/// authenticate before using it for native input. No discovery or fallback.
public final class NativeRuntimeSocket: @unchecked Sendable {
    private let lock = NSLock()
    private var fd: Int32
    private let readQueue = DispatchQueue(label: "aos.native-input.read")
    private let writeQueue = DispatchQueue(label: "aos.native-input.write")
    // Astrid's native uplink framing ceiling, not an application queue limit.
    private static let maxFrameBytes = 2 * 1024 * 1024

    private init(fd: Int32) { self.fd = fd }

    public static func connect(path: String, timeout: TimeInterval) async throws -> NativeRuntimeSocket {
        guard timeout.isFinite, timeout > 0 else { throw NativeRuntimeSocketError.unavailable }
        let channel = try await Task.detached {
            try open(path: path, timeout: timeout)
        }.value
        if Task.isCancelled { channel.close(); throw CancellationError() }
        return channel
    }

    private static func open(path: String, timeout: TimeInterval) throws -> NativeRuntimeSocket {
        guard path.hasPrefix("/"), !path.contains("\0"), path.utf8.count <= SocketEndpoint.maxPathBytes else {
            throw NativeRuntimeSocketError.unavailable
        }
        var accumulated = ""
        let components = path.split(separator: "/", omittingEmptySubsequences: false)
        for component in components.dropFirst() {
            guard !component.isEmpty, component != ".", component != ".." else {
                throw NativeRuntimeSocketError.unavailable
            }
            accumulated += "/" + component
            var info = stat()
            guard lstat(accumulated, &info) == 0, info.st_mode & S_IFMT != S_IFLNK else {
                throw NativeRuntimeSocketError.unavailable
            }
        }
        var info = stat()
        guard lstat(path, &info) == 0, info.st_mode & S_IFMT == S_IFSOCK,
              info.st_uid == getuid(), info.st_mode & 0o077 == 0 else {
            throw NativeRuntimeSocketError.unavailable
        }
        let socket = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard socket >= 0 else { throw NativeRuntimeSocketError.unavailable }
        let channel = NativeRuntimeSocket(fd: socket)
        do {
            guard fcntl(socket, F_SETFL, O_NONBLOCK) == 0,
                  fcntl(socket, F_SETFD, FD_CLOEXEC) == 0 else { throw NativeRuntimeSocketError.unavailable }
            var noSignal: Int32 = 1
            guard setsockopt(socket, SOL_SOCKET, SO_NOSIGPIPE, &noSignal,
                             socklen_t(MemoryLayout<Int32>.size)) == 0 else { throw NativeRuntimeSocketError.unavailable }
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            path.withCString { source in
                withUnsafeMutableBytes(of: &address.sun_path) { target in
                    target.copyMemory(from: UnsafeRawBufferPointer(start: source, count: strlen(source) + 1))
                }
            }
            let status = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(socket, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            if status != 0 {
                guard errno == EINPROGRESS else { throw NativeRuntimeSocketError.unavailable }
                try ready(socket, events: Int16(POLLOUT), deadline: ProcessInfo.processInfo.systemUptime + timeout)
                var error: Int32 = 0
                var length = socklen_t(MemoryLayout<Int32>.size)
                guard getsockopt(socket, SOL_SOCKET, SO_ERROR, &error, &length) == 0, error == 0 else {
                    throw NativeRuntimeSocketError.unavailable
                }
            }
            var uid: uid_t = 0
            var gid: gid_t = 0
            guard getpeereid(socket, &uid, &gid) == 0, uid == getuid() else {
                throw NativeRuntimeSocketError.unavailable
            }
            return channel
        } catch { channel.close(); throw NativeRuntimeSocketError.unavailable }
    }

    public func close() {
        lock.lock()
        let old = fd
        fd = -1
        if old >= 0 { Darwin.shutdown(old, SHUT_RDWR); Darwin.close(old) }
        lock.unlock()
    }
    deinit { close() }

    private func duplicate() throws -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        guard fd >= 0 else { throw NativeRuntimeSocketError.unavailable }
        let copy = fcntl(fd, F_DUPFD_CLOEXEC, 0)
        guard copy >= 0 else { throw NativeRuntimeSocketError.unavailable }
        return copy
    }

    public func readFrame(timeout: TimeInterval) async throws -> Data {
        try await perform(on: readQueue, timeout: timeout) { fd, deadline in
            let prefix = try Self.readExactly(fd, count: 4, deadline: deadline)
            let size = prefix.reduce(0) { ($0 << 8) | Int($1) }
            guard size > 0, size <= Self.maxFrameBytes else { throw NativeRuntimeSocketError.invalidFrame }
            return try Self.readExactly(fd, count: size, deadline: deadline)
        }
    }

    public func writeFrame(_ body: Data, timeout: TimeInterval) async throws {
        guard !body.isEmpty, body.count <= Self.maxFrameBytes else { throw NativeRuntimeSocketError.invalidFrame }
        let size = UInt32(body.count)
        let prefix = Data([UInt8(size >> 24), UInt8(truncatingIfNeeded: size >> 16),
                           UInt8(truncatingIfNeeded: size >> 8), UInt8(truncatingIfNeeded: size)])
        let bytes = prefix + body
        _ = try await perform(on: writeQueue, timeout: timeout) { fd, deadline in
            try bytes.withUnsafeBytes { raw in
                var offset = 0
                while offset < bytes.count {
                    try Self.ready(fd, events: Int16(POLLOUT), deadline: deadline)
                    let count = Darwin.write(fd, raw.baseAddress!.advanced(by: offset), bytes.count - offset)
                    if count < 0 && (errno == EINTR || errno == EAGAIN) { continue }
                    guard count > 0 else { throw NativeRuntimeSocketError.unavailable }
                    offset += count
                }
            }
            return Data()
        }
    }

    private func perform(on queue: DispatchQueue, timeout: TimeInterval,
                         operation: @escaping @Sendable (Int32, TimeInterval) throws -> Data) async throws -> Data {
        guard timeout.isFinite, timeout > 0 else { throw NativeRuntimeSocketError.unavailable }
        let deadline = ProcessInfo.processInfo.systemUptime + timeout
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                queue.async {
                    do {
                        let copy = try self.duplicate()
                        defer { Darwin.close(copy) }
                        continuation.resume(returning: try operation(copy, deadline))
                    } catch {
                        // A partial read/write cannot be safely retried as another frame.
                        self.close()
                        continuation.resume(throwing: error)
                    }
                }
            }
        } onCancel: { self.close() }
    }

    private static func ready(_ fd: Int32, events: Int16, deadline: TimeInterval) throws {
        while true {
            let remaining = deadline - ProcessInfo.processInfo.systemUptime
            guard remaining > 0 else { throw NativeRuntimeSocketError.unavailable }
            var descriptor = pollfd(fd: fd, events: events, revents: 0)
            let result = poll(&descriptor, 1, Int32(min(remaining * 1000 + 1, Double(Int32.max))))
            if result < 0 && errno == EINTR { continue }
            guard result > 0, descriptor.revents & events != 0 else { throw NativeRuntimeSocketError.unavailable }
            return
        }
    }

    private static func readExactly(_ fd: Int32, count: Int, deadline: TimeInterval) throws -> Data {
        var result = Data(count: count)
        try result.withUnsafeMutableBytes { raw in
            var offset = 0
            while offset < count {
                try ready(fd, events: Int16(POLLIN), deadline: deadline)
                let n = Darwin.read(fd, raw.baseAddress!.advanced(by: offset), count - offset)
                if n < 0 && (errno == EINTR || errno == EAGAIN) { continue }
                guard n > 0 else { throw NativeRuntimeSocketError.unavailable }
                offset += n
            }
        }
        return result
    }
}
