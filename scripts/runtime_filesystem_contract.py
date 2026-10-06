"""Native filesystem support shipped with Astrid 2026.9.0 and later."""

import re
import sys


def requires_native_filesystem(version: str) -> bool:
    stable = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-rc\.[1-9][0-9]*)?", version)
    if stable is None:
        raise ValueError("runtime filesystem contract requires canonical SemVer or numbered RC")
    return tuple(map(int, stable.groups())) >= (2026, 9, 0)


if __name__ == "__main__":
    print("required" if requires_native_filesystem(sys.argv[1]) else "optional")
