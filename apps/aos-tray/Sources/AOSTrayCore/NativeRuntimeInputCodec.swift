import Foundation

/// Astrid's local IPC wire format, not MCP or the approval presentation socket.
/// Callers must establish the authenticated runtime connection and enable its
/// private responder before handing it frames. This codec authenticates nothing.
public enum NativeRuntimeInputCodec {
    private static let kernelSource = UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    public enum Delivery: String, Decodable, Sendable {
        case delivered, unavailable, forbidden, invalid
    }

    private struct RequestEnvelope: Decodable {
        let topic: String
        let principal: String
        let source_id: UUID
        let payload: RequestPayload
    }
    private struct RequestPayload: Decodable {
        let request_id: UUID
        let capsule_id: String
        let field: Field
    }
    private struct Field: Decodable {
        let key: String
        let prompt: String
        let field_type: FieldType
        let `default`: String?
    }
    /// Serde's externally tagged OnboardingFieldType: unit variants are strings,
    /// while Enum carries its exact options. Unknown shapes are never guessed.
    private enum FieldType: Decodable {
        case text, secret, array, select([String])

        init(from decoder: any Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let name = try? container.decode(String.self) {
                switch name {
                case "Text": self = .text
                case "Secret": self = .secret
                case "Array": self = .array
                default: throw NativeInputError.invalidRequest
                }
            } else {
                let tagged = try container.decode([String: [String]].self)
                guard tagged.count == 1, let options = tagged["Enum"] else {
                    throw NativeInputError.invalidRequest
                }
                self = .select(options)
            }
        }
    }
    private struct ResultEnvelope: Decodable {
        let topic: String
        let principal: String
        let payload: ResultPayload
    }
    private struct ResultPayload: Decodable {
        let request_id: UUID
        let status: Delivery
    }
    private struct Reply: Encodable {
        let topic = "astrid.v1.private.elicit.reply"
        let source_id = NativeRuntimeInputCodec.kernelSource
        let payload: ReplyPayload
    }
    private struct ReplyPayload: Encodable {
        let type = "elicit_response"
        let request_id: UUID
        let value: String?
        let values: [String]?
    }

    /// Only authenticated private-route notifications can create a native form.
    public static func request(_ data: Data, principal: String) throws -> NativeInputRequest {
        guard data.count <= PresentationLimits.maxFrameBytes else { throw NativeInputError.invalidRequest }
        do {
            let envelope = try JSONDecoder().decode(RequestEnvelope.self, from: data)
            guard envelope.topic == "astrid.v1.private.elicit.request", envelope.principal == principal,
                  envelope.source_id == kernelSource else { throw NativeInputError.invalidRequest }
            let field = envelope.payload.field
            let kind: NativeInputRequest.Kind
            let options: [String]?
            switch field.field_type {
            case .text: kind = .text; options = nil
            case .secret: kind = .secret; options = nil
            case .array: kind = .array; options = nil
            case .select(let choices): kind = .select; options = choices
            }
            let request = NativeInputRequest(id: envelope.payload.request_id, principal: principal,
                capsule: envelope.payload.capsule_id, key: field.key, prompt: field.prompt,
                kind: kind, options: options, defaultValue: field.default)
            try request.validate()
            return request
        } catch {
            // A decoder error may contain input. Never return that diagnostic.
            throw NativeInputError.invalidRequest
        }
    }

    /// Result is transient secret-bearing data: send only to the authenticated
    /// runtime socket, never a log, snapshot, MCP writer, or retry spool.
    public static func privateReply(_ answer: NativeInputAnswer, to request: NativeInputRequest) throws -> Data {
        try request.validateAnswer(answer)
        let value: String?
        let values: [String]?
        switch answer {
        case .cancelled: value = nil; values = nil
        case .value(let input): value = input; values = nil
        case .values(let input): value = nil; values = input
        }
        return try JSONEncoder().encode(Reply(payload: ReplyPayload(
            request_id: request.id, value: value, values: values)))
    }

    /// `delivered` is not a stored-secret receipt: the suspended host invocation
    /// still checks authority and performs storage after delivery.
    public static func delivery(_ data: Data, for request: NativeInputRequest) throws -> Delivery {
        guard data.count <= PresentationLimits.maxFrameBytes else { throw NativeInputError.invalidRequest }
        do {
            let envelope = try JSONDecoder().decode(ResultEnvelope.self, from: data)
            guard envelope.topic == "astrid.v1.private.elicit.result",
                  envelope.principal == request.principal,
                  envelope.payload.request_id == request.id else { throw NativeInputError.invalidRequest }
            return envelope.payload.status
        } catch { throw NativeInputError.invalidRequest }
    }
}
