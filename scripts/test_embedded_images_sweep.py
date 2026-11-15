"""The image sweep must reject incomplete exports and preserve PDF content."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import subprocess

SPEC = importlib.util.spec_from_file_location('embedded_images', Path(__file__).with_name('embedded-images-sweep.py'))
SWEEP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SWEEP)


class ExportValidationTests(unittest.TestCase):
    def capture(self, directory, fmt):
        return SWEEP.capture(Path('/unused/d2'), directory, 'a', 'dagre', fmt, False, 1)

    def test_success_without_output_is_failure(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(SWEEP.subprocess, 'run') as run:
            run.return_value = subprocess.CompletedProcess([], 0, b'', b'')
            result = self.capture(Path(temp) / 'run', 'png')
            self.assertEqual(result['invalid'], ['missing output inventory'])

    def test_truncated_pixels_do_not_pass_container_validation(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp) / 'run'
            def run(*args, **kwargs):
                # Header and trailer pass the lightweight format check, but no frame exists.
                (directory / 'output.gif').write_bytes(b'GIF89a' + b'\0' * 20 + b';')
                return subprocess.CompletedProcess([], 0, b'', b'')
            with patch.object(SWEEP.subprocess, 'run', side_effect=run):
                result = self.capture(directory, 'gif')
            self.assertTrue(result['invalid'])

    def test_pdf_only_dates_are_normalized(self):
        data = b'/CreationDate (D:20261115090000Z) /ModDate (D:20261115090000Z) /Width 19 stream 12345'
        changed_date = data.replace(b'20261115090000', b'20261116090101')
        self.assertEqual(SWEEP.normalized(data, '.pdf'), SWEEP.normalized(changed_date, '.pdf'))
        changed_image = data.replace(b'/Width 19', b'/Width 20')
        self.assertNotEqual(SWEEP.normalized(data, '.pdf'), SWEEP.normalized(changed_image, '.pdf'))
        self.assertEqual(SWEEP.normalized(data, '.svg'), data)

    def test_timeout_is_not_a_successful_empty_inventory(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(SWEEP.subprocess, 'run', side_effect=subprocess.TimeoutExpired('d2', 1)):
            result = self.capture(Path(temp) / 'run', 'png')
            self.assertEqual(result['exit'], 'timeout')


if __name__ == '__main__':
    unittest.main()
