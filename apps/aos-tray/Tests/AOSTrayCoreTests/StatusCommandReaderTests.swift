import Foundation
import Darwin
import Testing
@testable import AOSTrayCore

@Suite(.serialized) struct StatusCommandReaderTests {
    private let stopped = #"{"state":"stopped","pid":0,"uptime_secs":0,"runtime_version":"test","ephemeral":false,"connected_clients":0,"loaded_capsules":[]}"#

    private func fixture(_ body: String, check: (String, String) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let command = root.appendingPathComponent("aos-fixture")
        try Data(("#!/bin/sh\n" + body).utf8).write(to: command)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: command.path)
        try check(command.path, root.path)
    }

    @Test func passesOnlyStatusArgumentsAndExplicitHome() throws {
        try fixture("[ \"$1\" = status ] && [ \"$2\" = --json ] && [ $# = 2 ] && [ -d \"$AOS_HOME\" ] || exit 7\nprintf '%s' '\(stopped)'\n") { command, home in
            let status = try StatusCommandReader.readBlocking(binary: command, home: home)
            #expect(status.state == .stopped)
        }
    }

    @Test func rejectsNonzeroEvenWithValidJSON() throws {
        try fixture("printf '%s' '\(stopped)'\nexit 7\n") { command, home in
            #expect(throws: (any Error).self) { try StatusCommandReader.readBlocking(binary: command, home: home) }
        }
    }

    @Test func inventoryFlagIsExplicitAndResponseIsDecoded() throws {
        let response = String(stopped.dropLast()) + #", "capsule_inventory":{"principal":"default","state":"stopped"}}"#
        try fixture("[ $# = 3 ] && [ \"$3\" = --include-capsules ] || exit 7\nprintf '%s' '\(response)'\n") { command, home in
            let status = try StatusCommandReader.readBlocking(binary: command, home: home, includeCapsules: true)
            #expect(status.capsuleInventory?.state == .stopped)
        }
    }

    @Test func requestedPrincipalIsPassedAndResponseMustMatch() throws {
        let response = String(stopped.dropLast()) + #", "capsule_inventory":{"principal":"alice","state":"stopped"}}"#
        try fixture("[ $# = 4 ] && [ \"$4\" = --principal=alice ] || exit 7\nprintf '%s' '\(response)'\n") { command, home in
            let status = try StatusCommandReader.readBlocking(binary: command, home: home,
                includeCapsules: true, principal: "alice")
            #expect(status.capsuleInventory?.principal == "alice")
        }
        try fixture("printf '%s' '\(response)'\n") { command, home in
            #expect(throws: (any Error).self) {
                try StatusCommandReader.readBlocking(binary: command, home: home,
                    includeCapsules: true, principal: "bob")
            }
        }
    }

    @Test func oversizedStreamIsStoppedAndReaped() throws {
        try assertStopped("""
        echo $$ > "$AOS_HOME/pid"
        sleep 0.05
        while :; do printf '%01000d' 0; done
        """, timeout: 3)
    }

    @Test func mountSelectionStaysOneArgumentAndRequiresMatchingResponse() throws {
        let response = #"{"state":"running","pid":42,"uptime_secs":1,"runtime_version":"test","ephemeral":false,"connected_clients":1,"loaded_capsules":[],"mounted_volume":{"mount_id":"80000000-0000-0000-0000-000000000001","mountpoint":"/Volumes/AOS QA","provider":"astrid-storage-provider-fskit","access":"read-write"}}"#
        try fixture("[ $# = 4 ] && [ \"$3\" = '--mountpoint=/Volumes/AOS QA' ] && [ \"$4\" = '--principal=default' ] || exit 7\nprintf '%s' '\(response)'\n") { command, home in
            let status = try StatusCommandReader.readBlocking(binary: command, home: home,
                principal: "default", mountpoint: "/Volumes/AOS QA")
            #expect(status.mountedVolume != nil)
        }
        try fixture("printf '%s' '\(response)'\n") { command, home in
            #expect(throws: (any Error).self) {
                try StatusCommandReader.readBlocking(binary: command, home: home,
                    principal: "default", mountpoint: "/Volumes/Other")
            }
        }
        try fixture("printf '%s' '\(stopped)'\n") { command, home in
            #expect(throws: (any Error).self) {
                try StatusCommandReader.readBlocking(binary: command, home: home, mountpoint: "/Volumes/AOS QA")
            }
        }
    }

    @Test func timeoutReapsChildEvenWhenItIgnoresTERMAndClosesOutput() throws {
        try assertStopped("""
        echo $$ > "$AOS_HOME/pid"
        sleep 0.05
        trap '' TERM
        exec 1>&-
        while :; do :; done
        """, timeout: 3)
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["AOS_TRAY_STATUS_TEST_BINARY"] != nil))
    func explicitDisposableRuntimeStatus() async throws {
        let environment = ProcessInfo.processInfo.environment
        let binary = try #require(environment["AOS_TRAY_STATUS_TEST_BINARY"])
        let home = try #require(environment["AOS_TRAY_STATUS_TEST_HOME"])
        let status = try await StatusCommandReader.read(binary: binary, home: home)
        #expect(status.state == .stopped)
        #expect(status.pid == 0)
        #expect(status.loadedCapsules.isEmpty)
    }

    private func assertStopped(_ script: String, timeout: TimeInterval) throws {
        try fixture(script) { command, home in
            #expect(throws: (any Error).self) {
                try StatusCommandReader.readBlocking(binary: command, home: home, timeout: timeout)
            }
            let pidPath = home + "/pid"
            var pidText: String?
            let deadline = ProcessInfo.processInfo.systemUptime + 1
            while ProcessInfo.processInfo.systemUptime < deadline {
                if let text = try? String(contentsOfFile: pidPath, encoding: .utf8) {
                    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !trimmed.isEmpty {
                        pidText = trimmed
                        break
                    }
                }
                Thread.sleep(forTimeInterval: 0.01)
            }
            let pid = try #require(Int32(pidText ?? ""))
            #expect(Darwin.kill(pid, 0) == -1)
            #expect(errno == ESRCH)
        }
    }
}
