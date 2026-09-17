import Foundation
import Darwin

/// A discovered principal. Presence in this directory is not permission to act,
/// approve, or enter secrets.
public struct OwnedPrincipal: Equatable, Sendable, Identifiable, Decodable {
    public let id: String
    public let enabled: Bool

    public static func isValidID(_ value: String) -> Bool {
        guard (1...64).contains(value.utf8.count) else { return false }
        return value.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") }
    }
}

public enum PrincipalDiscovery: Equatable, Sendable {
    case owned([OwnedPrincipal])
    case unsupported
    case failed
}

public enum PrincipalChoice: Equatable, Sendable {
    case selected(OwnedPrincipal)
    case emptyDirectory
    case staleIdentifier(String)
    case invalidIdentifier
    case discoveryUnavailable
}

public enum PrincipalPicker {
    public static func choose(selected: String?, from discovery: PrincipalDiscovery) -> PrincipalChoice {
        switch discovery {
        case .unsupported, .failed:
            return .discoveryUnavailable
        case .owned(let principals):
            if principals.isEmpty { return .emptyDirectory }
            let trimmed = selected?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !trimmed.isEmpty, OwnedPrincipal.isValidID(trimmed), trimmed != "anonymous" else {
                return .invalidIdentifier
            }
            guard let match = principals.first(where: { $0.id == trimmed }), match.enabled else {
                return .staleIdentifier(trimmed)
            }
            return .selected(match)
        }
    }
}

public enum PrincipalDiscoveryCopy {
    public static let localOperatorExplanation =
        "This list is what the local operator credential can discover on this installation. It is not a hosted login. Viewing a library uses this machine’s local keys and does not change native input, approvals, or secrets."
    public static let emptyDirectory = "No owned principals are available to this operator."
    public static let unsupported =
        "This AOS runtime does not support owned-principal discovery. The library view is unavailable."
    public static let failed = "Couldn’t discover owned principals. Nothing was loaded."
    public static let stale = "The selected principal is no longer in the owned directory. Nothing was loaded."
    public static let invalid = "That is not an owned principal. Nothing was loaded."

    public static func message(for choice: PrincipalChoice, discovery: PrincipalDiscovery) -> String? {
        switch choice {
        case .selected: return nil
        case .emptyDirectory: return emptyDirectory
        case .staleIdentifier: return stale
        case .invalidIdentifier: return invalid
        case .discoveryUnavailable:
            return discovery == .unsupported ? unsupported : failed
        }
    }
}

/// Runs only an explicitly selected AOS executable's owned-principal discovery command.
public enum PrincipalDiscoveryReader {
    public static func read(binary: String, home: String) async throws -> PrincipalDiscovery {
        try await Task.detached {
            try readBlocking(binary: binary, home: home)
        }.value
    }

    static func readBlocking(binary: String, home: String, timeout: TimeInterval = 15) throws -> PrincipalDiscovery {
        guard binary.hasPrefix("/"), home.hasPrefix("/"), timeout > 0, timeout.isFinite else {
            throw OverviewError.invalidStatus
        }
        let pipe = Pipe()
        defer {
            try? pipe.fileHandleForReading.close()
            try? pipe.fileHandleForWriting.close()
        }
        let descriptor = pipe.fileHandleForReading.fileDescriptor
        let flags = fcntl(descriptor, F_GETFL)
        guard flags >= 0, fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
            throw OverviewError.invalidStatus
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = ["principals", "--json"]
        var environment = ProcessInfo.processInfo.environment
        environment["AOS_HOME"] = home
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        try process.run()
        defer {
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
        }
        try pipe.fileHandleForWriting.close()
        let started = ProcessInfo.processInfo.systemUptime
        var bytes = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            guard ProcessInfo.processInfo.systemUptime - started < timeout else {
                return .failed
            }
            let count = Darwin.read(descriptor, &buffer, buffer.count)
            if count > 0 {
                guard bytes.count + count <= 65_536 else { return .failed }
                bytes.append(contentsOf: buffer.prefix(count))
            } else if count == 0 {
                if !process.isRunning { break }
                Thread.sleep(forTimeInterval: 0.01)
            } else if errno == EINTR {
                continue
            } else if errno == EAGAIN || errno == EWOULDBLOCK {
                Thread.sleep(forTimeInterval: 0.01)
            } else {
                return .failed
            }
        }
        if process.terminationStatus == 2 { return .unsupported }
        guard process.terminationStatus == 0 else { return .failed }
        do {
            let document = try JSONDecoder().decode(OwnedDiscoveryDocument.self, from: bytes)
            try document.validate()
            return .owned(document.principals)
        } catch {
            return .failed
        }
    }
}

private struct OwnedDiscoveryDocument: Decodable {
    let scope: String
    let authority: String
    let principals: [OwnedPrincipal]

    func validate() throws {
        guard scope == "owned", authority == "discovery" else { throw OverviewError.invalidStatus }
        var seen = Set<String>()
        for row in principals {
            guard OwnedPrincipal.isValidID(row.id), row.id != "anonymous", seen.insert(row.id).inserted else {
                throw OverviewError.invalidStatus
            }
        }
    }
}
