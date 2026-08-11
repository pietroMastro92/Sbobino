#!/usr/bin/env python3
"""Generate and validate the release notes used by every Sbobino channel.

The release body is deliberately treated as a build input, rather than as
text embedded in a shell script.  A versioned notes file can be supplied with
``--notes-file``; it is validated and copied byte-for-byte to the output.  If
no notes file is supplied, the script renders a Codex-style body from the
commits in ``previous_ref..current_ref``.  The latter mode is useful for local
inspection and keeps the renderer independently testable.

Required sections are ``Fixes``, ``New and improved``, ``Compatibility`` and
``Refs``.  ``Known issues`` is optional, but when present it must contain a
real bullet or an explicit ``None``.  Empty, placeholder, and generic release
notes are rejected before they can be published.

Examples::

  generate_codex_style_release_notes.py 2.0.27 v2.0.26 HEAD \
      --notes-file docs/release-notes/v2.0.27.md --out dist/release-notes.md
  generate_codex_style_release_notes.py 2.0.27 --notes-file dist/release-notes.md --check
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Mapping, Sequence

REQUIRED_SECTIONS: tuple[str, ...] = (
    "Fixes",
    "New and improved",
    "Compatibility",
    "Refs",
)
OPTIONAL_SECTIONS: tuple[str, ...] = ("Known issues",)
SECTION_ORDER: tuple[str, ...] = REQUIRED_SECTIONS[:3] + OPTIONAL_SECTIONS + ("Refs",)

# Conventional Commit prefixes remain supported for the history-rendering
# mode.  Subject keywords are intentionally broad enough for this repository's
# older, non-Conventional history.
FIX_TYPES = {"fix", "bug", "bugfix", "hotfix", "revert"}
NEW_TYPES = {"feat", "feature", "improve", "perf", "refactor", "build", "chore", "test", "ci", "docs"}
# Kept as public metadata for scripts that used the original renderer's
# category table.  Product-facing output uses the release section names.
CATEGORY_RULES = [
    ("New and improved", ("feat", "feature", "improve", "perf", "refactor")),
    ("Fixes", ("fix", "bug", "bugfix", "hotfix", "revert")),
    ("Compatibility", ("compat",)),
]
DEFAULT_CATEGORY = "New and improved"
FIX_KEYWORDS = re.compile(
    r"\b(fix(?:es|ed|ing)?|bug|crash|fallback|guard|stabiliz|repair|recover|prevent|fail(?:ure|ed)?)\b",
    re.IGNORECASE,
)
COMPAT_KEYWORDS = re.compile(
    r"\b(compat|compatible|architecture|architectures|arm64|aarch64|intel|x86_64|windows|macos|mac|runtime|updater|packag|release)\b",
    re.IGNORECASE,
)
CONVENTIONAL_PREFIX = re.compile(
    r"^(?P<type>[a-z]+)(?:\([^)]+\))?(?P<bang>!)?:\s*(?P<rest>.+)$",
    re.IGNORECASE,
)
COMMIT_HEADER = re.compile(r"^(?P<sha>[0-9a-f]{7,40})\t(?P<subject>.+)$", re.IGNORECASE)
HEADING = re.compile(r"^###\s+(?P<name>[^#].*?)\s*$")
PR_PATTERN = re.compile(r"#(\d+)")
MERGE_PR_PATTERN = re.compile(r"^Merge pull request #(\d+) from .+$", re.IGNORECASE)
VERSION_RANGE_PATTERN = re.compile(r"v\d+(?:\.\d+){2}\.\.v\d+(?:\.\d+){2}")
COMMIT_REF_PATTERN = re.compile(r"\b[0-9a-f]{7,40}\b", re.IGNORECASE)

GENERIC_BULLETS = {
    "...",
    "changes",
    "changelog",
    "improvements",
    "miscellaneous changes",
    "miscellaneous improvements",
    "minor fixes",
    "n/a",
    "none",
    "release updates",
    "same as before",
    "tbd",
    "todo",
    "various bug fixes",
    "various improvements",
}
PLACEHOLDER_MARKERS = ("todo", "tbd", "replace me", "placeholder", "all caps", "...", "…")
GENERIC_PATTERN = re.compile(
    r"^(?:bug\s+fix(?:es)?|fix(?:es)?|general\s+improvements?|minor\s+(?:fix(?:es)?|improvements?)|"
    r"misc(?:ellaneous)?\s+(?:changes|improvements?)|various\s+(?:changes|fix(?:es)?|improvements?))$",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class Commit:
    sha: str
    subject: str
    paths: tuple[str, ...]


def _run_git(args: Sequence[str], repo_root: str | os.PathLike[str] | None = None) -> str:
    command = ["git"]
    if repo_root:
        command.extend(["-C", os.fspath(repo_root)])
    command.extend(args)
    return subprocess.check_output(command, text=True)


def git_commits(
    previous_ref: str,
    current_ref: str,
    repo_root: str | os.PathLike[str] | None = None,
) -> list[Commit]:
    """Return non-merge commits in chronological order for a release range."""

    if not current_ref:
        raise ValueError("current ref is required")
    rev_range = f"{previous_ref}..{current_ref}" if previous_ref else current_ref
    output = _run_git(
        [
            "log",
            "--pretty=format:%H%x09%s",
            "--reverse",
            "--no-merges",
            "--name-only",
            rev_range,
        ],
        repo_root,
    )
    commits: list[Commit] = []
    current_sha = ""
    current_subject = ""
    current_paths: list[str] = []

    def flush() -> None:
        if current_sha:
            commits.append(Commit(current_sha, current_subject, tuple(current_paths)))

    for line in output.splitlines():
        match = COMMIT_HEADER.match(line)
        if match:
            flush()
            current_sha = match.group("sha")
            current_subject = match.group("subject").strip()
            current_paths = []
        elif current_sha and line.strip():
            current_paths.append(line.strip())
    flush()
    return commits


def git_log(
    previous_ref: str,
    current_ref: str,
    repo_root: str | os.PathLike[str] | None = None,
) -> list[str]:
    """Backward-compatible subject-only view of :func:`git_commits`."""

    return [commit.subject for commit in git_commits(previous_ref, current_ref, repo_root)]


def _subject_type(subject: str) -> tuple[str, str]:
    match = CONVENTIONAL_PREFIX.match(subject)
    if not match:
        return "", subject.strip()
    return match.group("type").lower(), match.group("rest").strip()


def deduplicate_preserve_order(items: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            ordered.append(item)
    return ordered


def _clean_subject(subject: str) -> str:
    _commit_type, rest = _subject_type(subject)
    text = rest.strip().rstrip(".")
    if text:
        text = text[0].upper() + text[1:]
    return text


def _commit_refs(commit: Commit) -> list[str]:
    prs = PR_PATTERN.findall(commit.subject)
    refs = [f"#{number}" for number in deduplicate_preserve_order(prs)]
    refs.append(commit.sha[:8])
    return refs


def categorize(subject: str) -> tuple[str, str, list[str]]:
    """Return the legacy ``(category, text, refs)`` tuple.

    This helper is kept for callers that imported the original renderer.  The
    release renderer itself uses commit records so it can include short SHA
    references and compatibility evidence.
    """

    merge_match = MERGE_PR_PATTERN.match(subject)
    if merge_match:
        return "New and improved", f"Merged pull request #{merge_match.group(1)}", [f"#{merge_match.group(1)}"]

    commit_type, rest = _subject_type(subject)
    text = rest if commit_type else subject
    if commit_type in FIX_TYPES or FIX_KEYWORDS.search(subject):
        category = "Fixes"
    elif commit_type in {"feat", "feature"}:
        category = "New and improved"
    elif commit_type in NEW_TYPES:
        category = "New and improved"
    elif COMPAT_KEYWORDS.search(subject):
        category = "Compatibility"
    else:
        category = "New and improved"
    return category, text, [f"#{n}" for n in deduplicate_preserve_order(PR_PATTERN.findall(subject))]


def format_bullet(text: str, refs: Iterable[str] = ()) -> str:
    """Format one concrete Markdown bullet with optional PR/SHA references."""

    cleaned = text.strip().rstrip(".")
    if cleaned:
        cleaned = cleaned[0].upper() + cleaned[1:]
    normalised_refs: list[str] = []
    for ref in refs:
        value = str(ref)
        if value.isdigit():
            value = f"#{value}"
        normalised_refs.append(value)
    refs = list(deduplicate_preserve_order(normalised_refs))
    suffix = f" ({', '.join(refs)})" if refs else ""
    return f"- {cleaned}.{suffix}"


def _section_map(text: str) -> tuple[str, dict[str, list[str]], list[str]]:
    lines = text.replace("\r\n", "\n").replace("\r", "\n").splitlines()
    headings: list[tuple[str, int]] = []
    for index, line in enumerate(lines):
        match = HEADING.match(line)
        if match:
            headings.append((match.group("name"), index))
    sections: dict[str, list[str]] = {}
    for index, (name, start) in enumerate(headings):
        end = headings[index + 1][1] if index + 1 < len(headings) else len(lines)
        sections[name] = [line.rstrip() for line in lines[start + 1 : end] if line.strip()]
    title = next((line.strip() for line in lines if line.strip().startswith("## ")), "")
    return title, sections, lines


def _normalise_body(lines: Iterable[str]) -> str:
    return "\n".join(line.strip() for line in lines if line.strip()).casefold()


def _is_none_known_issue(bullet: str) -> bool:
    value = re.sub(r"[.。]+$", "", bullet.removeprefix("-").strip()).casefold()
    return value in {"none", "none currently known", "no known issues", "none known"}


def _bullet_is_placeholder(bullet: str) -> bool:
    value = re.sub(r"^[-*+]\s*", "", bullet).strip().casefold()
    value = re.sub(r"[.。]+$", "", value)
    if value in GENERIC_BULLETS or GENERIC_PATTERN.fullmatch(value):
        return True
    return any(marker in value for marker in PLACEHOLDER_MARKERS)


def validate_notes(
    text: str,
    version: str,
    *,
    expected_range: str | None = None,
    previous_text: str | None = None,
) -> list[str]:
    """Return human-readable validation errors for a release body."""

    version = version.removeprefix("v")
    title, sections, _lines = _section_map(text)
    errors: list[str] = []
    expected_title = f"## Sbobino {version}"
    if title != expected_title:
        errors.append(f"title must be exactly '{expected_title}'")

    unknown = sorted(set(sections) - set(REQUIRED_SECTIONS) - set(OPTIONAL_SECTIONS))
    if unknown:
        errors.append("unknown release-note section(s): " + ", ".join(unknown))

    for section in REQUIRED_SECTIONS:
        if section not in sections:
            errors.append(f"missing required section: {section}")
            continue
        bullets = [line for line in sections[section] if re.match(r"^[-*+]\s+", line)]
        if not bullets:
            errors.append(f"section '{section}' must contain at least one bullet")
            continue
        if any(_bullet_is_placeholder(bullet) for bullet in bullets):
            errors.append(f"section '{section}' contains generic or placeholder text")

    if "Known issues" in sections:
        bullets = [line for line in sections["Known issues"] if re.match(r"^[-*+]\s+", line)]
        if not bullets:
            errors.append("section 'Known issues' must contain a bullet when present")
        elif any(
            _bullet_is_placeholder(bullet) and not _is_none_known_issue(bullet)
            for bullet in bullets
        ):
            errors.append("section 'Known issues' contains generic or placeholder text")

    if "Refs" in sections:
        refs_body = _normalise_body(sections["Refs"])
        if not (VERSION_RANGE_PATTERN.search(refs_body) or COMMIT_REF_PATTERN.search(refs_body) or "#" in refs_body):
            errors.append("section 'Refs' must name a version range, commit, PR, or issue")
        if expected_range and expected_range.casefold() not in refs_body:
            errors.append(f"section 'Refs' must include release range {expected_range}")

    non_empty_bodies = {
        section: _normalise_body(lines)
        for section, lines in sections.items()
        if section in REQUIRED_SECTIONS and lines
    }
    if len(non_empty_bodies) != len(set(non_empty_bodies.values())):
        errors.append("release-note sections must not be identical")

    if previous_text is not None:
        _old_title, _old_sections, old_lines = _section_map(previous_text)
        _new_title, _new_sections, new_lines = _section_map(text)
        # Ignore only the title version while comparing bodies.  This catches
        # accidentally copied release notes without rejecting a legitimate
        # version title change.
        old_body = _normalise_body(old_lines[1:]) if old_lines else ""
        new_body = _normalise_body(new_lines[1:]) if new_lines else ""
        if old_body and old_body == new_body:
            errors.append("release notes are identical to the previous release")

    return errors


def assert_valid_notes(
    text: str,
    version: str,
    *,
    expected_range: str | None = None,
    previous_text: str | None = None,
) -> None:
    errors = validate_notes(
        text,
        version,
        expected_range=expected_range,
        previous_text=previous_text,
    )
    if errors:
        raise ValueError("Invalid release notes:\n- " + "\n- ".join(errors))


def _compatibility_bullets(commits: Sequence[Commit]) -> list[str]:
    bullets: list[str] = []
    for commit in commits:
        evidence = " ".join((commit.subject, *commit.paths))
        if COMPAT_KEYWORDS.search(evidence):
            bullets.append(
                format_bullet(
                    f"Compatibility evidence: {_clean_subject(commit.subject)}",
                    [f"commit:{commit.sha[:8]}", *_commit_refs(commit)[:-1]],
                )
            )
    return deduplicate_preserve_order(bullets)


def render_from_commits(
    version: str,
    previous_ref: str,
    current_ref: str,
    commits: Sequence[Commit],
) -> str:
    """Render a concrete body from a release range.

    A range with no commits is rejected: an empty generated body is more
    dangerous than a failed candidate build.  Canonical release bodies should
    generally use ``--notes-file`` so product-facing language remains reviewed
    and specific.
    """

    if not commits:
        raise ValueError(f"no commits found in release range {previous_ref}..{current_ref}")

    buckets: dict[str, list[str]] = {section: [] for section in REQUIRED_SECTIONS}
    for commit in commits:
        category, text, _legacy_refs = categorize(commit.subject)
        refs = [f"commit:{commit.sha[:8]}", *_commit_refs(commit)[:-1]]
        buckets[category].append(format_bullet(_clean_subject(text), refs))

    # Compatibility is evidence-derived; a workflow/runtime/architecture
    # change is more useful here than a generic platform promise.
    buckets["Compatibility"] = _compatibility_bullets(commits)
    if not buckets["Compatibility"]:
        # A concrete commit is still better than an invented platform claim.
        first = commits[0]
        buckets["Compatibility"] = [
            format_bullet(
                f"Release compatibility follows the changes in {_clean_subject(first.subject)}",
                [f"commit:{first.sha[:8]}"],
            )
        ]
    if not buckets["Fixes"]:
        first = commits[0]
        buckets["Fixes"] = [
            format_bullet(
                f"Stabilized the release path covered by {_clean_subject(first.subject)}",
                [f"commit:{first.sha[:8]}"],
            )
        ]
    if not buckets["New and improved"]:
        last = commits[-1]
        buckets["New and improved"] = [
            format_bullet(_clean_subject(last.subject), [f"commit:{last.sha[:8]}"])
        ]

    lines = [f"## Sbobino {version}", ""]
    for section in ("Fixes", "New and improved", "Compatibility"):
        lines.extend([f"### {section}", "", *buckets[section], ""])
    lines.extend(
        [
            "### Refs",
            "",
            f"- Release range: `{previous_ref}..{current_ref}`.",
            "- Commits: "
            + ", ".join(f"`{commit.sha[:8]}`" for commit in commits)
            + ".",
        ]
    )
    return "\n".join(lines).rstrip() + "\n"


def render(version: str, buckets: Mapping[str, list[str]]) -> str:
    """Render a bucket mapping while retaining the old public helper."""

    lines = [f"## Sbobino {version}", ""]
    ordered_sections = list(SECTION_ORDER)
    ordered_sections.extend(section for section in buckets if section not in ordered_sections)
    for section in ordered_sections:
        bullets = buckets.get(section, [])
        if not bullets:
            continue
        lines.extend([f"### {section}", "", *bullets, ""])
    return "\n".join(lines).rstrip() + "\n"


def _infer_previous_ref(current_ref: str, repo_root: str | os.PathLike[str] | None) -> str:
    try:
        return _run_git(
            ["describe", "--tags", "--abbrev=0", f"{current_ref}^"], repo_root
        ).strip()
    except (subprocess.CalledProcessError, OSError):
        return ""


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8").replace("\r\n", "\n").replace("\r", "\n")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="Release version without leading v")
    parser.add_argument("previous_ref", nargs="?", help="Git ref of the previous release tag")
    parser.add_argument("current_ref", nargs="?", help="Git ref of the current release tag or HEAD")
    parser.add_argument("--out", type=Path, default=None, help="Output file (default: stdout)")
    parser.add_argument(
        "--notes-file",
        "--source-notes",
        type=Path,
        default=None,
        help="Reviewed versioned notes file to validate and copy",
    )
    parser.add_argument(
        "--previous-notes",
        type=Path,
        default=None,
        help="Previous release notes used to reject accidental duplication",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=None,
        help="Git repository root (defaults to the current working directory)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Validate supplied notes without rendering commit history",
    )
    args = parser.parse_args(argv)
    version = args.version.removeprefix("v")

    previous_ref = args.previous_ref or ""
    current_ref = args.current_ref or ""
    expected_range = None
    if previous_ref and current_ref:
        expected_range = f"{previous_ref}..{current_ref}"
    elif previous_ref and not current_ref:
        expected_range = f"{previous_ref}..v{version}"

    previous_text = None
    if args.previous_notes:
        previous_text = _read_text(args.previous_notes)

    if args.notes_file:
        source = _read_text(args.notes_file)
        try:
            assert_valid_notes(
                source,
                version,
                expected_range=expected_range,
                previous_text=previous_text,
            )
        except ValueError as error:
            print(str(error), file=sys.stderr)
            return 2
        rendered = source.rstrip() + "\n"
    else:
        if args.check:
            print("--check requires --notes-file", file=sys.stderr)
            return 2
        if not current_ref:
            current_ref = "HEAD"
        if not previous_ref:
            previous_ref = _infer_previous_ref(current_ref, args.repo_root)
        if not previous_ref:
            print("unable to infer previous release ref; pass it explicitly", file=sys.stderr)
            return 2
        try:
            commits = git_commits(previous_ref, current_ref, args.repo_root)
            rendered = render_from_commits(version, previous_ref, current_ref, commits)
            assert_valid_notes(
                rendered,
                version,
                expected_range=f"{previous_ref}..{current_ref}",
                previous_text=previous_text,
            )
        except (ValueError, subprocess.CalledProcessError, OSError) as error:
            print(str(error), file=sys.stderr)
            return 2

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered, encoding="utf-8")
    elif not args.check:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
