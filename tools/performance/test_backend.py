import unittest
from backend import metrics


class MetricsTest(unittest.TestCase):
    def test_named_and_repeated_stage_metrics_do_not_collide(self):
        self.assertEqual(metrics('index_build_ms=12\nclone rounds=7 p50_ms=0.125 max_ms=1\nsearch rounds=7 p50_ms=2 max_ms=4\n'), {
            'index_build_ms': 12.0, 'clone.p50_ms': 0.125, 'clone.max_ms': 1.0,
            'search.p50_ms': 2.0, 'search.max_ms': 4.0,
        })

    def test_does_not_interpret_fixture_path_as_a_metric(self):
        self.assertEqual(metrics('profile=release root=/tmp/hello notes=1000\n\n'), {})

    def test_rejects_invalid_timings(self):
        for value in ('nan', 'NaN', 'inf', '-inf', '-0.001'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                metrics(f'index_build_ms={value}\n')

    def test_rejects_duplicate_metrics_instead_of_overwriting(self):
        for output in (
            'index_build_ms=1 index_build_ms=2\n',
            'index_build_ms=1\nindex_build_ms=2\n',
            'clone rounds=7 p50_ms=1\nclone rounds=7 p50_ms=2\n',
        ):
            with self.subTest(output=output), self.assertRaises(ValueError):
                metrics(output)
