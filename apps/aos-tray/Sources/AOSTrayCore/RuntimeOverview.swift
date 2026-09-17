import Foundation

/// Mirrors the typed `aos status --json` response, not a process-list guess.
public struct RuntimeOverview: Decodable, Equatable, Sendable {
    public enum State: String, Decodable, Sendable { case running, stopped }
    public let state: State
    public let pid: UInt32
    public let uptimeSecs: UInt64
    public let runtimeVersion: String
    public let ephemeral: Bool
    public let connectedClients: UInt32
    public let loadedCapsules: [String]
    public let capsuleInventory: CapsuleLibrary?
    public let mountedVolume: MountedVolume?

    private enum CodingKeys: String, CodingKey {
        case state, pid, ephemeral
        case uptimeSecs = "uptime_secs", runtimeVersion = "runtime_version"
        case connectedClients = "connected_clients", loadedCapsules = "loaded_capsules"
        case capsuleInventory = "capsule_inventory"
        case mountedVolume = "mounted_volume"
    }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= 65_536 else { throw OverviewError.invalidStatus }
        let value = try JSONDecoder().decode(Self.self, from: data)
        guard !value.runtimeVersion.isEmpty,
              (value.state == .running && value.pid > 0) ||
                (value.state == .stopped && value.pid == 0 && value.connectedClients == 0 &&
                 value.uptimeSecs == 0 && value.loadedCapsules.isEmpty) else {
            throw OverviewError.invalidStatus
        }
        if let library = value.capsuleInventory {
            try library.validate()
            guard (value.state == .stopped) == (library.state == .stopped) else {
                throw OverviewError.invalidStatus
            }
        }
        if let mount = value.mountedVolume {
            guard value.state == .running else { throw OverviewError.invalidStatus }
            try mount.validate()
        }
        return value
    }
}

public enum OverviewError: Error { case invalidStatus }

/// Container metadata only. This is neither logical content size nor capacity.
public struct VolumeFileInfo: Equatable, Sendable {
    public let url: URL
    public let fileBytes: UInt64

    public static func read(aosHome: URL) throws -> Self {
        let url = aosHome.appendingPathComponent("runtime/astrid.volume")
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let size = attributes[.size] as? NSNumber else {
            throw OverviewError.invalidStatus
        }
        return Self(url: url, fileBytes: size.uint64Value)
    }
}
