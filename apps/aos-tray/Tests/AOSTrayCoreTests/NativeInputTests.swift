import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct NativeInputTests {
    private func request(_ kind: String, extra: [String: Any] = [:]) throws -> NativeInputRequest {
        var object: [String: Any] = ["id": UUID().uuidString, "principal": "codex-code",
            "capsule": "example", "key": "setting", "prompt": "Enter a value", "kind": kind]
        object.merge(extra) { _, new in new }
        return try NativeInputRequest.decode(JSONSerialization.data(withJSONObject: object))
    }

    @Test func textAndCancellationAreDifferent() throws {
        let input = try request("text")
        try input.validateAnswer(.value(""))
        try input.validateAnswer(.cancelled)
        #expect(throws: (any Error).self) { try input.validateAnswer(.values([])) }
    }

    @Test func secretHasNoDefaultAndRequiresNonemptyValue() throws {
        #expect(throws: (any Error).self) { try request("secret", extra: ["defaultValue": "not-a-real-secret"]) }
        let input = try request("secret")
        #expect(throws: (any Error).self) { try input.validateAnswer(.value("")) }
        try input.validateAnswer(.value("synthetic-test-value"))
        try input.validateAnswer(.cancelled)
    }

    @Test func selectionsCannotInventAnOption() throws {
        let input = try request("select", extra: ["options": ["a", "b"], "defaultValue": "a"])
        try input.validateAnswer(.value("b"))
        #expect(throws: (any Error).self) { try input.validateAnswer(.value("c")) }
        #expect(throws: (any Error).self) { try request("select", extra: ["options": ["a", "a"]]) }
        #expect(throws: (any Error).self) { try request("select", extra: ["options": ["a"], "defaultValue": "b"]) }
    }

    @Test func arraysAreBoundedAndNeverCoercedFromText() throws {
        let input = try request("array")
        try input.validateAnswer(.values(["one", "two"]))
        #expect(throws: (any Error).self) { try input.validateAnswer(.value("one,two")) }
        #expect(throws: (any Error).self) { try input.validateAnswer(.values(Array(repeating: "x", count: 65))) }
        #expect(throws: (any Error).self) { try input.validateAnswer(.values([String(repeating: "x", count: 4097)])) }
    }

    @Test func unsupportedFieldsAndOversizedAnswersFailClosed() throws {
        #expect(throws: (any Error).self) { try request("password") }
        #expect(throws: (any Error).self) { try request("text", extra: ["principal": ""]) }
        #expect(throws: (any Error).self) { try request("text", extra: ["options": ["a"]]) }
        let input = try request("text")
        #expect(throws: (any Error).self) { try input.validateAnswer(.value(String(repeating: "x", count: 4097))) }
    }

    @Test func secretDraftClearsAndDoesNotReuseInputAcrossRequests() throws {
        let input = try request("secret")
        var draft = NativeInputDraft(request: input)
        #expect(!draft.canSubmit(input))
        draft.secret = "synthetic-only"
        #expect(draft.canSubmit(input))
        draft.clearSecrets()
        #expect(draft.secret.isEmpty)
        #expect(!draft.canSubmit(input))
        #expect(NativeInputDraft(request: try request("secret")).secret.isEmpty)
    }

    @Test func listEditorKeepsDistinctEntriesAndAllowsAnExplicitEmptyList() throws {
        let input = try request("array")
        var draft = NativeInputDraft(request: input)
        draft.rows[0].value = "one,two"
        draft.addRow()
        draft.rows[1].value = "three"
        #expect(draft.answer(for: input) == .values(["one,two", "three"]))
        for row in draft.rows { draft.removeRow(id: row.id) }
        #expect(draft.answer(for: input) == .values([]))
        #expect(draft.canSubmit(input))
        for _ in 0..<100 { draft.addRow() }
        #expect(draft.rows.count == NativeInputRequest.maxItems)
    }

    @Test func editorPreservesDefaultsAndRejectsInventedSelection() throws {
        let text = try request("text", extra: ["defaultValue": "existing"])
        #expect(NativeInputDraft(request: text).text == "existing")
        let selection = try request("select", extra: ["options": ["a", "b"], "defaultValue": "b"])
        var draft = NativeInputDraft(request: selection)
        #expect(draft.selection == "b")
        draft.selection = "outside"
        #expect(!draft.canSubmit(selection))
    }

    @Test func diagnosticsNeverDescribeInputValues() throws {
        let sentinel = "synthetic-diagnostic-sentinel"
        var draft = NativeInputDraft(request: try request("secret"))
        draft.secret = sentinel
        #expect(!String(describing: draft).contains(sentinel))
        #expect(!String(reflecting: draft).contains(sentinel))
        for answer in [NativeInputAnswer.value(sentinel), .values([sentinel])] {
            #expect(!String(describing: answer).contains(sentinel))
            #expect(!String(reflecting: answer).contains(sentinel))
        }
    }

    @Test func invalidReplyDoesNotConsumeRequestButValidReplyIsSingleUse() throws {
        var resolution = try NativeInputResolution(request: request("secret"))
        #expect(resolution.take(.value("")) == nil)
        #expect(!resolution.isFinished)
        #expect(resolution.take(.value("synthetic")) == .value("synthetic"))
        #expect(resolution.take(.value("late")) == nil)
        #expect(resolution.take(.cancelled) == nil)
    }

    @Test func closedOrDisconnectedInputCannotBeSubmittedLater() throws {
        var resolution = try NativeInputResolution(request: request("text"))
        #expect(resolution.take(.cancelled) == .cancelled)
        #expect(resolution.isFinished)
        #expect(resolution.take(.value("late")) == nil)
    }
}
