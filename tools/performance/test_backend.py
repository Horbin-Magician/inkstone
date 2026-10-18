import unittest
from backend import metrics, summarize


class MetricsTest(unittest.TestCase):
    def runs(self):
        return [{'count': count, 'metrics': {'index_build_ms': value},
                 'process_peak_rss_bytes': value * 100, 'iteration': value}
                for count in (1000, 10000) for value in (3, 1, 2)]

    def test_summary_keeps_medians_for_both_corpus_sizes(self):
        self.assertEqual(summarize(self.runs()), {
            count: {'index_build_ms': 2, 'process_peak_rss_bytes': 200}
            for count in (1000, 10000)
        })

    def test_summary_rejects_missing_and_unexpected_corpus_sizes(self):
        for runs in ([], self.runs()[:3], self.runs()[3:],
                     self.runs() + [{'count': 100, 'iteration': 1}]):
            with self.subTest(counts=[run['count'] for run in runs]):
                with self.assertRaises(ValueError):
                    summarize(runs)

    def test_summary_requires_three_distinct_rounds_per_size(self):
        for count in (1000, 10000):
            for mode in ('missing', 'extra', 'duplicate', 'unnumbered'):
                with self.subTest(count=count, mode=mode):
                    runs = self.runs()
                    run = next(run for run in runs if run['count'] == count)
                    if mode == 'missing':
                        runs.remove(run)
                    elif mode == 'extra':
                        runs.append(dict(run))
                    elif mode == 'duplicate':
                        run['iteration'] = 1
                    else:
                        del run['iteration']
                    with self.assertRaises(ValueError):
                        summarize(runs)

    def test_summary_rejects_empty_or_inconsistent_metrics(self):
        for count in (1000, 10000):
            for mode in ('empty', 'missing', 'extra'):
                with self.subTest(count=count, mode=mode):
                    runs = self.runs()
                    group = [run for run in runs if run['count'] == count]
                    if mode == 'empty':
                        for run in group:
                            run['metrics'].clear()
                    elif mode == 'missing':
                        group[1]['metrics'].clear()
                    else:
                        group[1]['metrics']['search_ms'] = 1
                    with self.assertRaises(ValueError):
                        summarize(runs)

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
