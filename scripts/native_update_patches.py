#!/usr/bin/env python3
"""Make and list the patch updates in the signed native update manifest.

`generate` runs `caper-updater delta` from each retained earlier build of one
platform's archive to this release's, keeping patches that save enough to be
worth it. `add` writes a platform's full archive and those patches into
`latest.json`, keeping it within the signed manifest's size limit. `stale`
names the retained archives that fall out of the retained window.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

from native_changelog import encode, fit

# Patches from the newest earlier builds kept on the native-update-bases release.
RETAINED_BASES = 6
# A patch must be at most this share of the full archive to be listed.
MAX_PATCH_SHARE = 0.5
BASE_NAME = re.compile(r"(\d+)-(.+)")


def stem(asset):
    for suffix in (".tar.gz", ".zip"):
        if asset.endswith(suffix):
            return asset[: -len(suffix)]
    raise ValueError(f"unexpected archive {asset}")


def patch_name(asset, from_build, to_build):
    return f"{stem(asset)}-{from_build}-to-{to_build}.caperpatch"


def retained(bases, asset, build, keep=RETAINED_BASES):
    """The newest `keep` earlier builds of `asset` in `bases` (`<build>-<asset>`), newest first."""
    found = {}
    for path in bases.iterdir() if bases.is_dir() else []:
        match = BASE_NAME.fullmatch(path.name)
        if match and match[2] == asset and int(match[1]) < build:
            found[int(match[1])] = path
    return [(number, found[number]) for number in sorted(found, reverse=True)[:keep]]


def generate(platform, archive, build, bases, updater, output, keep=RETAINED_BASES):
    output.mkdir(parents=True, exist_ok=True)
    asset = archive.name
    limit = archive.stat().st_size * MAX_PATCH_SHARE
    patches = []
    for from_build, base in retained(bases, asset, build, keep):
        target = output / patch_name(asset, from_build, build)
        target.unlink(missing_ok=True)
        # The updater proves each patch by rebuilding this release from the base.
        result = subprocess.run(
            [
                str(updater),
                "delta",
                "--platform",
                platform,
                "--base",
                str(base),
                "--from-build",
                str(from_build),
                "--target",
                str(archive),
                "--to-build",
                str(build),
                "--output",
                str(target),
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"::warning::No patch from build {from_build}: {result.stderr.strip()}")
            target.unlink(missing_ok=True)
            continue
        summary = json.loads(result.stdout)
        if summary["size"] > limit:
            print(f"Skipped the patch from build {from_build}: {summary['size']} bytes is not small enough.")
            target.unlink()
            continue
        print(f"Patch from build {from_build}: {summary['size']} of {archive.stat().st_size} bytes.")
        patches.append(
            {"from_build": from_build, "file": target.name, "size": summary["size"], "sha256": summary["sha256"]}
        )
    (output / f"patches-{platform}.json").write_text(json.dumps(patches, indent=2) + "\n")
    return patches


def stale(names, keep=RETAINED_BASES):
    """Retained archives (`<build>-<asset>`) older than each asset's newest `keep` builds."""
    builds = {}
    for name in names:
        match = BASE_NAME.fullmatch(name)
        if match:
            builds.setdefault(match[2], set()).add(int(match[1]))
    kept = {asset: sorted(numbers, reverse=True)[:keep] for asset, numbers in builds.items()}
    return [name for name in names if (match := BASE_NAME.fullmatch(name)) and int(match[1]) not in kept[match[2]]]


def sha256(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def add(manifest, platform, archive, base_url, patches):
    entry = {"url": f"{base_url}/{archive.name}", "sha256": sha256(archive), "size": archive.stat().st_size}
    if patches:
        entry["patches"] = [
            {
                "from_build": patch["from_build"],
                "url": f"{base_url}/{patch['file']}",
                "sha256": patch["sha256"],
                "size": patch["size"],
            }
            for patch in sorted(patches, key=lambda patch: patch["from_build"], reverse=True)
        ]
    manifest["platforms"][platform] = entry
    # native_changelog.py fits a manifest that has no history yet.
    return fit(manifest) if "changelog" in manifest else manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    make = commands.add_parser("generate")
    make.add_argument("--platform", required=True)
    make.add_argument("--archive", type=Path, required=True)
    make.add_argument("--build", type=int, required=True)
    make.add_argument("--bases", type=Path, required=True)
    make.add_argument("--updater", type=Path, required=True)
    make.add_argument("--output", type=Path, required=True)
    write = commands.add_parser("add")
    write.add_argument("--manifest", type=Path, required=True)
    write.add_argument("--platform", required=True)
    write.add_argument("--archive", type=Path, required=True)
    write.add_argument("--base-url", required=True)
    write.add_argument("--patches", type=Path)
    commands.add_parser("stale", help="read retained asset names on stdin, print the ones to delete")
    args = parser.parse_args()
    if args.command == "stale":
        print("\n".join(stale(sys.stdin.read().split())))
    elif args.command == "generate":
        generate(args.platform, args.archive, args.build, args.bases, args.updater, args.output)
    else:
        manifest = json.loads(args.manifest.read_bytes())
        patches = json.loads(args.patches.read_text()) if args.patches and args.patches.is_file() else []
        args.manifest.write_bytes(encode(add(manifest, args.platform, args.archive, args.base_url, patches)))


if __name__ == "__main__":
    main()
