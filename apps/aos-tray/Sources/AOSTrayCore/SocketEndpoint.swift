import Darwin
import Foundation

public struct SocketInode: Equatable, Sendable {
    public var device: UInt64
    public var inode: UInt64
}

public enum SocketEndpoint {
    public static var maxPathBytes: Int {
        MemoryLayout.size(ofValue: sockaddr_un().sun_path) - 1
    }

    public static func validateBindPath(_ path: String) -> Result<String, SocketEndpointError> {
        guard path.hasPrefix("/"), !path.hasSuffix("/"), !path.contains("\0") else {
            return .failure(.invalidPath)
        }
        let components = path.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        guard components.first == "" else {
            return .failure(.invalidPath)
        }
        var accumulated = ""
        for (index, component) in components.enumerated() {
            if index == 0 {
                accumulated = "/"
                continue
            }
            if component.isEmpty || component == "." || component == ".." {
                return .failure(.invalidPath)
            }
            if accumulated == "/" {
                accumulated = "/" + component
            } else {
                accumulated += "/" + component
            }
            let isLast = index == components.count - 1
            var info = stat()
            let status = lstat(accumulated, &info)
            if isLast {
                if status == 0 {
                    return .failure(.endpointExists)
                }
                if errno != ENOENT {
                    return .failure(.invalidPath)
                }
                continue
            }
            if status != 0 {
                return .failure(.missingDirectory)
            }
            if (info.st_mode & S_IFMT) == S_IFLNK {
                return .failure(.symlinkComponent)
            }
            if (info.st_mode & S_IFMT) != S_IFDIR {
                return .failure(.missingDirectory)
            }
        }

        let parent = (path as NSString).deletingLastPathComponent
        var parentInfo = stat()
        guard lstat(parent, &parentInfo) == 0 else {
            return .failure(.missingDirectory)
        }
        if (parentInfo.st_mode & S_IFMT) == S_IFLNK {
            return .failure(.symlinkComponent)
        }
        guard (parentInfo.st_mode & S_IFMT) == S_IFDIR else {
            return .failure(.missingDirectory)
        }
        guard parentInfo.st_uid == getuid() else {
            return .failure(.directoryNotOwned)
        }
        guard (parentInfo.st_mode & 0o777) == 0o700 else {
            return .failure(.directoryNotPrivate)
        }
        let pathBytes = path.utf8.count
        guard pathBytes > 0, pathBytes <= maxPathBytes else {
            return .failure(.pathTooLong)
        }
        return .success(path)
    }

    public static func inode(at path: String) -> SocketInode? {
        var info = stat()
        guard lstat(path, &info) == 0 else { return nil }
        return SocketInode(device: UInt64(info.st_dev), inode: UInt64(info.st_ino))
    }

    public static func chmodSocket(_ path: String) -> Bool {
        chmod(path, 0o600) == 0
    }

    public static func unlinkIfOwned(_ path: String, expected: SocketInode) {
        guard let current = inode(at: path), current == expected else { return }
        _ = unlink(path)
    }
}

public enum SocketEndpointError: Equatable, Error, Sendable {
    case invalidPath
    case pathTooLong
    case missingDirectory
    case directoryNotOwned
    case directoryNotPrivate
    case symlinkComponent
    case endpointExists

    public var message: String {
        switch self {
        case .invalidPath:
            return "socket path must be an explicit absolute file path"
        case .pathTooLong:
            return "socket path exceeds the Unix sun_path limit"
        case .missingDirectory:
            return "socket directory must already exist"
        case .directoryNotOwned:
            return "socket directory must be owned by the current user"
        case .directoryNotPrivate:
            return "socket directory must have mode 0700"
        case .symlinkComponent:
            return "socket path must not contain symlink components"
        case .endpointExists:
            return "refusing to replace an existing socket endpoint"
        }
    }
}
