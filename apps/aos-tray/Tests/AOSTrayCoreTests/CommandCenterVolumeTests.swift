import Darwin
import Foundation
import Testing
@testable import AOSTrayCore

@Suite(.serialized) struct CommandCenterVolumeTests {
    private var alice: OwnedPrincipal { OwnedPrincipal(id: "alice", enabled: true) }
    private var bob: OwnedPrincipal { OwnedPrincipal(id: "bob", enabled: false) }

    private func withHome(_ body: (URL) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aos-vol-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(root)
    }

    private func fixture(_ body: String, home: URL, check: (String) throws -> Void) throws {
        let command = home.appendingPathComponent("aos-fixture")
        try Data(("#!/bin/sh\n" + body).utf8).write(to: command)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: command.path)
        try check(command.path)
    }

    @Test func mountArgumentsArePrincipalViewOnly() {
        let args = CommandCenterVolume.mountArguments(
            principal: "alice", mountpoint: "/tmp/aos-home/mnt/files")
        #expect(args == [
            "--principal", "alice", "storage", "mount", "--as", "alice", "/tmp/aos-home/mnt/files",
        ])
        #expect(!args.contains("--admin"))
        #expect(!args.contains("--fleet"))
        #expect(!args.contains("--read-write"))
        #expect(!args.contains { $0.contains(";") || $0.contains("&&") })
    }

    @Test func unmountArgumentsTargetCapturedPathOnly() {
        let args = CommandCenterVolume.unmountArguments(
            principal: "alice", mountpoint: "/tmp/aos-home/mnt/files")
        #expect(args == [
            "--principal", "alice", "storage", "unmount", "/tmp/aos-home/mnt/files",
        ])
        #expect(!args.contains("--admin"))
        #expect(!args.contains("--as"))
    }

    @Test func principalMustBeOwnedAndEnabled() throws {
        let owned = PrincipalDiscovery.owned([alice, bob])
        #expect(try CommandCenterVolume.resolvePrincipal(selected: "alice", discovery: owned) == "alice")
        #expect(throws: CommandCenterVolumeError.invalidPrincipal) {
            try CommandCenterVolume.resolvePrincipal(selected: "bob", discovery: owned)
        }
        #expect(throws: CommandCenterVolumeError.invalidPrincipal) {
            try CommandCenterVolume.resolvePrincipal(selected: "default", discovery: owned)
        }
        #expect(throws: CommandCenterVolumeError.invalidPrincipal) {
            try CommandCenterVolume.resolvePrincipal(selected: "alice", discovery: .failed)
        }
    }

    @Test func planDoesNotMountWhenStoppedOrOccupied() {
        #expect(CommandCenterVolume.plan(runtimeState: .stopped, mountState: .readyEmpty) == .refuseStopped)
        #expect(CommandCenterVolume.plan(runtimeState: .running, mountState: .occupied) == .refuseOccupied)
        #expect(CommandCenterVolume.plan(runtimeState: .running, mountState: .alreadyAstridFS) == .openExisting)
        #expect(CommandCenterVolume.plan(runtimeState: .running, mountState: .readyEmpty) == .mountThenOpen)
    }

    @Test func mountpointIsUnderCanonicalHome() throws {
        try withHome { home in
            let path = try CommandCenterVolume.mountpoint(home: home.path)
            #expect(path == NativeRuntimeSetup.realHome(home.path)! + "/mnt/files")
            #expect(!path.contains("/../"))
        }
    }

    @Test func prepareCreatesPrivateEmptyDirectoryAndRefusesHijack() throws {
        try withHome { home in
            let prepared = try CommandCenterVolume.prepare(home: home.path)
            #expect(prepared.path == NativeRuntimeSetup.realHome(home.path)! + "/mnt/files")
            #expect(prepared.state == .readyEmpty)
            var info = stat()
            #expect(prepared.path.withCString { lstat($0, &info) } == 0)
            #expect((info.st_mode & S_IFMT) == S_IFDIR)
            #expect((info.st_mode & 0o777) == 0o700)

            try Data("nope".utf8).write(to: URL(fileURLWithPath: prepared.path).appendingPathComponent("x"))
            #expect(throws: CommandCenterVolumeError.occupied) {
                _ = try CommandCenterVolume.prepare(home: home.path)
            }
        }
    }

    @Test func symlinkMountpointIsOccupied() throws {
        try withHome { home in
            let mnt = home.appendingPathComponent("mnt")
            try FileManager.default.createDirectory(at: mnt, withIntermediateDirectories: false)
            let target = home.appendingPathComponent("other")
            try FileManager.default.createDirectory(at: target, withIntermediateDirectories: false)
            #expect(symlink(target.path, home.appendingPathComponent("mnt/files").path) == 0)
            #expect(CommandCenterVolume.inspect(path: home.path + "/mnt/files") == .occupied)
            #expect(throws: CommandCenterVolumeError.occupied) {
                _ = try CommandCenterVolume.prepare(home: home.path)
            }
        }
    }

    @Test func fileAtMountpointIsOccupied() throws {
        try withHome { home in
            let mnt = home.appendingPathComponent("mnt")
            try FileManager.default.createDirectory(at: mnt, withIntermediateDirectories: false)
            try Data("file".utf8).write(to: home.appendingPathComponent("mnt/files"))
            #expect(throws: CommandCenterVolumeError.occupied) {
                _ = try CommandCenterVolume.prepare(home: home.path)
            }
        }
    }

    @Test func mountSpawnUsesExactArgvAndSelectedHome() throws {
        try withHome { home in
            try fixture("""
            printf '%s\\n' "$AOS_HOME" > "$AOS_HOME/env-home"
            printf '%s\\n' "$ASTRID_PRINCIPAL" > "$AOS_HOME/env-principal"
            printf '%s\\n' "$ASTRID_HOME" > "$AOS_HOME/env-astrid"
            for arg in "$@"; do printf '<%s>\\n' "$arg"; done > "$AOS_HOME/args"
            """, home: home) { command in
                let path = try CommandCenterVolume.mountpoint(home: home.path)
                try CommandCenterVolume.run(
                    binary: command, home: home.path, principal: "alice",
                    arguments: CommandCenterVolume.mountArguments(principal: "alice", mountpoint: path),
                    timeout: 5
                )
                let args = try String(contentsOfFile: home.path + "/args", encoding: .utf8)
                #expect(args == "<--principal>\n<alice>\n<storage>\n<mount>\n<--as>\n<alice>\n<\(path)>\n")
                #expect(try String(contentsOfFile: home.path + "/env-home", encoding: .utf8)
                    .trimmingCharacters(in: .whitespacesAndNewlines) == NativeRuntimeSetup.realHome(home.path))
                #expect(try String(contentsOfFile: home.path + "/env-principal", encoding: .utf8)
                    .trimmingCharacters(in: .whitespacesAndNewlines) == "alice")
                let astrid = try String(contentsOfFile: home.path + "/env-astrid", encoding: .utf8)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                #expect(astrid != home.path)
            }
        }
    }

    @Test func nonzeroMountIsNotSuccess() throws {
        try withHome { home in
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 7\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.commandFailed) {
                    try CommandCenterVolume.run(
                        binary: command, home: home.path, principal: "alice",
                        arguments: CommandCenterVolume.mountArguments(
                            principal: "alice", mountpoint: home.path + "/mnt/files"),
                        timeout: 5
                    )
                }
                #expect(FileManager.default.fileExists(atPath: home.path + "/spawned"))
            }
        }
    }

    @Test func nonzeroUnmountIsNotSuccess() throws {
        try withHome { home in
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 3\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.commandFailed) {
                    try CommandCenterVolume.run(
                        binary: command, home: home.path, principal: "alice",
                        arguments: CommandCenterVolume.unmountArguments(
                            principal: "alice", mountpoint: home.path + "/mnt/files"),
                        timeout: 5
                    )
                }
            }
        }
    }

    @Test func ejectWithoutAstridfsDoesNotSpawn() throws {
        try withHome { home in
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 0\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.notMounted) {
                    try CommandCenterVolume.ejectBlocking(
                        binary: command, home: home.path, selectedPrincipal: "alice",
                        discovery: .owned([alice])
                    )
                }
                #expect(!FileManager.default.fileExists(atPath: home.path + "/spawned"))
            }
        }
    }

    @Test func stoppedRuntimeDoesNotSpawnMount() throws {
        try withHome { home in
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 0\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.runtimeStopped) {
                    try CommandCenterVolume.openFilesBlocking(
                        binary: command, home: home.path, selectedPrincipal: "alice",
                        discovery: .owned([alice]), runtimeState: .stopped
                    )
                }
                #expect(!FileManager.default.fileExists(atPath: home.path + "/spawned"))
                #expect(!FileManager.default.fileExists(atPath: home.path + "/mnt/files"))
            }
        }
    }

    @Test func occupiedPathDoesNotSpawnMount() throws {
        try withHome { home in
            let mnt = home.appendingPathComponent("mnt")
            try FileManager.default.createDirectory(at: mnt, withIntermediateDirectories: false)
            try Data("hijack".utf8).write(to: home.appendingPathComponent("mnt/files"))
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 0\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.occupied) {
                    try CommandCenterVolume.openFilesBlocking(
                        binary: command, home: home.path, selectedPrincipal: "alice",
                        discovery: .owned([alice]), runtimeState: .running
                    )
                }
                #expect(!FileManager.default.fileExists(atPath: home.path + "/spawned"))
            }
        }
    }

    @Test func mountCliSuccessIsNotOpenSuccessWithoutNativeAstridfs() throws {
        try withHome { home in
            let path = try CommandCenterVolume.mountpoint(home: home.path)
            let status = #"{"state":"running","pid":42,"uptime_secs":1,"runtime_version":"test","ephemeral":false,"connected_clients":1,"loaded_capsules":[],"mounted_volume":{"mount_id":"80000000-0000-0000-0000-000000000001","mountpoint":"\#(path)","provider":"astrid-storage-provider-fskit","access":"read-write"}}"#
            try fixture("""
            echo invoked >> "$AOS_HOME/spawned"
            if [ "$1" = --principal ]; then
              [ "$2" = alice ] && [ "$3" = storage ] && [ "$4" = mount ] && [ "$5" = --as ] && [ "$6" = alice ] || exit 7
              exit 0
            fi
            [ "$1" = status ] && [ "$2" = --json ] || exit 7
            printf '%s' '\(status)'
            """, home: home) { command in
                #expect(throws: CommandCenterVolumeError.unverified) {
                    try CommandCenterVolume.openFilesBlocking(
                        binary: command, home: home.path, selectedPrincipal: "alice",
                        discovery: .owned([alice]), runtimeState: .running
                    )
                }
                let spawned = try String(contentsOfFile: home.path + "/spawned", encoding: .utf8)
                #expect(spawned.contains("invoked"))
            }
        }
    }

    @Test func ordinaryDirectoryIsNotAstridfs() throws {
        try withHome { home in
            #expect(!MountedVolume.isExactAstridFS(home.path))
            #expect(!MountedVolume.isExactAstridFS(home.path + "/../" + home.lastPathComponent))
        }
    }

    @Test func tmpAliasIsSameCanonicalPathAndExtraSymlinkStaysOccupied() throws {
        let name = "aos-vol-alias-\(UUID().uuidString)"
        let canonicalHome = "/private/tmp/\(name)"
        let aliasHome = "/tmp/\(name)"
        let canonicalLeaf = canonicalHome + "/mnt/files"
        let aliasLeaf = aliasHome + "/mnt/files"
        try FileManager.default.createDirectory(atPath: canonicalLeaf, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: canonicalHome) }

        let foundation = URL(fileURLWithPath: canonicalLeaf).resolvingSymlinksInPath().path
        #expect(foundation == aliasLeaf)
        #expect(foundation != canonicalLeaf)
        #expect(NativeRuntimeSetup.realExistingDirectory(aliasLeaf) == canonicalLeaf)
        #expect(NativeRuntimeSetup.realExistingDirectory(canonicalLeaf) == canonicalLeaf)
        #expect(try CommandCenterVolume.mountpoint(home: aliasHome) == canonicalLeaf)
        #expect(try CommandCenterVolume.mountpoint(home: canonicalHome) == canonicalLeaf)
        #expect(!MountedVolume.isExactAstridFS(canonicalLeaf))
        #expect(!MountedVolume.isExactAstridFS(aliasLeaf))
        #expect(CommandCenterVolume.inspect(path: canonicalLeaf) == .readyEmpty)
        #expect(CommandCenterVolume.inspect(path: aliasLeaf) == .readyEmpty)

        try FileManager.default.removeItem(atPath: canonicalLeaf)
        let other = canonicalHome + "/other"
        try FileManager.default.createDirectory(atPath: other, withIntermediateDirectories: false)
        #expect(symlink(other, canonicalLeaf) == 0)
        #expect(NativeRuntimeSetup.realExistingDirectory(canonicalLeaf) == nil)
        #expect(NativeRuntimeSetup.realExistingDirectory(aliasLeaf) == nil)
        #expect(!MountedVolume.isExactAstridFS(canonicalLeaf))
        #expect(CommandCenterVolume.inspect(path: canonicalLeaf) == .occupied)
        #expect(throws: CommandCenterVolumeError.occupied) {
            _ = try CommandCenterVolume.prepare(home: canonicalHome)
        }
    }

    @Test func finderVolumeMountRequiresMacOS26() {
        #expect(DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0)))
        #expect(DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 26, minorVersion: 6, patchVersion: 2)))
        #expect(!DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 15, minorVersion: 4, patchVersion: 0)))
        #expect(!DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 13, minorVersion: 6, patchVersion: 1)))
        #expect(!DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 11, minorVersion: 0, patchVersion: 0)))
        #expect(!DarwinPlatform.finderVolumeMountAvailable(
            version: OperatingSystemVersion(majorVersion: 25, minorVersion: 0, patchVersion: 0)))
    }

    @Test func unavailableFinderMountDoesNotSpawn() throws {
        try withHome { home in
            try fixture("echo spawned > \"$AOS_HOME/spawned\"; exit 0\n", home: home) { command in
                #expect(throws: CommandCenterVolumeError.requiresMacOS26) {
                    try CommandCenterVolume.openFilesBlocking(
                        binary: command, home: home.path, selectedPrincipal: "alice",
                        discovery: .owned([alice]), runtimeState: .running,
                        finderMountAvailable: false
                    )
                }
                #expect(!FileManager.default.fileExists(atPath: home.path + "/spawned"))
            }
        }
    }
}
