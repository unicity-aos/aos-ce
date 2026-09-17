import Foundation
import Darwin
import Testing
@testable import AOSTrayCore

@Suite struct NativeRuntimeSetupTests {
    private func fixture(_ body: String, check: (String, String) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let command = root.appendingPathComponent("aos-fixture")
        try Data(("#!/bin/sh\n" + body).utf8).write(to: command)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: command.path)
        try check(command.path, root.path)
    }

    private var alice: OwnedPrincipal { OwnedPrincipal(id: "alice", enabled: true) }
    private var bob: OwnedPrincipal { OwnedPrincipal(id: "bob", enabled: false) }

    private var successPrinter: String {
        #"printf '{"scope":"local-personal","authority":"setup","principal":"alice","connectionPath":"%s/native-input/connection.json","restartRequired":true,"connected":false}' "$AOS_HOME""#
    }

    @Test func commandUsesOnlyNativeSetupContractAndNeverAToken() {
        let args = NativeRuntimeSetup.commandArguments(principal: "alice")
        #expect(args == ["native-setup", "--principal", "alice", "--confirm-enroll", "--confirm-route", "--json"])
        #expect(!args.contains { $0.contains("token") || $0.contains("astrid_pair") || $0 == "--force" })
    }

    @Test func gateRequiresOwnedChoiceAndBothConfirms() {
        let owned = PrincipalDiscovery.owned([alice, bob])
        #expect(NativeRuntimeSetup.evaluate(selected: "alice", discovery: owned, confirmEnroll: true, confirmRoute: true) == .run("alice"))
        #expect(NativeRuntimeSetup.evaluate(selected: "alice", discovery: owned, confirmEnroll: false, confirmRoute: true) == .unconfirmed)
        #expect(NativeRuntimeSetup.evaluate(selected: "alice", discovery: owned, confirmEnroll: true, confirmRoute: false) == .unconfirmed)
        #expect(NativeRuntimeSetup.evaluate(selected: nil, discovery: .owned([]), confirmEnroll: true, confirmRoute: true) == .emptyDirectory)
        #expect(NativeRuntimeSetup.evaluate(selected: "carol", discovery: owned, confirmEnroll: true, confirmRoute: true) == .staleIdentifier("carol"))
        #expect(NativeRuntimeSetup.evaluate(selected: "bob", discovery: owned, confirmEnroll: true, confirmRoute: true) == .staleIdentifier("bob"))
        #expect(NativeRuntimeSetup.evaluate(selected: "anonymous", discovery: owned, confirmEnroll: true, confirmRoute: true) == .invalidIdentifier)
        #expect(NativeRuntimeSetup.evaluate(selected: "alice", discovery: .unsupported, confirmEnroll: true, confirmRoute: true) == .unsupported)
        #expect(NativeRuntimeSetup.evaluate(selected: "alice", discovery: .failed, confirmEnroll: true, confirmRoute: true) == .failed)
        #expect(NativeRuntimeSetupCopy.message(for: .unconfirmed) == NativeRuntimeSetupCopy.unconfirmed)
        #expect(NativeRuntimeSetupCopy.message(for: .unsupported) == NativeRuntimeSetupCopy.unsupported)
    }

    @Test func defaultPathIsPrivateAndAdoptOnlyLoadsExistingEnrollment() throws {
        let root = URL(fileURLWithPath: "/private/tmp/ani-setup-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        #expect(NativeRuntimeSetup.defaultConnectionPath(home: root.path) == root.path + "/native-input/connection.json")
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: root.path) == nil)

        let directory = root.appendingPathComponent("native-input")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let file = directory.appendingPathComponent("connection.json")
        try Data("not-json".utf8).write(to: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: root.path) == nil)

        let object: [String: Any] = [
            "socketPath": root.path + "/run/system.sock",
            "principal": "alice",
            "privateKeyPath": root.path + "/runtime/keys/local/aos-tray.ed25519",
            "tokenPath": root.path + "/run/system.token",
            "capacity": 8,
            "inputTimeoutSeconds": 120,
            "ioTimeoutSeconds": 5,
            "readTimeoutSeconds": 3600,
        ]
        try JSONSerialization.data(withJSONObject: object).write(to: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: root.path) == file.path)

        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: root.path) == nil)
        #expect(!NativeRuntimeSetup.existingEnrollment(home: root.path, configPath: nil))
        #expect(NativeRuntimeSetup.existingEnrollment(home: root.path, configPath: file.path))
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        #expect(NativeRuntimeSetup.existingEnrollment(home: root.path, configPath: nil))
        #expect(!NativeRuntimeSetup.existingEnrollment(home: nil, configPath: nil))
        #expect(!NativeRuntimeSetup.existingEnrollment(home: "relative", configPath: "relative"))
    }

    @Test func tmpAliasHomeCanonicalizesAndAdoptsTheCanonicalConnection() throws {
        let name = "ani-setup-alias-\(UUID())"
        let alias = "/tmp/\(name)"
        let canonical = "/private/tmp/\(name)"
        try FileManager.default.createDirectory(atPath: canonical, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: canonical) }
        #expect(NativeRuntimeSetup.realHome(alias) == canonical)
        #expect(NativeRuntimeSetup.defaultConnectionPath(home: alias) == canonical + "/native-input/connection.json")
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: alias) == nil)

        let directory = URL(fileURLWithPath: canonical).appendingPathComponent("native-input")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let file = directory.appendingPathComponent("connection.json")
        let object: [String: Any] = [
            "socketPath": canonical + "/run/system.sock",
            "principal": "alice",
            "privateKeyPath": canonical + "/runtime/keys/local/aos-tray.ed25519",
            "tokenPath": canonical + "/run/system.token",
            "capacity": 8,
            "inputTimeoutSeconds": 120,
            "ioTimeoutSeconds": 5,
            "readTimeoutSeconds": 3600,
        ]
        try JSONSerialization.data(withJSONObject: object).write(to: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        #expect(NativeRuntimeSetup.adoptableConnectionPath(home: alias) == file.path)
        #expect(NativeRuntimeSetup.existingEnrollment(home: alias, configPath: nil))
    }

    @Test func danglingSymlinkHomeFailsClosedWithoutMutation() throws {
        let parent = URL(fileURLWithPath: "/private/tmp/ani-setup-dangle-\(UUID())")
        try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: parent) }
        let dangle = parent.appendingPathComponent("home")
        #expect(symlink("/does/not/exist-\(UUID())", dangle.path) == 0)
        #expect(NativeRuntimeSetup.realHome(dangle.path) == nil)
        #expect(NativeRuntimeSetup.defaultConnectionPath(home: dangle.path) == nil)
        try fixture("printf spawned > \"$AOS_HOME/spawned\"; \(successPrinter)\n") { command, _ in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: dangle.path, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        let names = try FileManager.default.contentsOfDirectory(atPath: parent.path)
        #expect(names == ["home"])
        #expect(!FileManager.default.fileExists(atPath: parent.appendingPathComponent("native-input").path))
    }

    @Test func successfulCommandContractNeverPutsATokenOnArgv() throws {
        try fixture("""
        for arg in "$@"; do printf '<%s>\\n' "$arg"; done > "$AOS_HOME/args"
        if [ -p /dev/stdin ]; then cat > "$AOS_HOME/stdin"; fi
        [ "$1" = native-setup ] && [ "$2" = --principal ] && [ "$3" = alice ] && [ "$4" = --confirm-enroll ] && [ "$5" = --confirm-route ] && [ "$6" = --json ] && [ $# = 6 ] || exit 7
        \(successPrinter)
        """) { command, home in
            let result = try NativeRuntimeSetup.runBlocking(
                binary: command, home: home, principal: "alice",
                confirmEnroll: true, confirmRoute: true
            )
            #expect(result.scope == "local-personal")
            #expect(result.authority == "setup")
            #expect(result.principal == "alice")
            #expect(result.restartRequired)
            #expect(!result.connected)
            #expect(result.connectionPath == NativeRuntimeSetup.defaultConnectionPath(home: home))
            let args = try String(contentsOfFile: home + "/args", encoding: .utf8)
            #expect(args == "<native-setup>\n<--principal>\n<alice>\n<--confirm-enroll>\n<--confirm-route>\n<--json>\n")
            #expect(!args.contains("token"))
            #expect(!args.contains("astrid_pair"))
            #expect(!FileManager.default.fileExists(atPath: home + "/stdin"))
        }
    }

    @Test func receiptMustMatchRequestedPrincipalAndExpectedConnection() throws {
        try fixture(#"""
        printf '{"scope":"local-personal","authority":"setup","principal":"bob","connectionPath":"%s/native-input/connection.json","restartRequired":true,"connected":false}' "$AOS_HOME"
        """#) { command, home in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        #expect(OwnedPrincipal.isValidID("token"))
        try fixture(#"""
        printf '{"scope":"local-personal","authority":"setup","principal":"token","connectionPath":"%s/native-input/connection.json","restartRequired":true,"connected":false}' "$AOS_HOME"
        """#) { command, home in
            let result = try NativeRuntimeSetup.runBlocking(
                binary: command, home: home, principal: "token",
                confirmEnroll: true, confirmRoute: true
            )
            #expect(result.principal == "token")
            #expect(result.connectionPath == NativeRuntimeSetup.defaultConnectionPath(home: home))
        }
        try fixture(#"""
        printf '{"scope":"local-personal","authority":"setup","principal":"alice","connectionPath":"%s/native-input/connection.json","restartRequired":true,"connected":false,"token":"x"}' "$AOS_HOME"
        """#) { command, home in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
    }

    @Test func unconfirmedAndInvalidNeverSpawn() throws {
        try fixture("printf spawned > \"$AOS_HOME/spawned\"; printf '%s' '{}'\n") { command, home in
            #expect(throws: NativeRuntimeSetupError.unconfirmed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: false, confirmRoute: true
                )
            }
            #expect(throws: NativeRuntimeSetupError.unconfirmed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: false
                )
            }
            #expect(throws: NativeRuntimeSetupError.invalidPrincipal) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "anonymous",
                    confirmEnroll: true, confirmRoute: true
                )
            }
            #expect(throws: NativeRuntimeSetupError.invalidPrincipal) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "not/a/principal",
                    confirmEnroll: true, confirmRoute: true
                )
            }
            #expect(!FileManager.default.fileExists(atPath: home + "/spawned"))
        }
    }

    @Test func unsupportedExistingAndFailedAreClassifiedWithoutFallback() throws {
        try fixture("printf '%s' 'aos: native-input setup is not supported by this runtime' >&2\nexit 2\n") { command, home in
            #expect(throws: NativeRuntimeSetupError.unsupported) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        try fixture("printf '%s' 'aos: a native-input connection already exists; not overwritten' >&2\nexit 1\n") { command, home in
            #expect(throws: NativeRuntimeSetupError.existing) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        try fixture("printf '%s' 'aos: native-input setup failed: principal requires a delegated device' >&2\nexit 1\n") { command, home in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        try fixture(#"""
        printf '{"scope":"local-personal","authority":"setup","principal":"alice","connectionPath":"%s/native-input/connection.json","restartRequired":true,"connected":false,"token":"astrid_pair_device-token"}' "$AOS_HOME"
        """#) { command, home in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
        try fixture(#"""
        printf '{"scope":"local-personal","authority":"setup","principal":"alice","connectionPath":"%s/native-input/connection.json","restartRequired":false,"connected":true}' "$AOS_HOME"
        """#) { command, home in
            #expect(throws: NativeRuntimeSetupError.failed) {
                try NativeRuntimeSetup.runBlocking(
                    binary: command, home: home, principal: "alice",
                    confirmEnroll: true, confirmRoute: true
                )
            }
        }
    }
}
