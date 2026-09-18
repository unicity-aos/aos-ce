import Foundation
import ServiceManagement
import AOSTrayCore

@MainActor
enum LoginItemRegistration {
    static func registerInstalledApplication() throws {
        let bundlePath = Bundle.main.bundlePath
        guard LoginItemPolicy.shouldRegister(
            bundlePath: bundlePath,
            userHome: FileManager.default.homeDirectoryForCurrentUser.path
        ) else {
            return
        }

        switch SMAppService.mainApp.status {
        case .notRegistered:
            try SMAppService.mainApp.register()
        case .enabled, .requiresApproval:
            return
        case .notFound:
            throw NSError(
                domain: "ai.unicity.aos.tray.login-item",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "macOS could not find the installed Command Center login item"]
            )
        @unknown default:
            return
        }
    }
}
