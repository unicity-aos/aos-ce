import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct NativeRuntimeInputCodecTests {
    private let id = UUID()
    private func frame(principal: String = "alice", topic: String = "astrid.v1.private.elicit.request",
                       source: UUID = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!,
                       kind: Any = "Secret", secretDefault: String? = nil) throws -> Data {
        var field: [String: Any] = ["key": "token", "prompt": "Enter your token", "field_type": kind]
        if let secretDefault { field["default"] = secretDefault }
        return try JSONSerialization.data(withJSONObject: ["topic": topic, "principal": principal,
            "source_id": source.uuidString, "payload": ["request_id": id.uuidString,
                "capsule_id": "example", "field": field]])
    }

    @Test func runtimeShapeReachesSecretForm() throws {
        let request = try NativeRuntimeInputCodec.request(frame(), principal: "alice")
        #expect(request.id == id)
        #expect(request.principal == "alice")
        #expect(request.capsule == "example")
        #expect(request.key == "token")
        #expect(request.kind == .secret)
        #expect(request.defaultValue == nil)
    }

    @Test func ordinaryFormsKeepTypedShapeAndDefaults() throws {
        let text = try NativeRuntimeInputCodec.request(
            frame(kind: "Text", secretDefault: "suggestion"), principal: "alice")
        #expect(text.kind == .text)
        #expect(text.defaultValue == "suggestion")
        let select = try NativeRuntimeInputCodec.request(
            frame(kind: ["Enum": ["one", "two"]], secretDefault: "two"), principal: "alice")
        #expect(select.kind == .select)
        #expect(select.options == ["one", "two"])
        #expect(select.defaultValue == "two")
        let array = try NativeRuntimeInputCodec.request(frame(kind: "Array"), principal: "alice")
        #expect(array.kind == .array)
        #expect(array.defaultValue == nil)
    }

    @Test func malformedChoicesAndUnsafeDefaultsAreRefused() throws {
        let invalid: [Data] = [
            try frame(kind: ["Enum": [String]()]),
            try frame(kind: ["Enum": ["one", "one"]]),
            try frame(kind: ["Enum": ["one"]], secretDefault: "invented"),
            try frame(kind: ["Enum": ["one"], "Other": ["two"]]),
            try frame(kind: ["Select": ["one"]]),
            try frame(kind: "Array", secretDefault: "not-a-list"),
            try frame(kind: "Secret", secretDefault: "do-not-display")
        ]
        for bytes in invalid {
            #expect(throws: NativeInputError.self) {
                try NativeRuntimeInputCodec.request(bytes, principal: "alice")
            }
        }
    }

    @Test func emptyOrdinaryInputIsNotCancellation() throws {
        let text = try NativeRuntimeInputCodec.request(frame(kind: "Text"), principal: "alice")
        let array = try NativeRuntimeInputCodec.request(frame(kind: "Array"), principal: "alice")
        let emptyText = try replyPayload(.value(""), to: text)
        #expect(emptyText["value"] as? String == "")
        #expect(emptyText["values"] == nil)
        let emptyList = try replyPayload(.values([]), to: array)
        #expect(emptyList["values"] as? [String] == [])
        #expect(emptyList["value"] == nil)
        for request in [text, array] {
            let cancelled = try replyPayload(.cancelled, to: request)
            #expect(cancelled["value"] == nil)
            #expect(cancelled["values"] == nil)
        }
    }

    @Test func formRepliesCannotInventChoicesOrCoerceShapes() throws {
        let select = try NativeRuntimeInputCodec.request(
            frame(kind: ["Enum": ["one", "two"]]), principal: "alice")
        #expect(try replyPayload(.value("two"), to: select)["value"] as? String == "two")
        #expect(throws: NativeInputError.self) {
            try NativeRuntimeInputCodec.privateReply(.value("invented"), to: select)
        }
        let array = try NativeRuntimeInputCodec.request(frame(kind: "Array"), principal: "alice")
        #expect(throws: NativeInputError.self) {
            try NativeRuntimeInputCodec.privateReply(.value("one,two"), to: array)
        }
        #expect(try replyPayload(.values(["one,two", "three"]), to: array)["values"]
            as? [String] == ["one,two", "three"])
    }

    private func replyPayload(_ answer: NativeInputAnswer, to request: NativeInputRequest)
        throws -> [String: Any] {
        let bytes = try NativeRuntimeInputCodec.privateReply(answer, to: request)
        let envelope = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        #expect(envelope["topic"] as? String == "astrid.v1.private.elicit.reply")
        return try #require(envelope["payload"] as? [String: Any])
    }

    @Test func wrongIdentityOriginOrFormIsRefused() throws {
        for data in [try frame(principal: "bob"), try frame(topic: "astrid.v1.admin.input"),
                     try frame(topic: "astrid.v1.elicit"),
                     try frame(source: UUID()), try frame(kind: "Unknown"),
                     try frame(secretDefault: "synthetic-never-display")] {
            #expect(throws: (any Error).self) {
                try NativeRuntimeInputCodec.request(data, principal: "alice")
            }
        }
    }

    @Test func answersHaveOnlyPrivateDestinationAndNoClaimedIdentity() throws {
        let request = try NativeRuntimeInputCodec.request(frame(), principal: "alice")
        let bytes = try NativeRuntimeInputCodec.privateReply(.value("synthetic-input"), to: request)
        let object = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        #expect(object["topic"] as? String == "astrid.v1.private.elicit.reply")
        #expect(object["principal"] == nil)
        #expect(object["device_key_id"] == nil)
        let payload = try #require(object["payload"] as? [String: Any])
        #expect(payload["type"] as? String == "elicit_response")
        #expect(payload["value"] as? String == "synthetic-input")
        #expect(payload["request_id"] as? String == id.uuidString)
        #expect(payload["values"] == nil)
        #expect(payload["capsule_id"] == nil)
        #expect(payload["key"] == nil)
    }

    @Test func cancellationOmitsValueAndInvalidAnswerCannotEncode() throws {
        let request = try NativeRuntimeInputCodec.request(frame(), principal: "alice")
        let bytes = try NativeRuntimeInputCodec.privateReply(.cancelled, to: request)
        let object = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        let payload = try #require(object["payload"] as? [String: Any])
        #expect(payload["value"] == nil)
        for answer in [NativeInputAnswer.value(""), .values(["x"])] {
            #expect(throws: (any Error).self) { try NativeRuntimeInputCodec.privateReply(answer, to: request) }
        }
    }

    @Test func deliveryMustMatchRequestAndPrincipal() throws {
        let request = try NativeRuntimeInputCodec.request(frame(), principal: "alice")
        for status in ["delivered", "unavailable", "forbidden", "invalid"] {
            let bytes = try result(status: status)
            #expect(try NativeRuntimeInputCodec.delivery(bytes, for: request).rawValue == status)
        }
        for bytes in [try result(status: "stored"), try result(principal: "bob"),
                      try result(requestID: UUID()), try result(topic: "astrid.v1.elicit")] {
            #expect(throws: (any Error).self) { try NativeRuntimeInputCodec.delivery(bytes, for: request) }
        }
    }

    @Test func oversizedAndMalformedFramesHaveGenericErrors() throws {
        for bytes in [Data(repeating: 32, count: PresentationLimits.maxFrameBytes + 1),
                      Data("synthetic-secret-not-json".utf8)] {
            do {
                _ = try NativeRuntimeInputCodec.request(bytes, principal: "alice")
                Issue.record("invalid frame accepted")
            } catch { #expect(String(describing: error) == "invalidRequest") }
        }
    }

    private func result(status: String = "delivered", principal: String = "alice",
                        requestID: UUID? = nil, topic: String = "astrid.v1.private.elicit.result") throws -> Data {
        try JSONSerialization.data(withJSONObject: ["topic": topic, "principal": principal,
            "payload": ["request_id": (requestID ?? id).uuidString, "status": status]])
    }
}
