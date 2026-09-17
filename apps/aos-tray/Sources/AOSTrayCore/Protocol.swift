import Foundation

public enum PresentationLimits {
    public static let maxFrameBytes = 16_384
    public static let maxMessageBytes = 4_096
    public static let minIDBytes = 1
    public static let maxIDBytes = 128
    public static let minOptions = 1
    public static let maxOptions = 4
    public static let minTimeoutSeconds = 1
    public static let maxTimeoutSeconds = 300
    public static let maxLabelBytes = 512
}

public struct ValidatedPresentationRequest: Equatable, Sendable {
    public var id: String
    public var message: String
    public var options: [String]
    public var timeoutSeconds: Int
    public var consent: ConsentPresentation?

    public init(id: String, message: String, options: [String], timeoutSeconds: Int,
                consent: ConsentPresentation? = nil) {
        self.id = id
        self.message = message
        self.options = options
        self.timeoutSeconds = timeoutSeconds
        self.consent = consent
    }
}

public struct PresentationResponse: Equatable, Sendable {
    public var id: String
    public var selected: Int?

    public init(id: String, selected: Int?) {
        self.id = id
        self.selected = selected
    }
}

public enum PresentationCodec {
    public static func parseRequest(_ frame: Data) -> Result<ValidatedPresentationRequest, ProtocolError> {
        guard !frame.isEmpty else {
            return .failure(.malformedFrame)
        }
        guard frame.count <= PresentationLimits.maxFrameBytes else {
            return .failure(.frameTooLarge)
        }
        guard let object = try? JSONSerialization.jsonObject(with: frame) as? [String: Any] else {
            return .failure(.malformedFrame)
        }
        guard integer(object["version"]) == 1 else {
            return .failure(.unsupportedVersion)
        }
        guard let id = object["id"] as? String else {
            return .failure(.invalidID)
        }
        let idBytes = id.utf8.count
        guard idBytes >= PresentationLimits.minIDBytes, idBytes <= PresentationLimits.maxIDBytes else {
            return .failure(.invalidID)
        }
        guard let message = object["message"] as? String else {
            return .failure(.invalidMessage)
        }
        let messageBytes = message.utf8.count
        guard messageBytes >= 1, messageBytes <= PresentationLimits.maxMessageBytes else {
            return .failure(.invalidMessage)
        }
        guard let rawOptions = object["options"] as? [Any] else {
            return .failure(.invalidOptions)
        }
        guard (PresentationLimits.minOptions...PresentationLimits.maxOptions).contains(rawOptions.count) else {
            return .failure(.invalidOptions)
        }
        var labels: [String] = []
        labels.reserveCapacity(rawOptions.count)
        for item in rawOptions {
            guard let option = item as? [String: Any], let label = option["label"] as? String else {
                return .failure(.invalidOptions)
            }
            let labelBytes = label.utf8.count
            guard labelBytes >= 1, labelBytes <= PresentationLimits.maxLabelBytes else {
                return .failure(.invalidOptions)
            }
            labels.append(label)
        }
        guard let timeout = integer(object["timeoutSeconds"]),
              (PresentationLimits.minTimeoutSeconds...PresentationLimits.maxTimeoutSeconds).contains(timeout)
        else {
            return .failure(.invalidTimeout)
        }
        var consent: ConsentPresentation?
        if let rawConsent = object["consent"] {
            guard let consentObject = rawConsent as? [String: Any],
                  let data = try? JSONSerialization.data(withJSONObject: consentObject),
                  let decoded = try? JSONDecoder().decode(ConsentPresentation.self, from: data),
                  decoded.isValid(optionCount: labels.count)
            else { return .failure(.invalidConsent) }
            consent = decoded
        }
        return .success(
            ValidatedPresentationRequest(
                id: id,
                message: message,
                options: labels,
                timeoutSeconds: timeout,
                consent: consent
            )
        )
    }

    public static func encodeResponse(_ response: PresentationResponse) throws -> Data {
        let selected: Any = response.selected.map { $0 as Any } ?? NSNull()
        let object: [String: Any] = [
            "version": 1,
            "id": response.id,
            "selected": selected,
        ]
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        guard data.count <= PresentationLimits.maxFrameBytes else {
            throw ProtocolError.frameTooLarge
        }
        return data
    }

    public static func selection(_ selected: Int?, optionCount: Int) -> Int? {
        guard let selected else { return nil }
        guard selected >= 0, selected < optionCount else { return nil }
        return selected
    }

    private static func integer(_ value: Any?) -> Int? {
        switch value {
        case let number as Int:
            return number
        case let number as NSNumber:
            guard CFGetTypeID(number) != CFBooleanGetTypeID() else { return nil }
            let doubleValue = number.doubleValue
            guard doubleValue.rounded() == doubleValue else { return nil }
            return number.intValue
        default:
            return nil
        }
    }
}

public enum ProtocolError: Equatable, Error, Sendable {
    case malformedFrame
    case frameTooLarge
    case unsupportedVersion
    case invalidID
    case invalidMessage
    case invalidOptions
    case invalidTimeout
    case invalidConsent
}
