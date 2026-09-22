import Foundation

public struct UpdateInventory: Decodable, Sendable {
    public let schemaVersion: Int
    public let channel: String
    public let checkedAt: UInt64?
    public let items: [UpdateItem]
    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", channel, checkedAt = "checked_at", items
    }
    public static func decode(_ data: Data) throws -> Self {
        let value = try JSONDecoder().decode(Self.self, from: data)
        guard value.schemaVersion == 1,
              ["stable", "dev", "nightly"].contains(value.channel),
              value.items.count <= 256,
              Set(value.items.map(\.id)).count == value.items.count else {
            throw UpdateError.invalidResponse
        }
        return value
    }
}

public struct UpdateItem: Decodable, Sendable, Identifiable {
    public let id: String
    public let name: String
    public let installedVersion: String
    public let candidateVersion: String?
    public let availability: String
    public let verification: String
    public let action: String
    public let message: String
    enum CodingKeys: String, CodingKey {
        case id, name, installedVersion = "installed_version", candidateVersion = "candidate_version"
        case availability, verification, action, message
    }
    public var canApply: Bool { availability == "available" && action == "apply" }
    public var label: String {
        switch availability {
        case "available": "Update available"
        case "current": "Up to date"
        case "ahead": "Newer than this channel"
        case "activation_required": "Installed · activation not confirmed"
        case "failed": "Check or update failed"
        case "managed": "Managed by AOS"
        case "unsupported": "Manual update"
        case "stale": "Check again · previous result expired"
        default: "Not checked"
        }
    }
}

public enum UpdateError: Error, LocalizedError {
    case invalidResponse
    case operation(String)
    public var errorDescription: String? {
        switch self {
        case .invalidResponse: "Update information is unavailable. Check that this AOS version supports Command Center updates, or run aos updates check."
        case .operation(let message): message
        }
    }
}

public enum UpdateCommand: Sendable {
    case refresh
    case list
    case check(channel: String)
    case apply(selection: String, channel: String? = nil)
    case capsules(principal: String)
    public var retryCommand: UpdateCommand {
        switch self {
        case .apply(_, let channel): .check(channel: channel ?? "stable")
        default: self
        }
    }
    var arguments: [String] {
        switch self {
        case .refresh: ["updates", "refresh", "--json"]
        case .list: ["updates", "list", "--json"]
        case .check(let channel): ["updates", "check", "--channel=\(channel)", "--json"]
        case .apply(let selection, let channel): ["updates", "apply", selection, "--yes", "--json"] + (channel.map { ["--expected-channel=\($0)"] } ?? [])
        case .capsules(let principal): ["updates", "capsules", "--principal=\(principal)", "--json"]
        }
    }
}

public enum UpdateCommandReader {
    static func decodeResult(_ data: Data, status: Int32) throws -> UpdateInventory {
        struct Failure: Decodable {
            let schema_version: Int
            let error: String
        }
        if status != 0, let failure = try? JSONDecoder().decode(Failure.self, from: data),
           failure.schema_version == 1, !failure.error.isEmpty, failure.error.utf8.count <= 4096 {
            throw UpdateError.operation(failure.error)
        }
        guard let inventory = try? UpdateInventory.decode(data),
              status == 0 || inventory.items.contains(where: { $0.availability == "failed" }) else {
            throw UpdateError.invalidResponse
        }
        return inventory
    }

    public static func run(binary: String, home: String, command: UpdateCommand) async throws -> UpdateInventory {
        try await Task.detached {
            guard binary.hasPrefix("/"), home.hasPrefix("/") else { throw UpdateError.invalidResponse }
            let pipe = Pipe()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: binary)
            process.arguments = command.arguments
            var environment = ProcessInfo.processInfo.environment
            environment["AOS_HOME"] = home
            process.environment = environment
            process.standardInput = FileHandle.nullDevice
            process.standardOutput = pipe
            process.standardError = FileHandle.nullDevice
            try process.run()
            try pipe.fileHandleForWriting.close()
            // The product command bounds its own subprocess tree and commits
            // status even if this window is hidden. Do not kill an installer
            // halfway through replacing the application when a view closes.
            var data = Data()
            while let part = try pipe.fileHandleForReading.read(upToCount: 8192), !part.isEmpty {
                if data.count + part.count <= 65_536 { data.append(part) }
                else { process.terminate(); process.waitUntilExit(); throw UpdateError.invalidResponse }
            }
            process.waitUntilExit()
            try pipe.fileHandleForReading.close()
            // Failed operations return a structured failure inventory with a
            // nonzero exit status. Decode it rather than hiding the failure.
            return try decodeResult(data, status: process.terminationStatus)
        }.value
    }
}
