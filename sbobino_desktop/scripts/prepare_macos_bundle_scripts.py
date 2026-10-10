#!/usr/bin/env python3
"""Seal shell launchers as resources so updater archives retain their integrity."""

import pathlib
import sys


def prepare(app: pathlib.Path) -> None:
    executables = app / "Contents" / "MacOS"
    resources = app / "Contents" / "Resources" / "runtime-launchers"
    scripts = []
    for path in executables.iterdir():
        if path.is_file() and not path.is_symlink():
            with path.open("rb") as handle:
                if handle.read(2) == b"#!":
                    scripts.append(path)
    if not scripts:
        return
    resources.mkdir(parents=True, exist_ok=True)
    for path in scripts:
        target = resources / path.name
        if target.exists() or target.is_symlink():
            raise FileExistsError(target)
    for path in scripts:
        path.rename(resources / path.name)
        path.symlink_to("../Resources/runtime-launchers/" + path.name)


if __name__ == "__main__":
    prepare(pathlib.Path(sys.argv[1]))
