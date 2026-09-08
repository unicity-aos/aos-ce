#!/usr/bin/env python3
"""Extract a curated release section without rewriting published history."""

import argparse
from pathlib import Path
import re


def notes(text: str, version: str) -> str:
    matches = list(re.finditer(r"^## \[([^]]+)\].*$", text, re.MULTILINE))
    selected = [i for i, match in enumerate(matches) if match[1] == version]
    if len(selected) != 1:
        raise ValueError(f"expected one changelog section for {version}")
    index = selected[0]
    end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
    result = text[matches[index].end():end].strip()
    if not result:
        raise ValueError(f"empty changelog section for {version}")
    return result + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    args = parser.parse_args()
    try:
        print(notes(args.changelog.read_text(), args.version), end="")
    except ValueError as error:
        parser.exit(1, f"{error}\n")
