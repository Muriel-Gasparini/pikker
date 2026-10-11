"""Exercise the publication validator with independent temporary fixtures."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).with_name("check-publication.py")


class PublicationChecks(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="pikker"\nversion="0.1.0-beta.1"\nlicense="MIT"\n'
            'repository="https://github.com/Muriel-Gasparini/pikker"\n', encoding="utf-8")
        for name in ["LICENSE", "THIRD_PARTY_NOTICES.txt"]:
            (self.root / name).write_text("Distribution license notice\n" * 10, encoding="utf-8")
        (self.root / "README.md").write_text("[License](LICENSE)\n", encoding="utf-8")
        self.stage()

    def stage(self):
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)

    def check(self, tag="v0.1.0-beta.1"):
        return subprocess.run([sys.executable, str(CHECKER), "--root", str(self.root),
                               "--tag", tag], capture_output=True, text=True)

    def test_valid_distribution_passes(self):
        self.assertEqual(self.check().returncode, 0)

    def test_mismatched_version_is_rejected(self):
        self.assertEqual(self.check("v0.2.0").returncode, 1)

    def test_untrusted_tag_is_rejected(self):
        for tag in ["v0.1.0-beta.1;echo BAD", "v../main", "0.1.0-beta.1", "v01.1.0"]:
            self.assertEqual(self.check(tag).returncode, 1)

    def test_broken_local_link_is_rejected(self):
        (self.root / "README.md").write_text("[Missing](docs/missing.md)", encoding="utf-8")
        self.assertEqual(self.check().returncode, 1)

    def test_missing_notices_are_rejected(self):
        (self.root / "THIRD_PARTY_NOTICES.txt").unlink()
        self.assertEqual(self.check().returncode, 1)

    def test_tracked_private_data_is_rejected(self):
        (self.root / "profile.json").write_text("{}", encoding="utf-8")
        self.stage()
        self.assertEqual(self.check().returncode, 1)

    def test_tracked_internal_document_is_rejected(self):
        (self.root / "docs/superpowers").mkdir(parents=True)
        (self.root / "docs/superpowers/plan.md").write_text("internal", encoding="utf-8")
        self.stage()
        self.assertEqual(self.check().returncode, 1)


if __name__ == "__main__":
    unittest.main()
