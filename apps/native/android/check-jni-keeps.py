#!/usr/bin/env python3
"""Fail when R8 renamed or removed a Java class that bundled native code finds by name.

JNI code looks classes up with strings such as "org/jni_zero/JniZero". R8 only
learns about those lookups from keep rules, so a library that ships without
consumer rules (webrtc-sdk's org.jni_zero) still builds, then aborts the app
with "java_class == null" when its library loads.

usage: check-jni-keeps.py MAPPING USAGE ARCHIVE...
MAPPING and USAGE are R8's mapping.txt and usage.txt; each ARCHIVE is an APK or
AAB whose lib/<abi>/*.so files are scanned.
"""
import re
import sys
import zipfile

# At least two lowercase package segments, then a capitalized class name;
# signatures add a leading "L". Names R8 never saw (java.*, android.*, stray
# bytes) are ignored below, so only real program classes are judged.
CLASS_NAME = re.compile(rb"(?<![A-Za-z0-9_/.$])L?((?:[a-z][a-z0-9_]*[/.]){2,}[A-Z][A-Za-z0-9_$]*)")
MAPPED = re.compile(r"^(\S+) -> (\S+):$")


def referenced_classes(archives):
    names, libraries = set(), 0
    for path in archives:
        with zipfile.ZipFile(path) as archive:
            for entry in archive.namelist():
                if not re.search(r"(^|/)lib/[^/]+/[^/]+\.so$", entry):
                    continue
                libraries += 1
                for match in CLASS_NAME.finditer(archive.read(entry)):
                    names.add(match.group(1).decode().replace("/", "."))
    return names, libraries


def main(mapping_path, usage_path, *archives):
    if not archives:
        sys.exit(__doc__)
    renamed = {}
    with open(mapping_path, encoding="utf-8") as mapping:
        for line in mapping:
            if match := MAPPED.match(line.rstrip("\n")):
                renamed[match.group(1)] = match.group(2)
    with open(usage_path, encoding="utf-8") as usage:
        # usage.txt lists a wholly removed class on an unindented line without a colon.
        removed = {line.strip() for line in usage if line[:1].strip() and not line.rstrip().endswith(":")}

    names, libraries = referenced_classes(archives)
    if not libraries:
        sys.exit("No native libraries found in " + ", ".join(archives))
    problems, checked = [], 0
    for name in sorted(names):
        if name in removed:
            problems.append(f"{name} was removed")
        elif name in renamed:
            checked += 1
            if renamed[name] != name:
                problems.append(f"{name} was renamed to {renamed[name]}")
    if problems:
        print("Native code looks these classes up by name; add keep rules to app/proguard-rules.pro:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        sys.exit(1)
    if not checked:
        sys.exit("No class named in native code matched R8's mapping; the check cannot vouch for this build.")
    print(f"JNI class names intact: {checked} referenced from {libraries} native libraries.")


if __name__ == "__main__":
    main(*sys.argv[1:])
