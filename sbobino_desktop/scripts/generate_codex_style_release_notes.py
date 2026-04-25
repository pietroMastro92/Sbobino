#!/usr/bin/env python3
"""Render Codex-style release notes from commit history.

Buckets commits between two refs by Conventional Commits prefix:
  - feat / feat(...) -> New Features
  - fix  / fix(...)  -> Bug Fixes
  - docs / docs(...) -> Documentation
  - everything else  -> Chores

PR numbers `(#NN)` mentioned in the subject (or merge commits like
`Merge pull request #NN from ...`) are appended to the bullet so each
line ends with `(#NN, #MM)` Codex-style.

Usage:
  generate_codex_style_release_notes.py <version> <previous-ref> <current-ref> [--out PATH]

If --out is omitted the rendered Markdown is written to stdout.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable

CATEGORY_RULES = [
    ("New Features", ("feat",)),
    ("Bug Fixes", ("fix",)),
    ("Documentation", ("docs", "doc")),
]

# Anything else (chore, style, refactor, test, perf, build, ci, revert,
# unprefixed, merge commits, ...) lands in Chores.
DEFAULT_CATEGORY = "Chores"

PR_PATTERN = re.compile(r"#(\d+)")
CONVENTIONAL_PREFIX = re.compile(
    r"^(?P<type>[a-z]+)(?:\([^)]+\))?(?P<bang>!)?:\s*(?P<rest>.+)$",
    re.IGNORECASE,
)
MERGE_PR_PATTERN = re.compile(r"^Merge pull request #(\d+) from .+$", re.IGNORECASE)


def git_log(prev: str, curr: str) -> list[str]:
    """Return one-line subjects in ascending chronological order. Merge commits
    are omitted because the squashed/feature commits they reference already
    carry the meaningful subject."""
    rev_range = f"{prev}..{curr}" if prev else curr
    out = subprocess.check_output(
        [
            "git",
            "log",
            "--pretty=format:%s",
            "--reverse",
            "--no-merges",
            rev_range,
        ],
        text=True,
    )
    return [line for line in out.splitlines() if line.strip()]


def categorize(subject: str) -> tuple[str, str, list[str]]:
    """Return (category, cleaned_text, pr_numbers) for a single commit subject."""
    pr_numbers = PR_PATTERN.findall(subject)

    merge_match = MERGE_PR_PATTERN.match(subject)
    if merge_match:
        # Merge commits carry only the PR number; use the squash subject from
        # the linked PR if available later. Fall back to the merge text.
        return (
            DEFAULT_CATEGORY,
            f"Merged pull request #{merge_match.group(1)}.",
            [merge_match.group(1)],
        )

    match = CONVENTIONAL_PREFIX.match(subject)
    if match:
        commit_type = match.group("type").lower()
        rest = match.group("rest").strip()
        for category, prefixes in CATEGORY_RULES:
            if commit_type in prefixes:
                return category, rest, pr_numbers
        return DEFAULT_CATEGORY, rest, pr_numbers

    # Unprefixed commit: stays in Chores with the original subject.
    return DEFAULT_CATEGORY, subject, pr_numbers


def deduplicate_preserve_order(items: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            ordered.append(item)
    return ordered


def format_bullet(text: str, pr_numbers: list[str]) -> str:
    text = text.strip().rstrip(".")
    if text:
        text = text[0].upper() + text[1:]
    if pr_numbers:
        refs = ", ".join(f"#{n}" for n in deduplicate_preserve_order(pr_numbers))
        return f"- {text}. ({refs})"
    return f"- {text}."


def render(version: str, buckets: dict[str, list[str]]) -> str:
    lines: list[str] = [f"## Sbobino {version}", ""]
    section_order = [name for name, _ in CATEGORY_RULES] + [DEFAULT_CATEGORY]
    for section in section_order:
        bullets = buckets.get(section, [])
        if not bullets:
            continue
        lines.append(f"### {section}")
        lines.append("")
        lines.extend(bullets)
        lines.append("")
    if lines[-1] == "":
        lines.pop()
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="Release version without leading v")
    parser.add_argument("previous_ref", help="Git ref of the previous release tag")
    parser.add_argument("current_ref", help="Git ref of the current release tag or HEAD")
    parser.add_argument("--out", type=Path, default=None, help="Output file (default: stdout)")
    args = parser.parse_args()

    subjects = git_log(args.previous_ref, args.current_ref)

    buckets: dict[str, list[str]] = {}
    for subject in subjects:
        category, text, pr_numbers = categorize(subject)
        buckets.setdefault(category, []).append(format_bullet(text, pr_numbers))

    rendered = render(args.version, buckets)
    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
