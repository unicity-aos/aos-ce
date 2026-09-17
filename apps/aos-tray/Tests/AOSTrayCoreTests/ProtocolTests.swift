import Foundation
import Testing
@testable import AOSTrayCore

@Suite
struct PresentationCodecTests {
    @Test func acceptsBoundedRequestAndIgnoresUnknownKeys() throws {
        let frame = """
        {"version":1,"id":"correlation-id","message":"Runtime-supplied explanation","options":[{"label":"Allow"},{"label":"Deny"}],"timeoutSeconds":120,"password":"nope","principal":"invent-me"}
        """.data(using: .utf8)!
        let request = try PresentationCodec.parseRequest(frame).get()
        #expect(request.id == "correlation-id")
        #expect(request.message == "Runtime-supplied explanation")
        #expect(request.options == ["Allow", "Deny"])
        #expect(request.timeoutSeconds == 120)
    }

    @Test func missingTimeoutIsRejected() {
        let frame = """
        {"version":1,"id":"a","message":"hello","options":[{"label":"Allow"}]}
        """.data(using: .utf8)!
        #expect(PresentationCodec.parseRequest(frame) == .failure(.invalidTimeout))
    }

    @Test func rejectsOutOfRangeFields() {
        #expect(PresentationCodec.parseRequest(Data()) == .failure(.malformedFrame))
        #expect(PresentationCodec.parseRequest(Data(repeating: 0x61, count: 16_385)) == .failure(.frameTooLarge))
        #expect(parse(id: "", message: "hello", options: 1, timeout: 120) == .failure(.invalidID))
        #expect(parse(id: String(repeating: "a", count: 129), message: "hello", options: 1, timeout: 120) == .failure(.invalidID))
        #expect(parse(id: "a", message: "", options: 1, timeout: 120) == .failure(.invalidMessage))
        #expect(parse(id: "a", message: String(repeating: "m", count: 4_097), options: 1, timeout: 120) == .failure(.invalidMessage))
        #expect(parse(id: "a", message: "hello", options: 0, timeout: 120) == .failure(.invalidOptions))
        #expect(parse(id: "a", message: "hello", options: 5, timeout: 120) == .failure(.invalidOptions))
        #expect(parse(id: "a", message: "hello", options: 1, timeout: 0) == .failure(.invalidTimeout))
        #expect(parse(id: "a", message: "hello", options: 1, timeout: 301) == .failure(.invalidTimeout))
        #expect(PresentationCodec.parseRequest(version: 2) == .failure(.unsupportedVersion))
    }

    @Test func emptyLabelAndOversizedLabelAreRejected() {
        let empty = """
        {"version":1,"id":"a","message":"hello","options":[{"label":""}],"timeoutSeconds":1}
        """.data(using: .utf8)!
        #expect(PresentationCodec.parseRequest(empty) == .failure(.invalidOptions))
        let huge = String(repeating: "x", count: 513)
        let oversized = """
        {"version":1,"id":"a","message":"hello","options":[{"label":"\(huge)"}],"timeoutSeconds":1}
        """.data(using: .utf8)!
        #expect(PresentationCodec.parseRequest(oversized) == .failure(.invalidOptions))
    }

    @Test func encodeCancelAndBoundSelection() throws {
        let payload = try PresentationCodec.encodeResponse(PresentationResponse(id: "correlation-id", selected: nil))
        let object = try JSONSerialization.jsonObject(with: payload) as? [String: Any]
        #expect(object?["version"] as? Int == 1)
        #expect(object?["id"] as? String == "correlation-id")
        #expect(object?["selected"] is NSNull)
        #expect(PresentationCodec.selection(0, optionCount: 2) == 0)
        #expect(PresentationCodec.selection(1, optionCount: 2) == 1)
        #expect(PresentationCodec.selection(2, optionCount: 2) == nil)
        #expect(PresentationCodec.selection(-1, optionCount: 2) == nil)
        #expect(PresentationCodec.selection(nil, optionCount: 2) == nil)
    }

    private func parse(id: String, message: String, options: Int, timeout: Int) -> Result<ValidatedPresentationRequest, ProtocolError> {
        let labels = (0..<options).map { "{\"label\":\"opt\($0)\"}" }.joined(separator: ",")
        let frame = """
        {"version":1,"id":"\(id)","message":"\(message)","options":[\(labels)],"timeoutSeconds":\(timeout)}
        """.data(using: .utf8)!
        return PresentationCodec.parseRequest(frame)
    }
}

private extension PresentationCodec {
    static func parseRequest(version: Int) -> Result<ValidatedPresentationRequest, ProtocolError> {
        let frame = """
        {"version":\(version),"id":"a","message":"hello","options":[{"label":"Allow"}],"timeoutSeconds":1}
        """.data(using: .utf8)!
        return parseRequest(frame)
    }
}
