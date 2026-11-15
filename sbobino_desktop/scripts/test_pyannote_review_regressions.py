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
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_setup_pyannote_staging import non_personal_staging_base, run_probe
from test_pyannote_portable_runtime import portable


def shell_function(path, name):
    source = path.read_text()
    start = source.index(name + '() {')
    return source[start:source.index('\n}\n', start) + 3]


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
        subprocess.run(['xcrun', 'clang', '-arch', os.uname().machine, '-dynamiclib', '-Wl,-headerpad_max_install_names', str(c), '-o', str(output), *map(str, options)], check=True, capture_output=True)
        c.unlink()
        subprocess.run(['codesign', '--force', '--sign', '-', str(output)], check=True, capture_output=True)
        return output

    def test_host_basename_collision_is_refused_before_mutating_the_payload(self):
        payload = self.root / 'payload'
        roots = []
        for group, value in [('a', 11), ('b', 22)]:
            host = self.root / ('host-' + group)
            roots.append(str(host))
            library = host / 'libsame.dylib'
            self.build(library, f'int fixture_value(void){{return {value};}}',
                       '-Wl,-install_name,' + str(library))
            self.build(payload / ('consumer-' + group + '.dylib'),
                       'extern int fixture_value(void); int value(void){return fixture_value();}',
                       library, '-Wl,-install_name,@rpath/consumer-' + group + '.dylib')
        before = {str(p.relative_to(payload)): p.read_bytes()
                  for p in payload.rglob('*') if p.is_file()}
        with mock.patch.object(portable, 'HOST_PREFIXES', portable.HOST_PREFIXES + tuple(roots)):
            with self.assertRaisesRegex(SystemExit, 'same basename'):
                portable.sanitize_runtime(payload)
        after = {str(p.relative_to(payload)): p.read_bytes()
                 for p in payload.rglob('*') if p.is_file()}
        self.assertEqual(before, after, 'a rejected plan must not rewrite or copy any file')

    def run_runtime_removal(self, target, root):
        script = Path(__file__).parent / 'setup_bundled_pyannote.sh'
        function = shell_function(script, 'replace_runtime_dir')
        return subprocess.run(
            ['bash', '-c', function + '\nreplace_runtime_dir "$1" "$2"',
             'runtime-removal', str(target), str(root)],
            env={**os.environ, 'PYANNOTE_PORTABLE_RUNTIME_LIB': str(script.parent / 'lib/pyannote_portable_runtime.py')},
            capture_output=True, text=True)

    def test_runtime_removal_refuses_an_intermediate_symlink_escape(self):
        root = self.root / 'vetted root'
        outside = self.root / 'outside'
        root.mkdir()
        precious = outside / 'aarch64-apple-darwin'
        precious.mkdir(parents=True)
        sentinel = precious / 'SENTINEL.txt'
        sentinel.write_text('preserve')
        (root / 'python').symlink_to(outside, target_is_directory=True)
        result = self.run_runtime_removal(root / 'python/aarch64-apple-darwin', root)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sentinel.read_text(), 'preserve')

    def test_runtime_removal_accepts_a_fresh_contained_target(self):
        root = self.root / 'new resource root'
        root.mkdir()
        target = root / 'python/aarch64-apple-darwin'
        result = self.run_runtime_removal(target, root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(target.exists())

    def test_local_preflight_refuses_a_new_work_dir_inside_checkout(self):
        script = Path(__file__).parent / 'local_packaging_gate.sh'
        checkout = self.root / 'checkout'
        checkout.mkdir()
        work = checkout / 'not-created-yet'
        functions = shell_function(script, 'record') + shell_function(script, 'preflight')
        result = subprocess.run(
            ['bash', '-c', functions + '\nSTATUS=0; CHECKS=""; WORK_DIR="$1"; REPO_ROOT="$2"; PORTABLE="$3"; preflight',
             'preflight', str(work), str(checkout), str(script.parent / 'lib/pyannote_portable_runtime.py')],
            capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('outside the checkout', result.stdout + result.stderr)
        self.assertFalse(work.exists(), 'rejection must precede mkdir')

    def test_readiness_rejects_relative_tmpdir_before_any_gate(self):
        script = Path(__file__).parent / 'release_readiness.sh'
        temp = self.root / 'relative'
        temp.mkdir()
        result = subprocess.run(
            ['bash', str(script), '2.0.38'], cwd=self.root,
            env={**os.environ, 'TMPDIR': 'relative',
                 'SBOBINO_PYANNOTE_RESOURCE_ROOT': str(self.root / 'unused-payload')},
            capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('TMPDIR must name an absolute existing directory', result.stderr)
        self.assertNotIn('Version coherence verified', result.stdout)
        self.assertEqual(list(temp.iterdir()), [])

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
            result = subprocess.run(['/usr/bin/arch', '-' + os.uname().machine, sys.executable, '-B', '-c', code, *paths], capture_output=True, text=True, env={k: v for k, v in os.environ.items() if not k.startswith('DYLD_')}, check=True)
            self.assertEqual(json.loads(result.stdout), expected)
        report = portable.audit_runtime(moved, strict_closure=True)
        self.assertEqual(report['violations'], [])


if __name__ == '__main__':
    unittest.main()
