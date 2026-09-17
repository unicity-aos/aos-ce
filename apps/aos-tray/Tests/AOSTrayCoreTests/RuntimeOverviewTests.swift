import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct RuntimeOverviewTests {
    @Test func inspectionRequiresAnExplicitPairAndCannotMixDemo() throws {
        let args = try LaunchArguments.parse(["aos-tray", "--aos-binary", "/opt/aos", "--aos-home", "/test/aos"]).get()
        #expect(args.aosBinary == "/opt/aos")
        #expect(args.aosHome == "/test/aos")
        for invalid in [
            ["aos-tray", "--aos-binary", "/opt/aos"],
            ["aos-tray", "--aos-home", "/test/aos"],
            ["aos-tray", "--aos-binary", "aos", "--aos-home", "/test/aos"],
            ["aos-tray", "--demo", "--aos-binary", "/opt/aos", "--aos-home", "/test/aos"]
        ] {
            #expect(throws: (any Error).self) { try LaunchArguments.parse(invalid).get() }
        }
    }

    @Test func typedRunningStatusPreservesLoadedNotInstalledInventory() throws {
        let data = Data(#"{"state":"running","pid":42,"uptime_secs":8,"runtime_version":"2026.9.2","ephemeral":true,"connected_clients":2,"loaded_capsules":["aos-mcp"]}"#.utf8)
        let status = try RuntimeOverview.decode(data)
        #expect(status.state == .running)
        #expect(status.connectedClients == 2)
        #expect(status.loadedCapsules == ["aos-mcp"])
    }

    @Test func invalidOrPartialStatusDoesNotBecomeStopped() {
        for value in ["{}", #"{"state":"unreachable"}"#,
            #"{"state":"stopped","pid":42,"uptime_secs":0,"runtime_version":"x","ephemeral":false,"connected_clients":0,"loaded_capsules":[]}"#] {
            #expect(throws: (any Error).self) { try RuntimeOverview.decode(Data(value.utf8)) }
        }
    }

    @Test func inventoryStateMustMatchRuntimeAndOlderResponsesStillWork() throws {
        let old = #"{"state":"running","pid":42,"uptime_secs":8,"runtime_version":"2026.9.2","ephemeral":true,"connected_clients":2,"loaded_capsules":[]}"#
        let decoded = try RuntimeOverview.decode(Data(old.utf8))
        #expect(decoded.capsuleInventory == nil)
        let mismatch = String(old.dropLast()) + #", "capsule_inventory":{"principal":"default","state":"stopped"}}"#
        #expect(throws: (any Error).self) { try RuntimeOverview.decode(Data(mismatch.utf8)) }
        let stopped = #"{"state":"stopped","pid":0,"uptime_secs":0,"runtime_version":"2026.9.2","ephemeral":false,"connected_clients":0,"loaded_capsules":[],"capsule_inventory":{"principal":"default","state":"available","capsules":[]}}"#
        #expect(throws: (any Error).self) { try RuntimeOverview.decode(Data(stopped.utf8)) }
    }

    @Test func volumeUsesActualFileSizeAndRejectsMissingOrSymlink() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("runtime"), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        #expect(throws: (any Error).self) { try VolumeFileInfo.read(aosHome: root) }
        let volume = root.appendingPathComponent("runtime/astrid.volume")
        try Data(repeating: 1, count: 37).write(to: volume)
        #expect(try VolumeFileInfo.read(aosHome: root).fileBytes == 37)
        try FileManager.default.removeItem(at: volume)
        try FileManager.default.createSymbolicLink(at: volume, withDestinationURL: root)
        #expect(throws: (any Error).self) { try VolumeFileInfo.read(aosHome: root) }
    }
}
