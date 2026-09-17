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
        guard let canonical = NativeRuntimeSetup.realExistingDirectory(mountpoint),
              Self.isExactAstridFS(canonical) else { throw OverviewError.invalidStatus }
        return URL(fileURLWithPath: canonical, isDirectory: true)
    }

    /// True only when `path` is the live astridfs mount root, not a directory on another filesystem.
    /// Identity is libc `realpath` versus Darwin `statfs` `f_mntonname`, not Foundation
    /// `resolvingSymlinksInPath` (which presents `/private/tmp` as `/tmp`).
    static func isExactAstridFS(_ path: String) -> Bool {
        guard let canonical = NativeRuntimeSetup.realExistingDirectory(path) else { return false }
        var info = statfs()
        guard canonical.withCString({ statfs($0, &info) }) == 0 else { return false }
        let root = withUnsafeBytes(of: info.f_mntonname) {
            String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self)
        }
        let type = withUnsafeBytes(of: info.f_fstypename) {
            String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self)
        }
        return matchesReportedMountRoot(path, reportedRoot: root) && type == "astridfs"
    }

    /// Kernel mount-root identity. Foundation `/tmp` presentation is not a match
    /// for a `/private/tmp` `f_mntonname`.
    static func matchesReportedMountRoot(_ path: String, reportedRoot: String) -> Bool {
        guard let canonical = NativeRuntimeSetup.realExistingDirectory(path) else { return false }
        return canonical == reportedRoot
    }
}
