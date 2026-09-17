import Foundation
import Darwin

/// Navigation metadata validated against the selected runtime, not a volume label.
public struct MountedVolume: Decodable, Equatable, Sendable {
    public let mountID: UUID
    public let mountpoint: String
    public let provider: String
    public let access: String

    private enum CodingKeys: String, CodingKey {
        case mountID = "mount_id", mountpoint, provider, access
    }

    func validate() throws {
        guard mountpoint.hasPrefix("/"),
              !mountpoint.split(separator: "/").contains(where: { $0 == "." || $0 == ".." }),
              provider == "astrid-storage-provider-fskit",
              ["read-only", "read-write"].contains(access) else { throw OverviewError.invalidStatus }
    }

    /// Recheck immediately before Finder dispatch. A stale lease or an ordinary
    /// directory must not be presented as the mounted filesystem.
    public func verifiedNativeRoot() throws -> URL {
        try validate()
        let url = URL(fileURLWithPath: mountpoint).resolvingSymlinksInPath()
        guard url.path == mountpoint else { throw OverviewError.invalidStatus }
        var info = statfs()
        guard statfs(mountpoint, &info) == 0 else { throw OverviewError.invalidStatus }
        let root = withUnsafeBytes(of: info.f_mntonname) {
            String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self)
        }
        let type = withUnsafeBytes(of: info.f_fstypename) {
            String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self)
        }
        guard root == mountpoint, type == "astridfs" else { throw OverviewError.invalidStatus }
        return url
    }
}
