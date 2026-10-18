"""Preserve benchmark output before checking the process exit status."""
import subprocess


def run_logged(command, stdout_path, stderr_path=None):
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
    if stderr_path is None:
        stdout_path.write_text(result.stdout + result.stderr, encoding="utf-8")
    else:
        stdout_path.write_text(result.stdout, encoding="utf-8")
        stderr_path.write_text(result.stderr, encoding="utf-8")
    result.check_returncode()
    return result
