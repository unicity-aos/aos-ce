import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct ConsentPresentationTests {
    @Test func explicitScopeDoesNotAlterResponse() throws {
        let request = try decode(consent: [
            "version": 1, "kind": "capsule_access", "capsule": "notes",
            "principal": "codex-code", "lifetimes": ["durable", "none"],
        ]).get()
        #expect(request.consent?.kind == .capsuleAccess)
        #expect(request.consent?.principal == "codex-code")
        #expect(request.consent?.resource == nil)
        #expect(request.consent?.lifetimes == [.durable, .none])
        let data = try PresentationCodec.encodeResponse(.init(id: request.id, selected: 0))
        let response = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        #expect(Set(response.keys) == ["version", "id", "selected"])
        #expect(response["selected"] as? Int == 0)
    }

    @Test func malformedMetadataCannotMislabelChoices() throws {
        for consent: Any in [
            "not an object",
            ["version": 2, "kind": "capsule_access", "lifetimes": ["durable", "none"]],
            ["version": 1, "kind": "unknown", "lifetimes": ["durable", "none"]],
            ["version": 1, "kind": "action_approval", "lifetimes": ["durable"]],
            ["version": 1, "kind": "action_approval", "lifetimes": ["forever", "none"]],
            ["version": 1, "kind": "capsule_access", "capsule": "", "lifetimes": ["durable", "none"]],
        ] {
            #expect(try decode(consent: consent) == .failure(.invalidConsent))
        }
    }

    @Test func missingMetadataRemainsGeneric() throws {
        let request = try decode(consent: nil).get()
        #expect(request.consent == nil)
        #expect(request.options == ["Allow", "Deny"])
    }

    private func decode(consent: Any?) throws -> Result<ValidatedPresentationRequest, ProtocolError> {
        var object: [String: Any] = [
            "version": 1, "id": "request", "message": "Runtime request",
            "options": [["label": "Allow"], ["label": "Deny"]], "timeoutSeconds": 120,
        ]
        if let consent { object["consent"] = consent }
        return PresentationCodec.parseRequest(try JSONSerialization.data(withJSONObject: object))
    }
}
