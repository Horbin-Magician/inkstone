"""Validate an already committed version, tag it and atomically push the release."""
import argparse
import re
import subprocess

from package import ROOT, metadata


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def release(version, dry_run=False):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("Expected a stable version, e.g. 0.1.0")
    if git("status", "--porcelain"):
        raise ValueError("Commit changes before releasing")
    if metadata()["version"] != version:
        raise ValueError("Version must match Cargo workspace and lockfile")
    if not (ROOT / f"docs/releases/{version}.md").read_text(encoding="utf-8").strip():
        raise ValueError("Release notes required")
    branch = git("symbolic-ref", "--short", "HEAD")
    tag = f"v{version}"
    if git("tag", "--list", tag) or git("ls-remote", "--tags", "origin", f"refs/tags/{tag}"):
        raise ValueError("Release tag already exists")
    push = ("push", "--atomic", "origin", f"HEAD:refs/heads/{branch}", f"refs/tags/{tag}")
    if dry_run:
        print(f"Would tag HEAD as {tag} and run: git {' '.join(push)}")
        return
    git("tag", "-a", tag, "-m", f"Release {version}")
    git(*push)
    print(f"Pushed {tag}; follow the Publish release workflow on GitHub.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--dry-run", action="store_true")
    release(**vars(parser.parse_args()))
