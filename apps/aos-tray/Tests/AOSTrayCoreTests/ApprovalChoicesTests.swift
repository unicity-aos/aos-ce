import Foundation
import Testing
@testable import AOSTrayCore

struct ApprovalChoicesTests {
    @Test func reorderedChoicesRetainOriginalIndices() throws {
        let data = Data(#"{"version":1,"kind":"action_approval","lifetimes":["none","durable","none","session"]}"#.utf8)
        let consent = try JSONDecoder().decode(ConsentPresentation.self, from: data)
        var prompt = RuntimePromptRow(id: "p", requestID: "r", message: "Test",
            options: ["Deny", "Always Approve", "Approve Once", "Approve for Session"], consent: consent)
        let choices = try #require(ApprovalChoices(prompt))
        #expect(choices.once == 2)
        #expect(choices.session == 3)
        #expect(choices.remembered == 1)
        #expect(choices.deny == 0)
        #expect(!choices.restartOnly)
        prompt.options[1] = "Something else"
        #expect(ApprovalChoices(prompt) == nil)
        prompt.consent = nil
        #expect(ApprovalChoices(prompt) == nil)
    }

    @Test func legacyRestartIsNotCalledDurable() throws {
        let data = Data(#"{"version":1,"kind":"action_approval","lifetimes":["none","session","until_runtime_restart","none"]}"#.utf8)
        let consent = try JSONDecoder().decode(ConsentPresentation.self, from: data)
        let prompt = RuntimePromptRow(id: "p", requestID: "r", message: "Test",
            options: ["Approve Once", "Approve for Session", "Until runtime restart", "Deny"], consent: consent)
        #expect(ApprovalChoices(prompt)?.restartOnly == true)
    }
}
