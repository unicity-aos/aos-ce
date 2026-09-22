import Foundation
import Testing
@testable import AOSTrayCore

struct UpdateInventoryTests {
    private func data(_ availability: String, action: String = "none") -> Data {
        Data("""
        {"schema_version":1,"channel":"stable","checked_at":123,"items":[{"id":"aos","name":"AOS","installed_version":"2026.9.3","candidate_version":"2026.10.0","availability":"\(availability)","verification":"metadata","action":"\(action)","message":"Test metadata"}]}
        """.utf8)
    }
    @Test func failureAndActivationNeverBecomeCurrent() throws {
        let failed = try UpdateInventory.decode(data("failed", action: "apply")).items[0]
        #expect(!failed.canApply)
        #expect(failed.label == "Check or update failed")
        let installed = try UpdateInventory.decode(data("activation_required")).items[0]
        #expect(!installed.canApply)
        #expect(installed.label != "Up to date")
        #expect(try UpdateInventory.decode(data("available", action: "apply")).items[0].canApply)
    }
    @Test func commandsAreExplicitAndNeverInvokeAShell() {
        #expect(UpdateCommand.list.arguments == ["updates", "list", "--json"])
        #expect(UpdateCommand.check(channel: "dev").arguments == ["updates", "check", "--channel=dev", "--json"])
        #expect(UpdateCommand.apply(selection: "oracle:codex").arguments == ["updates", "apply", "oracle:codex", "--yes", "--json"])
    }
    @Test func rejectsUnknownSchema() {
        #expect(throws: (any Error).self) {
            try UpdateInventory.decode(Data("{\"schema_version\":99,\"channel\":\"stable\",\"items\":[]}".utf8))
        }
    }
}
