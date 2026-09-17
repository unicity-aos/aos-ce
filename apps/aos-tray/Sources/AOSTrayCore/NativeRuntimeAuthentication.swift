import CryptoKit
import Foundation

extension NativeRuntimeSocket {
    /// Prove an explicitly supplied device key to Astrid. No anonymous/token-only
    /// downgrade, key discovery, credential persistence, or automatic registration.
    public func authenticate(principal: String, token: Data, signingKey: Curve25519.Signing.PrivateKey,
                             timeout: TimeInterval) async throws {
        struct Request: Encodable {
            let token: String
            let protocol_version = 1
            let client_version = "aos-native-input"
            let claimed_principal: String
            var signature: String?
        }
        struct Response: Decodable {
            let status: String
            let protocol_version: Int
            let challenge: String?
        }
        do {
            guard token.count == 32, !principal.isEmpty, principal.utf8.count <= PresentationLimits.maxIDBytes,
                  !principal.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else {
                throw NativeRuntimeSocketError.authenticationFailed
            }
            var request = Request(token: token.map { String(format: "%02x", $0) }.joined(),
                                  claimed_principal: principal)
            try await writeFrame(JSONEncoder().encode(request), timeout: timeout)
            let challengeBytes = try await readFrame(timeout: timeout)
            guard challengeBytes.count <= 4096 else { throw NativeRuntimeSocketError.authenticationFailed }
            let challenge = try JSONDecoder().decode(Response.self, from: challengeBytes)
            guard challenge.status == "ok", challenge.protocol_version == 1,
                  let nonce = challenge.challenge, nonce.utf8.count == 64,
                  nonce.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
                throw NativeRuntimeSocketError.authenticationFailed
            }
            let message = Data("astrid-principal-auth:v1:\(principal):\(nonce)".utf8)
            request.signature = try signingKey.signature(for: message).map { String(format: "%02x", $0) }.joined()
            try await writeFrame(JSONEncoder().encode(request), timeout: timeout)
            let resultBytes = try await readFrame(timeout: timeout)
            guard resultBytes.count <= 4096 else { throw NativeRuntimeSocketError.authenticationFailed }
            let result = try JSONDecoder().decode(Response.self, from: resultBytes)
            guard result.status == "ok", result.protocol_version == 1, result.challenge == nil else {
                throw NativeRuntimeSocketError.authenticationFailed
            }
        } catch {
            close()
            throw NativeRuntimeSocketError.authenticationFailed
        }
    }
}
