import Foundation
import Testing
@testable import AOSTrayCore

@Suite
struct InstalledRuntimeLaunchTests {
    @Test func parseDoesNotBindAHome() throws {
        let parsed = try LaunchArguments.parse(["aos-tray"]).get()
        #expect(parsed.aosBinary == nil)
        #expect(parsed.aosHome == nil)
        #expect(parsed.mode == .disconnected)
    }

    @Test func noArgsBindsCurrentUserHomeBinAos() throws {
        try withUserHome { userHome, _ in
            let expectedHome = NativeRuntimeSetup.realHome(userHome.path + "/.aos")
            let expectedBinary = (expectedHome ?? "") + "/bin/aos"
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse(["aos-tray"]).get(),
                environment: ["PATH": "/evil/bin"],
                userHome: userHome.path,
                binaryExists: { $0 == expectedBinary }
            )
            #expect(launch.aosHome == expectedHome)
            #expect(launch.aosBinary == expectedBinary)
            #expect(launch.expectedBinary == expectedBinary)
            #expect(launch.launchError == nil)
        }
    }

    @Test func missingDefaultBinaryIsReportedWithoutPathSearch() throws {
        try withUserHome { userHome, root in
            let evil = root.appendingPathComponent("evil/aos")
            try FileManager.default.createDirectory(at: evil.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("#!/bin/sh\n".utf8).write(to: evil)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: evil.path)
            let expectedHome = NativeRuntimeSetup.realHome(userHome.path + "/.aos")
            let expectedBinary = (expectedHome ?? "") + "/bin/aos"
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse(["aos-tray"]).get(),
                environment: ["PATH": evil.deletingLastPathComponent().path],
                userHome: userHome.path,
                binaryExists: { $0 == evil.path }
            )
            #expect(launch.aosHome == expectedHome)
            #expect(launch.aosBinary == nil)
            #expect(launch.expectedBinary == expectedBinary)
            #expect(launch.launchError == NativeRuntimeSetupCopy.missingBinary(expectedBinary))
            #expect(launch.expectedBinary != evil.path)
        }
    }

    @Test func validAbsoluteAosHomeOverridesDefaultHome() throws {
        try withUserHome { userHome, root in
            let custom = root.appendingPathComponent("custom-aos")
            try FileManager.default.createDirectory(at: custom, withIntermediateDirectories: true)
            let expectedHome = NativeRuntimeSetup.realHome(custom.path)
            let expectedBinary = (expectedHome ?? "") + "/bin/aos"
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse(["aos-tray"]).get(),
                environment: ["AOS_HOME": custom.path, "PATH": "/usr/bin"],
                userHome: userHome.path,
                binaryExists: { $0 == expectedBinary }
            )
            #expect(launch.aosHome == expectedHome)
            #expect(launch.aosBinary == expectedBinary)
            #expect(launch.aosHome != NativeRuntimeSetup.realHome(userHome.path + "/.aos"))
        }
    }

    @Test func invalidAosHomeDoesNotFallBackToDefault() throws {
        try withUserHome { userHome, _ in
            for value in ["relative", "/tmp/../aos", "", "  ", "/tmp/./aos"] {
                let launch = InstalledRuntimeLaunch.apply(
                    arguments: try LaunchArguments.parse(["aos-tray"]).get(),
                    environment: ["AOS_HOME": value],
                    userHome: userHome.path,
                    binaryExists: { _ in true }
                )
                #expect(launch.aosBinary == nil)
                #expect(launch.aosHome == nil)
                #expect(launch.expectedBinary == nil)
                #expect(launch.launchError == NativeRuntimeSetupCopy.invalidHome)
            }
        }
    }

    @Test func explicitPairIgnoresAosHomeAndPath() throws {
        try withUserHome { userHome, _ in
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse([
                    "aos-tray", "--aos-binary", "/opt/aos", "--aos-home", "/opt/home",
                ]).get(),
                environment: ["AOS_HOME": userHome.path + "/.aos", "PATH": "/evil/bin"],
                userHome: userHome.path,
                binaryExists: { _ in false }
            )
            #expect(launch.aosBinary == "/opt/aos")
            #expect(launch.aosHome == "/opt/home")
            #expect(launch.expectedBinary == "/opt/aos")
            #expect(launch.launchError == nil)
        }
    }

    @Test func demoSnapshotAndSocketStayUnbound() throws {
        try withUserHome { userHome, _ in
            let cases = [
                ["aos-tray", "--demo"],
                ["aos-tray", "--snapshot"],
                ["aos-tray", "--demo", "--snapshot"],
                ["aos-tray", "--socket", "/private/tmp/aos.sock"],
            ]
            for args in cases {
                let launch = InstalledRuntimeLaunch.apply(
                    arguments: try LaunchArguments.parse(args).get(),
                    environment: [:],
                    userHome: userHome.path,
                    binaryExists: { _ in true }
                )
                #expect(launch.aosBinary == nil)
                #expect(launch.aosHome == nil)
                #expect(launch.launchError == nil)
            }
        }
    }

    @Test func overviewWithoutPairStillBindsDefaultInstall() throws {
        try withUserHome { userHome, _ in
            let expectedHome = NativeRuntimeSetup.realHome(userHome.path + "/.aos")
            let expectedBinary = (expectedHome ?? "") + "/bin/aos"
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse(["aos-tray", "--overview"]).get(),
                environment: [:],
                userHome: userHome.path,
                binaryExists: { $0 == expectedBinary }
            )
            #expect(launch.aosHome == expectedHome)
            #expect(launch.aosBinary == expectedBinary)
        }
    }

    @Test func nativeInputConfigDoesNotDisableDefaultHome() throws {
        try withUserHome { userHome, _ in
            let expectedHome = NativeRuntimeSetup.realHome(userHome.path + "/.aos")
            let launch = InstalledRuntimeLaunch.apply(
                arguments: try LaunchArguments.parse([
                    "aos-tray", "--native-input-config", "/tmp/connection.json",
                ]).get(),
                environment: [:],
                userHome: userHome.path,
                binaryExists: { _ in true }
            )
            #expect(launch.aosHome == expectedHome)
            #expect(launch.aosBinary == (expectedHome ?? "") + "/bin/aos")
        }
    }

    @Test func usableBinaryRejectsDirectoriesAndRequiresExecute() throws {
        try withUserHome { _, root in
            let dir = root.appendingPathComponent("not-a-binary")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            #expect(!InstalledRuntimeLaunch.isUsableBinary(dir.path))
            let file = root.appendingPathComponent("aos")
            try Data("#!/bin/sh\n".utf8).write(to: file)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
            #expect(!InstalledRuntimeLaunch.isUsableBinary(file.path))
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: file.path)
            #expect(InstalledRuntimeLaunch.isUsableBinary(file.path))
            #expect(!InstalledRuntimeLaunch.isUsableBinary("aos"))
        }
    }

    private func withUserHome(_ body: (URL, URL) throws -> Void) throws {
        let root = URL(fileURLWithPath: "/private/tmp/aos-tray-launch-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let userHome = root.appendingPathComponent("user")
        try FileManager.default.createDirectory(at: userHome, withIntermediateDirectories: true)
        try body(userHome, root)
    }
}
