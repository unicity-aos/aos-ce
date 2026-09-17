import CryptoKit
import Darwin
import Foundation

public enum NativeRuntimeConfigurationError: Error { case invalidConfiguration, unavailableCredential, credentialAbsent }

/// Explicit local connection settings, never secret values. Device pairing and
/// operator responder selection are separate prerequisites, not side effects.
public struct NativeRuntimeConfiguration: Codable, Sendable {
    public let socketPath: String
    public let principal: String
    public let privateKeyPath: String
    public let tokenPath: String
    public let capacity: Int
    public let inputTimeoutSeconds: Double
    public let ioTimeoutSeconds: Double
    public let readTimeoutSeconds: Double

    public static func load(path: String) throws -> Self {
        do {
            let config = try JSONDecoder().decode(Self.self, from: PrivateRuntimeFile.read(path, maximum: 16 * 1024))
            guard !config.principal.isEmpty, config.principal != "anonymous",
                  config.principal.utf8.count <= PresentationLimits.maxIDBytes,
                  config.principal.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) || "-_".unicodeScalars.contains($0) }),
                  config.capacity > 0,
                  [config.inputTimeoutSeconds, config.ioTimeoutSeconds, config.readTimeoutSeconds]
                    .allSatisfy({ $0.isFinite && $0 > 0 }),
                  [config.socketPath, config.privateKeyPath, config.tokenPath].allSatisfy(PrivateRuntimeFile.validPath) else {
                throw NativeRuntimeConfigurationError.invalidConfiguration
            }
            return config
        } catch { throw NativeRuntimeConfigurationError.invalidConfiguration }
    }

    /// Load only the explicitly selected files and authenticate without fallback.
    /// Neither key nor token is returned, printed, or encoded into UI snapshots.
    public func connect() async throws -> NativeRuntimeSocket {
        var keyBytes = Data()
        var tokenBytes = Data()
        var token = Data()
        defer {
            keyBytes.resetBytes(in: keyBytes.indices)
            tokenBytes.resetBytes(in: tokenBytes.indices)
            token.resetBytes(in: token.indices)
        }
        let key: Curve25519.Signing.PrivateKey
        do {
            keyBytes = try PrivateRuntimeFile.read(privateKeyPath, maximum: 32)
            guard keyBytes.count == 32 else { throw NativeRuntimeConfigurationError.unavailableCredential }
            key = try Curve25519.Signing.PrivateKey(rawRepresentation: keyBytes)
            do {
                tokenBytes = try PrivateRuntimeFile.read(tokenPath, maximum: 64)
            } catch NativeRuntimeConfigurationError.credentialAbsent {
                // The runtime retires this session file on stop. Retry a fresh
                // authentication after restart, never reuse the previous token.
                throw NativeRuntimeSocketError.unavailable
            }
            guard tokenBytes.count == 64 else { throw NativeRuntimeConfigurationError.unavailableCredential }
            func nibble(_ byte: UInt8) throws -> UInt8 {
                switch byte {
                case 48...57: return byte - 48
                case 97...102: return byte - 97 + 10
                default: throw NativeRuntimeConfigurationError.unavailableCredential
                }
            }
            for offset in stride(from: 0, to: 64, by: 2) {
                token.append(try nibble(tokenBytes[offset]) * 16 + nibble(tokenBytes[offset + 1]))
            }
        } catch NativeRuntimeSocketError.unavailable {
            throw NativeRuntimeSocketError.unavailable
        } catch { throw NativeRuntimeConfigurationError.unavailableCredential }
        let socket = try await NativeRuntimeSocket.connect(path: socketPath, timeout: ioTimeoutSeconds)
        try await socket.authenticate(principal: principal, token: token, signingKey: key, timeout: ioTimeoutSeconds)
        return socket
    }
}

/// Descriptor-relative traversal rejects symlink ancestors as well as leaves.
/// The opened leaf (not a prior path stat) must be private and owned by this user.
enum PrivateRuntimeFile {
    static func validPath(_ path: String) -> Bool {
        path.hasPrefix("/") && !path.contains("\0") &&
        path.split(separator: "/", omittingEmptySubsequences: false).dropFirst()
            .allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." }
    }

    static func read(_ path: String, maximum: Int) throws -> Data {
        guard validPath(path) else { throw NativeRuntimeConfigurationError.unavailableCredential }
        var fd = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard fd >= 0 else { throw NativeRuntimeConfigurationError.unavailableCredential }
        defer { Darwin.close(fd) }
        let parts = path.split(separator: "/")
        for (index, part) in parts.enumerated() {
            let flags = O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK | (index == parts.count - 1 ? 0 : O_DIRECTORY)
            let next = String(part).withCString { openat(fd, $0, flags) }
            guard next >= 0 else {
                if errno == ENOENT { throw NativeRuntimeConfigurationError.credentialAbsent }
                throw NativeRuntimeConfigurationError.unavailableCredential
            }
            Darwin.close(fd)
            fd = next
        }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_uid == getuid(), info.st_mode & 0o077 == 0,
              info.st_size > 0, info.st_size <= maximum else {
            throw NativeRuntimeConfigurationError.unavailableCredential
        }
        var bytes = [UInt8](repeating: 0, count: maximum + 1)
        defer { _ = bytes.withUnsafeMutableBytes { $0.initializeMemory(as: UInt8.self, repeating: 0) } }
        var count = 0
        while count < bytes.count {
            let remaining = bytes.count - count
            let n = bytes.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress!.advanced(by: count), remaining) }
            if n < 0 && errno == EINTR { continue }
            guard n >= 0 else { throw NativeRuntimeConfigurationError.unavailableCredential }
            if n == 0 { break }
            count += n
        }
        guard count > 0, count <= maximum else { throw NativeRuntimeConfigurationError.unavailableCredential }
        return Data(bytes.prefix(count))
    }
}
