/// First valid completion wins. Stores request metadata, never an answer.
/// The window and transport cancellation path share this state.
public struct NativeInputResolution {
    private let request: NativeInputRequest
    public private(set) var isFinished = false

    public init(request: NativeInputRequest) throws {
        try request.validate()
        self.request = request
    }

    public mutating func take(_ answer: NativeInputAnswer) -> NativeInputAnswer? {
        guard !isFinished, (try? request.validateAnswer(answer)) != nil else { return nil }
        isFinished = true
        return answer
    }
}
