"""Regression checks for the preparation CLI's private JSON/file transport."""
from contextlib import redirect_stdout
from decimal import Decimal
import importlib.util
import io
import json
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name('prepare-legacy-subscription-originals.py')
SPEC = importlib.util.spec_from_file_location('prepare_originals_cli', SCRIPT)
CLI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLI)


class PreparationTransportTests(unittest.TestCase):
    def test_large_preparation_preserves_decimal_and_existing_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = {}
            for name in ('financial-material', 'source', 'review'):
                path = root / (name + '.json')
                path.write_text('{"amount":1234567890.123456789}')
                inputs[name] = path
            output = root / 'prepared'
            material = {'records': [], 'review': {'decision': 'fixture'},
                        'source': {'amount': Decimal('1234567890.123456789')}}
            sql = '-- private transport fixture\n' + ('x' * (17 * 1024 * 1024))
            arguments = [str(SCRIPT), '--migration-id', 'transport-fixture',
                         '--output-dir', str(output)]
            for name, path in inputs.items():
                arguments.extend(['--' + name, str(path)])
            # Writer validation has its own real-PG suite. These substitutions
            # isolate the CLI's decimal reader, capacity and O_EXCL behavior.
            with patch.object(sys, 'argv', arguments), \
                    patch.object(CLI.W, 'prepare_material', return_value=material) as prepare, \
                    patch.object(CLI.W, 'build_apply_sql', return_value=sql), \
                    redirect_stdout(io.StringIO()):
                CLI.main()
                self.assertIsInstance(prepare.call_args.args[1]['amount'], Decimal)
                saved = output / 'material.json'
                before = saved.read_bytes()
                decoded = json.loads(before, parse_float=Decimal)
                self.assertEqual(decoded, material)
                self.assertEqual(stat.S_IMODE(saved.stat().st_mode), 0o600)
                self.assertEqual((output / 'apply.sql').stat().st_size, len(sql.encode()))
                with self.assertRaisesRegex(CLI.W.F.IDENTITY.MappingError, 'original_output_exists'):
                    CLI.main()
                self.assertEqual(saved.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
