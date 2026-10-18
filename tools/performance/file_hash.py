"""Bounded-memory hashing for benchmark inputs and artifacts."""
import hashlib


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_unchanged(path, expected):
    """Reject measurements that span different versions of an input artifact."""
    if sha256(path) != expected:
        raise ValueError(f"benchmark input changed during measurement: {path.name}")
