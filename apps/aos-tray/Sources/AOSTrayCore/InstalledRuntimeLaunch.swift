import Darwin
import Foundation

/// Finder / no-args launch binding for the current-user AOS install.
/// Explicit CLI pairs, demo, snapshot, and socket presenter modes are unchanged.
public struct InstalledRuntimeLaunch: Equatable, Sendable {
    public var aosBinary: String?
    public var aosHome: String?
    public var expectedBinary: String?
    public var launchError: String?

    public static func apply(
        arguments: LaunchArguments,
        environment: [String: String],
        userHome: String,
        binaryExists: (String) -> Bool = InstalledRuntimeLaunch.isUsableBinary
    ) -> InstalledRuntimeLaunch {
        if arguments.mode == .demo || arguments.snapshot || arguments.socketPath != nil {
            return InstalledRuntimeLaunch(
                aosBinary: arguments.aosBinary,
                aosHome: arguments.aosHome,
                expectedBinary: arguments.aosBinary,
                launchError: nil
            )
        }
        if let binary = arguments.aosBinary, let home = arguments.aosHome {
            return InstalledRuntimeLaunch(
                aosBinary: binary,
                aosHome: home,
                expectedBinary: binary,
                launchError: nil
            )
        }

        guard let home = defaultHome(environment: environment, userHome: userHome) else {
            return InstalledRuntimeLaunch(
                aosBinary: nil,
                aosHome: nil,
                expectedBinary: nil,
                launchError: NativeRuntimeSetupCopy.invalidHome
            )
        }
        let binary = home + "/bin/aos"
        if binaryExists(binary) {
            return InstalledRuntimeLaunch(
                aosBinary: binary,
                aosHome: home,
                expectedBinary: binary,
                launchError: nil
            )
        }
        return InstalledRuntimeLaunch(
            aosBinary: nil,
            aosHome: home,
            expectedBinary: binary,
            launchError: NativeRuntimeSetupCopy.missingBinary(binary)
        )
    }

    public static func isUsableBinary(_ path: String) -> Bool {
        guard path.hasPrefix("/"), !path.contains("\0") else { return false }
        var info = stat()
        guard path.withCString({ stat($0, &info) }) == 0 else { return false }
        guard (info.st_mode & S_IFMT) == S_IFREG else { return false }
        return access(path, X_OK) == 0
    }

    private static func defaultHome(
        environment: [String: String],
        userHome: String
    ) -> String? {
        if let override = environment["AOS_HOME"] {
            let trimmed = override.trimmingCharacters(in: .whitespacesAndNewlines)
            return NativeRuntimeSetup.realHome(trimmed)
        }
        guard userHome.hasPrefix("/"), !userHome.contains("\0") else {
            return nil
        }
        let trimmed = userHome.hasSuffix("/") ? String(userHome.dropLast()) : userHome
        return NativeRuntimeSetup.realHome(trimmed + "/.aos")
    }
}
