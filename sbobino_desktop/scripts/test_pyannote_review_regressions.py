#!/usr/bin/env python3
"""Native counterexamples from the independent 2.0.38 packaging review."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_setup_pyannote_staging import non_personal_staging_base, run_probe
from test_pyannote_portable_runtime import portable


class RunpathContextTests(unittest.TestCase):
    def test_exact_loader_path_is_a_directory(self):
        binary = Path('/payload/lib/libtest.dylib')
        self.assertEqual(portable._resolve_rpath_entries(binary, ['@loader_path']), [binary.parent])

    def test_executable_path_requires_an_explicit_host_context(self):
        binary = Path('/payload/lib/libtest.dylib')
        self.assertEqual(portable._resolve_rpath_entries(binary, ['@executable_path/../lib']), [])
        self.assertEqual(portable._resolve_rpath_entries(binary, ['@executable_path'], Path('/payload/bin')), [Path('/payload/bin')])


@unittest.skipUnless(sys.platform == 'darwin', 'requires native macOS loader')
class NativeReviewRegressions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=non_personal_staging_base(), prefix='review ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def build(self, output, source, *options):
        output.parent.mkdir(parents=True, exist_ok=True)
        c = output.with_suffix('.c')
        c.write_text(source)
        subprocess.run(['xcrun', 'clang', '-dynamiclib', '-Wl,-headerpad_max_install_names', str(c), '-o', str(output), *map(str, options)], check=True, capture_output=True)
        c.unlink()
        subprocess.run(['codesign', '--force', '--sign', '-', str(output)], check=True, capture_output=True)
        return output

    def test_symlinked_staging_parent_is_rejected_before_mkdir(self):
        protected = self.root / 'Library' / 'Application Support' / 'synthetic-profile'
        protected.mkdir(parents=True)
        alias = self.root / 'alias'
        alias.symlink_to(protected, target_is_directory=True)
        result, _ = run_probe(alias / 'must-not-be-created')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((protected / 'must-not-be-created').exists())

    def test_unreachable_rpath_is_not_resolved_by_basename(self):
        payload = self.root / 'payload'
        orphan = self.build(payload / 'elsewhere' / 'liborphan.dylib', 'int fixture_value(void){return 11;}', '-Wl,-install_name,@rpath/liborphan.dylib')
        self.build(payload / 'libuse.dylib', 'extern int fixture_value(void); int value(void){return fixture_value();}', orphan, '-Wl,-install_name,@rpath/libuse.dylib')
        report = portable.audit_runtime(payload, strict_closure=True)
        self.assertIn('unresolved_rpath_dependency', [item['kind'] for item in report['violations']])

    def test_duplicate_libraries_keep_separate_identities_in_both_load_orders(self):
        payload = self.root / 'payload'
        libraries = []
        for group, value in [('a', 11), ('b', 22)]:
            lib = self.build(payload / group / 'libsame.dylib', f'int fixture_value(void){{return {value};}}', '-Wl,-install_name,@rpath/libsame.dylib')
            self.build(payload / f'consumer-{group}.dylib', 'extern int fixture_value(void); int value(void){return fixture_value();}', lib, f'-Wl,-rpath,@loader_path/{group}', f'-Wl,-install_name,@rpath/consumer-{group}.dylib')
            subprocess.run(['install_name_tool', '-id', f'/DLC/{group}/libsame.dylib', str(lib)], check=True, capture_output=True)
            libraries.append(lib)
        portable.sanitize_runtime(payload)
        self.assertNotEqual(portable.read_install_names(libraries[0]), portable.read_install_names(libraries[1]))
        moved = self.root / 'relocated payload'
        payload.rename(moved)
        code = 'import ctypes,json,sys;print(json.dumps([ctypes.CDLL(p).value() for p in sys.argv[1:]]))'
        for order, expected in [(('a', 'b'), [11, 22]), (('b', 'a'), [22, 11])]:
            paths = [str(moved / f'consumer-{group}.dylib') for group in order]
            result = subprocess.run([sys.executable, '-B', '-c', code, *paths], capture_output=True, text=True, env={k: v for k, v in os.environ.items() if not k.startswith('DYLD_')}, check=True)
            self.assertEqual(json.loads(result.stdout), expected)
        report = portable.audit_runtime(moved, strict_closure=True)
        self.assertEqual(report['violations'], [])


if __name__ == '__main__':
    unittest.main()
