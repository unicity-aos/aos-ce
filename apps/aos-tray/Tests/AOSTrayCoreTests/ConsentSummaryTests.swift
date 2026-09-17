import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct ConsentSummaryTests {
    private let original = "A capsule tool is requesting capability approval.\nAction: fetch\nResource: github.com\nApprove this request?"

    @Test func structuredActionKeepsScopeAndOriginalMessage() throws {
        let summary = ConsentSummary(try prompt())
        #expect(summary.title == "Allow this action?")
        #expect(summary.message == "fetch")
        #expect(summary.resource == "github.com")
        #expect(summary.reason == "Capsule requests approval")
        #expect(summary.originalMessage == original)
    }

    @Test func incompleteOrOtherMetadataKeepsOriginalMessage() throws {
        for metadata in [
            "{\"version\":1,\"kind\":\"action_approval\",\"action\":\"fetch\",\"lifetimes\":[\"none\",\"none\"]}",
            "{\"version\":1,\"kind\":\"ingress\",\"lifetimes\":[\"session\",\"none\"]}",
            "{\"version\":2,\"kind\":\"action_approval\",\"action\":\"fetch\",\"resource\":\"github.com\",\"lifetimes\":[\"none\",\"none\"]}"
        ] {
            let summary = ConsentSummary(try prompt(metadata))
            #expect(summary.message == original)
            #expect(summary.originalMessage == nil)
            #expect(summary.resource == nil)
        }
        let generic = RuntimePromptRow(id: "id", requestID: "request", message: original,
                                       options: ["Allow", "Deny"])
        #expect(ConsentSummary(generic).message == original)
    }

    private func prompt(_ metadata: String = "{\"version\":1,\"kind\":\"action_approval\",\"action\":\"fetch\",\"resource\":\"github.com\",\"reason\":\"Capsule requests approval\",\"lifetimes\":[\"none\",\"none\"]}") throws -> RuntimePromptRow {
        let consent = try JSONDecoder().decode(ConsentPresentation.self, from: Data(metadata.utf8))
        return RuntimePromptRow(id: "id", requestID: "request", message: original,
                                options: ["Allow", "Deny"], consent: consent)
    }
}
