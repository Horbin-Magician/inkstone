#!/usr/bin/env python3
"""Record three isolated release backend runs per fixed generated vault size.

Build `cargo build --release --locked -p inkstone-core --example benchmark` first.
This measures backend stages, not native UI latency. No user vault is accepted.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import statistics
import subprocess
import tempfile


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def metrics(stdout):
    result = {}
    for line in stdout.splitlines():
        tokens = dict(re.findall(r'(\w+)=([^\s]+)', line))
        # Only explicitly named stage timings, never arbitrary source text.
        stage = line.split()[0] if line.split() else ''
        for key, value in tokens.items():
            if key.endswith('_ms'):
                result[key if key != 'p50_ms' and key != 'max_ms' else f'{stage}.{key}'] = float(value)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/examples/benchmark'))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('requires macOS time -l; RSS units are bytes')
    project = Path(__file__).resolve().parents[2]
    binary = args.binary.resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix='backend-v1-', dir=project / 'target'))
    metadata = {
        'schema_version': 1, 'corpus_version': 'backend-v1',
        'scope': 'backend stage timings; process RSS includes fixture creation, all stages and teardown',
        'os_cache_cleared': False, 'binary': str(binary), 'binary_sha256': digest(binary),
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=project, text=True).strip(),
        'rustc': subprocess.check_output(['rustc', '--version'], cwd=project, text=True).strip(),
        'platform': platform.platform(), 'machine': platform.machine(),
        'hardware': subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string', 'hw.memsize'], text=True).splitlines(),
    }
    (root / 'source.patch').write_bytes(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=project))
    (root / 'status.txt').write_bytes(subprocess.check_output(['git', 'status', '--short'], cwd=project))
    (root / 'manifest.json').write_text(json.dumps(metadata, indent=2))
    runs = []
    for count in (1000, 10000):
        expected = None
        for iteration in range(3):
            label = f'{count}-{iteration + 1}'
            command = ['/usr/bin/time', '-l', str(binary), str(count), '1', str(root / f'{label}-fixture')]
            completed = subprocess.run(command, capture_output=True, text=True)
            (root / f'{label}.stdout').write_text(completed.stdout)
            (root / f'{label}.stderr').write_text(completed.stderr)
            completed.check_returncode()
            if 'profile=release backend_only=true' not in completed.stdout:
                raise ValueError('runner requires release backend benchmark')
            fixture_root = root / f'{label}-fixture'
            files = [{
                'path': str(path.relative_to(fixture_root / 'vault')),
                'bytes': path.stat().st_size, 'sha256': digest(path),
            } for path in sorted((fixture_root / 'vault').rglob('*.md'))]
            if len(files) != count:
                raise ValueError('fixture count changed')
            if expected is not None and files != expected:
                raise ValueError('fixture differs between rounds')
            expected = files
            (root / f'{label}.corpus.json').write_text(json.dumps(files, indent=2))
            runs.append({
                'count': count, 'iteration': iteration + 1, 'command': command,
                'fixture_root': str(fixture_root), 'metrics': metrics(completed.stdout),
                'process_peak_rss_bytes': int(re.search(r'(\d+)\s+maximum resident set size', completed.stderr)[1]),
            })
            (root / 'runs.json').write_text(json.dumps(runs, indent=2))
    if digest(binary) != metadata['binary_sha256']:
        raise ValueError('binary changed during measurement')
    summary = {}
    for count in (1000, 10000):
        group = [run for run in runs if run['count'] == count]
        summary[count] = {key: statistics.median(run['metrics'][key] for run in group)
                          for key in group[0]['metrics']}
        summary[count]['process_peak_rss_bytes'] = statistics.median(run['process_peak_rss_bytes'] for run in group)
    (root / 'results.json').write_text(json.dumps({'schema_version': 1, 'runs': runs, 'medians': summary}, indent=2))
    print(root)
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
