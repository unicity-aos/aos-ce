import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct CapsuleLibraryTests {
    private func decode(_ json: String) throws -> CapsuleLibrary {
        let library = try JSONDecoder().decode(CapsuleLibrary.self, from: Data(json.utf8))
        try library.validate()
        return library
    }

    @Test func unavailableIsNotEmptyInventory() throws {
        for state in ["stopped", "unavailable"] {
            let library = try decode("{\"principal\":\"alice\",\"state\":\"\(state)\"}")
            #expect(library.capsules == nil)
        }
        for invalid in [
            #"{"principal":"alice","state":"available"}"#,
            #"{"principal":"alice","state":"stopped","capsules":[]}"#,
            #"{"principal":"","state":"available","capsules":[]}"#,
            #"{"principal":"alice","state":"available","capsules":[{"name":"x","version":"1"},{"name":"x","version":"2"}]}"#
        ] {
            #expect(throws: (any Error).self) { try decode(invalid) }
        }
    }

    @Test func searchesNamesAndDescriptionsWithoutInventingGrants() throws {
        let library = try decode(#"{"principal":"alice","state":"available","capsules":[{"name":"forge","version":"1","description":"Build software"},{"name":"notes","version":"2","description":null}]}"#)
        #expect(library.matching(" SOFTWARE ").map(\.name) == ["forge"])
        #expect(library.matching("NOTES").map(\.name) == ["notes"])
        #expect(library.matching("unknown").isEmpty)
        #expect(library.matching("").count == 2)
    }

    @Test func hundredCapsulesRemainSearchable() throws {
        let entries = (0..<100).map { ["name": "capsule-\($0)", "version": "1.0", "description": "Example package"] }
        let data = try JSONSerialization.data(withJSONObject: ["principal": "alice", "state": "available", "capsules": entries])
        let library = try JSONDecoder().decode(CapsuleLibrary.self, from: data)
        try library.validate()
        #expect(library.matching("").count == 100)
        #expect(library.matching("capsule-99").map(\.name) == ["capsule-99"])
    }
}
