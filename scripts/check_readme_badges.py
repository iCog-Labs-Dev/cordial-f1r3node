#!/usr/bin/env python3
"""Keep the README's repository count badges aligned with source files."""

import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def manifest(path: Path) -> dict:
    with path.open("rb") as source:
        return tomllib.load(source)


def main() -> int:
    workspace = manifest(ROOT / "Cargo.toml")
    members = workspace["workspace"]["members"]
    benchmarks = sum(
        len(manifest(ROOT / member / "Cargo.toml").get("bench", []))
        for member in members
    )
    property_suites = sum(
        len(list((ROOT / member / "tests").glob("prop_*.rs")))
        for member in members
    )
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    counts = (
        ("workspace crates", r"/badge/workspace-(\d+)%20crates-", len(members)),
        ("benchmark targets", r"/badge/bench%20targets-(\d+)-", benchmarks),
        ("property suites", r"/badge/property%20suites-(\d+)-", property_suites),
    )
    for label, pattern, expected in counts:
        match = re.search(pattern, readme)
        if match is None or int(match.group(1)) != expected:
            print(f"README badge for {label} must say {expected}", file=sys.stderr)
            return 1
        print(f"{label}: {expected}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
