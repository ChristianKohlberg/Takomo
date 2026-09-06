"""Exercise the CI classifier against real commit ranges, including fail-closed history."""
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name('ci-full-required.sh').resolve()


class ScopeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = pathlib.Path(self.tmp.name)
        self.git('init', '-q')
        self.git('config', 'user.email', 'ci@example.invalid')
        self.git('config', 'user.name', 'CI fixture')
        self.git('config', 'commit.gpgsign', 'false')
        self.write('Dockerfile', 'FROM scratch\n')
        self.write('web/src/view.tsx', 'original\n')
        self.commit()
        self.base = self.git('rev-parse', 'HEAD').strip()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.repo, text=True)

    def write(self, name, text):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')

    def classify(self, base=None):
        return subprocess.check_output(
            ['bash', str(SCRIPT), self.base if base is None else base],
            cwd=self.repo, text=True).strip()

    def test_ordinary_ui_and_unusual_filename_use_fast_lane(self):
        self.write('web/src/view.tsx', 'changed\n')
        self.write('docs/a file\nwith newline.md', 'notes\n')
        self.commit()
        self.assertEqual(self.classify(), 'false')

    def test_packaging_addition_uses_full_lane(self):
        self.write('web/package-lock.json', '{}\n')
        self.commit()
        self.assertEqual(self.classify(), 'true')

    def test_packaging_deletion_uses_full_lane(self):
        (self.repo / 'Dockerfile').unlink()
        self.commit()
        self.assertEqual(self.classify(), 'true')

    def test_rename_out_of_packaging_still_uses_full_lane(self):
        self.git('mv', 'Dockerfile', 'notes.txt')
        self.commit()
        self.assertEqual(self.classify(), 'true')

    def test_no_change_uses_fast_lane(self):
        self.assertEqual(self.classify(), 'false')

    def test_missing_history_fails_closed(self):
        for base in ('', '0' * 40, 'not-a-commit'):
            with self.subTest(base=base):
                self.assertEqual(self.classify(base), 'true')


if __name__ == '__main__':
    unittest.main()
