import Foundation
import Darwin
import Testing
@testable import AOSTrayCore

@Suite struct PrincipalDiscoveryTests {
    private func fixture(_ body: String, check: (String, String) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        let command = root.appendingPathComponent("aos-fixture")
        try Data(("#!/bin/sh\n" + body).utf8).write(to: command)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: command.path)
        try check(command.path, root.path)
    }

    @Test func pickerSelectsEnabledOwnedPrincipalAndRejectsEmptyStaleAndInvalid() {
        let alice = OwnedPrincipal(id: "alice", enabled: true)
        let bob = OwnedPrincipal(id: "bob", enabled: false)
        let owned = PrincipalDiscovery.owned([alice, bob])

        #expect(PrincipalPicker.choose(selected: "alice", from: owned) == .selected(alice))
        #expect(PrincipalPicker.choose(selected: "  alice  ", from: owned) == .selected(alice))
        #expect(PrincipalPicker.choose(selected: nil, from: .owned([])) == .emptyDirectory)
        #expect(PrincipalPicker.choose(selected: "alice", from: .owned([])) == .emptyDirectory)
        #expect(PrincipalPicker.choose(selected: "carol", from: owned) == .staleIdentifier("carol"))
        #expect(PrincipalPicker.choose(selected: "bob", from: owned) == .staleIdentifier("bob"))
        #expect(PrincipalPicker.choose(selected: "", from: owned) == .invalidIdentifier)
        #expect(PrincipalPicker.choose(selected: "not/a/principal", from: owned) == .invalidIdentifier)
        #expect(PrincipalPicker.choose(selected: "anonymous", from: owned) == .invalidIdentifier)
        #expect(PrincipalPicker.choose(selected: "alice", from: .unsupported) == .discoveryUnavailable)
        #expect(PrincipalPicker.choose(selected: "alice", from: .failed) == .discoveryUnavailable)
        #expect(PrincipalDiscoveryCopy.message(for: .emptyDirectory, discovery: .owned([]))
                == PrincipalDiscoveryCopy.emptyDirectory)
        #expect(PrincipalDiscoveryCopy.message(for: .discoveryUnavailable, discovery: .unsupported)
                == PrincipalDiscoveryCopy.unsupported)
        #expect(PrincipalDiscoveryCopy.message(for: .discoveryUnavailable, discovery: .failed)
                == PrincipalDiscoveryCopy.failed)
    }

    @Test func readerUsesOnlyOwnedDiscoveryCommand() throws {
        let envelope = #"{"scope":"owned","authority":"discovery","principals":[{"id":"alice","enabled":true},{"id":"default","enabled":false}]}"#
        try fixture("[ \"$1\" = principals ] && [ \"$2\" = --json ] && [ $# = 2 ] && [ -d \"$AOS_HOME\" ] || exit 7\nprintf '%s' '\(envelope)'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .owned([
                OwnedPrincipal(id: "alice", enabled: true),
                OwnedPrincipal(id: "default", enabled: false),
            ]))
        }
    }

    @Test func emptyOwnedDirectoryIsOwnedNotAnError() throws {
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .owned([]))
        }
    }

    @Test func unsupportedExitIsNotAGlobalFallback() throws {
        try fixture("printf '%s' 'error: unexpected argument --mine' >&2\nexit 2\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .unsupported)
        }
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[]}'\nexit 2\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .unsupported)
        }
    }

    @Test func authAndMalformedEnvelopesFailClosed() throws {
        try fixture("printf '%s' 'principal discovery requires a user-delegated device' >&2\nexit 1\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '[{\"principal\":\"alice\",\"enabled\":true}]'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '{\"scope\":\"global\",\"authority\":\"discovery\",\"principals\":[]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"acting\",\"principals\":[]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[{\"id\":\"alice\",\"enabled\":true},{\"id\":\"alice\",\"enabled\":false}]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[{\"id\":\"anonymous\",\"enabled\":true}]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
        try fixture("printf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[{\"id\":\"not/a/principal\",\"enabled\":true}]}'\n") { command, home in
            let discovery = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            #expect(discovery == .failed)
        }
    }

    @Test func readerDoesNotPassALibraryPrincipal() throws {
        try fixture("for arg in \"$@\"; do printf '<%s>\\n' \"$arg\"; done > \"$AOS_HOME/args\"\nprintf '%s' '{\"scope\":\"owned\",\"authority\":\"discovery\",\"principals\":[]}'\n") { command, home in
            _ = try PrincipalDiscoveryReader.readBlocking(binary: command, home: home)
            let args = try String(contentsOfFile: home + "/args", encoding: .utf8)
            #expect(args == "<principals>\n<--json>\n")
            #expect(!args.contains("--principal"))
        }
    }
}
