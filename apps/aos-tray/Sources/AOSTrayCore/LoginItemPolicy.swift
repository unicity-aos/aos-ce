import Foundation

/// Restricts login-item registration to the stable per-user installation.
/// Preview and versioned release copies must never register themselves.
public enum LoginItemPolicy {
    public static let applicationName = "AOS Command Center.app"

    public static func shouldRegister(bundlePath: String, userHome: String) -> Bool {
        guard bundlePath.hasPrefix("/"), userHome.hasPrefix("/"),
              !bundlePath.contains("\0"), !userHome.contains("\0") else {
            return false
        }
        let home = userHome.hasSuffix("/") ? String(userHome.dropLast()) : userHome
        return bundlePath == home + "/Applications/" + applicationName
    }
}
