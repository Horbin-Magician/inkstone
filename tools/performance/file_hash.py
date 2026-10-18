"""Bounded-memory hashing for benchmark inputs and artifacts."""
import hashlib


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()
