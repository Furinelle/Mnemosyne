#!/usr/bin/env python3
"""Small contracts for the offline native performance harness."""
import importlib.util
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('native_perf', Path(__file__).with_name('native_perf.py'))
perf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf)


class PerfHarnessTest(unittest.TestCase):
    def test_recipe_and_lane(self):
        self.assertEqual(perf.recipe('v2-evolution', 100), {
            'memories': 100, 'support': 20, 'post_revision_support': 1,
            'double_revise': 10, 'supersede': 10, 'checkpoints': 5, 'pending': 5})
        self.assertEqual(perf.lane([{'score_breakdown': {'bm25': 1}}]), 'lexical')
        self.assertEqual(perf.lane([{'score_breakdown': {'bm25': 1,
            'vector_diagnostic': {'status': 'missing_assets'}}}]), 'lexical_fallback')

    def test_subprocess_timeout(self):
        runner = perf.Runner(Path(sys.executable), os.environ.copy(), .001,
                             time.monotonic() + 2)
        with self.assertRaises(perf.Blocked):
            runner.call(['-c', 'import time; time.sleep(.1)'], Path.cwd())

    def test_formal_v2_fixture_and_six_paths(self):
        binary = os.environ.get('MNEMOSYNE_PERF_BINARY')
        if not binary:
            self.skipTest('set MNEMOSYNE_PERF_BINARY to run native smoke')
        with tempfile.TemporaryDirectory(prefix='mnemosyne-perf-test-') as tmp:
            root = Path(tmp)
            env = dict(HOME=str(root / 'home'), MNEMOSYNE_HOME=str(root / 'global'),
                       PATH='', MNEMOSYNE_TRACE_TIMING='1', MNEMOSYNE_AUTO_INIT='0')
            runner = perf.Runner(Path(binary).resolve(), env, 120, time.monotonic() + 300)
            project = root / 'project'
            recipe = perf.build(runner, project, 'v2-evolution', 10, 100 * 1024 * 1024)
            counts = perf.verify(project, 'v2-evolution', recipe)
            self.assertEqual(counts['memories'], 10)
            self.assertEqual(counts['sources'], 13)
            self.assertEqual(counts['superseded'], 1)
            self.assertEqual(counts['pending'], 1)
            self.assertEqual(len(perf.corpus_hash(project)), 64)
            for scenario in perf.SCENARIOS:
                sample = root / f'sample-{scenario}'
                import shutil
                shutil.copytree(project, sample)
                observation = perf.measure(runner, sample, 'v2-evolution', scenario, 0)
                self.assertGreater(observation['elapsed_ms'], 0, scenario)
                self.assertNotEqual(observation['actual_lane'], 'unknown', scenario)


if __name__ == '__main__':
    unittest.main()
