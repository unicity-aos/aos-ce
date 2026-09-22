import Foundation

/// Product macOS floors. Finder/FSKit mounts are optional and 26-only.
public enum DarwinPlatform {
    public static let commandCenterMajor = 13
    public static let finderVolumeMountMajor = 26

    public static let finderVolumeMountUnavailable =
        "Finder volume mounting needs macOS 26 and an approved Astrid filesystem extension. AOS itself still runs."

    public static func finderVolumeMountAvailable(
        version: OperatingSystemVersion = ProcessInfo.processInfo.operatingSystemVersion
    ) -> Bool {
        version.majorVersion >= finderVolumeMountMajor
    }
}
