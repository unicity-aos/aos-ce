import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct MountedVolumeTests {
    private let base = #"{"state":"running","pid":42,"uptime_secs":1,"runtime_version":"test","ephemeral":false,"connected_clients":1,"loaded_capsules":[]}"#

    @Test func requiresRunningRuntimeAndKnownProvider() throws {
        let mount = #"{"mount_id":"80000000-0000-0000-0000-000000000001","mountpoint":"/Volumes/AOS","provider":"astrid-storage-provider-fskit","access":"read-write"}"#
        let json = String(base.dropLast()) + ",\"mounted_volume\":" + mount + "}"
        let status = try RuntimeOverview.decode(Data(json.utf8))
        #expect(status.mountedVolume?.mountpoint == "/Volumes/AOS")
        for invalid in [json.replacingOccurrences(of: "astrid-storage-provider-fskit", with: "other"),
                        json.replacingOccurrences(of: "/Volumes/AOS", with: "/Volumes/../AOS"),
                        json.replacingOccurrences(of: "read-write", with: "unknown")] {
            #expect(throws: (any Error).self) { try RuntimeOverview.decode(Data(invalid.utf8)) }
        }
        let stopped = json.replacingOccurrences(of: "\"running\"", with: "\"stopped\"")
            .replacingOccurrences(of: "\"pid\":42", with: "\"pid\":0")
            .replacingOccurrences(of: "\"uptime_secs\":1", with: "\"uptime_secs\":0")
            .replacingOccurrences(of: "\"connected_clients\":1", with: "\"connected_clients\":0")
        #expect(throws: (any Error).self) { try RuntimeOverview.decode(Data(stopped.utf8)) }
    }

    @Test func ordinaryFolderIsNotAMountedAstridVolume() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let bytes = try JSONSerialization.data(withJSONObject: [
            "mount_id": UUID().uuidString, "mountpoint": root.resolvingSymlinksInPath().path,
            "provider": "astrid-storage-provider-fskit", "access": "read-write"
        ])
        let mount = try JSONDecoder().decode(MountedVolume.self, from: bytes)
        #expect(throws: (any Error).self) { try mount.verifiedNativeRoot() }
    }

    @Test func privateTmpAliasIsNotFoundationIdentityAndOrdinaryFolderStillFails() throws {
        let name = "aos-vol-alias-\(UUID().uuidString)"
        let canonical = "/private/tmp/\(name)/mnt/files"
        let alias = "/tmp/\(name)/mnt/files"
        try FileManager.default.createDirectory(atPath: canonical, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: "/private/tmp/\(name)") }

        #expect(URL(fileURLWithPath: canonical).resolvingSymlinksInPath().path == alias)
        #expect(NativeRuntimeSetup.realExistingDirectory(canonical) == canonical)
        #expect(NativeRuntimeSetup.realExistingDirectory(alias) == canonical)
        #expect(MountedVolume.matchesReportedMountRoot(canonical, reportedRoot: canonical))
        #expect(MountedVolume.matchesReportedMountRoot(alias, reportedRoot: canonical))
        #expect(!MountedVolume.matchesReportedMountRoot(canonical, reportedRoot: alias))

        let bytes = try JSONSerialization.data(withJSONObject: [
            "mount_id": UUID().uuidString, "mountpoint": canonical,
            "provider": "astrid-storage-provider-fskit", "access": "read-write"
        ])
        let mount = try JSONDecoder().decode(MountedVolume.self, from: bytes)
        #expect(throws: (any Error).self) { try mount.verifiedNativeRoot() }
        #expect(!MountedVolume.isExactAstridFS(canonical))
        #expect(!MountedVolume.isExactAstridFS(alias))
    }
}
