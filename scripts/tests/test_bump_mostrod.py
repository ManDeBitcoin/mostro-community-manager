"""scripts/bump-mostrod.py moves every pin of the daemon release together."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[2]
PINNED_FILES = (
    "config/versions.json",
    "docker/Dockerfile",
    "docker/Dockerfile.mostro",
    "api/src/daemon.rs",
    "scripts/container-smoke.sh",
    "scripts/verify-mostro-image.sh",
    "config/upstream/NOTICE-mostrod.md",
    "config/upstream/README.md",
)
NEW_AMD64 = "a" * 64
NEW_ARM64 = "b" * 64


class BumpMostrodTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        for relative in PINNED_FILES:
            (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(SOURCE / relative, self.root / relative)
        self.old = json.loads((SOURCE / "config/versions.json").read_text())["mostro"]
        major, minor, patch = (int(part) for part in self.old["version"].split("."))
        self.new = f"{major}.{minor + 1}.0"
        self.template = self.root / "new-template.toml"
        self.template.write_text("[mostro]\nfee = 0\n")
        self.proto = self.root / "new-admin.proto"
        self.proto.write_text('syntax = "proto3";\n')

    def bump(self, *extra):
        return subprocess.run(
            [sys.executable, str(SOURCE / "scripts/bump-mostrod.py"), self.new,
             "--amd64", NEW_AMD64, "--arm64", NEW_ARM64,
             "--template", str(self.template), "--proto", str(self.proto),
             "--root", str(self.root), *extra],
            capture_output=True, text=True,
        )

    def test_every_pin_moves_to_the_new_release(self):
        result = self.bump("--verified")
        self.assertEqual(result.returncode, 0, result.stderr)
        mostro = json.loads((self.root / "config/versions.json").read_text())["mostro"]
        self.assertEqual(mostro["version"], self.new)
        self.assertEqual(mostro["tag"], f"v{self.new}")
        self.assertEqual((mostro["linux_amd64_sha256"], mostro["linux_arm64_sha256"]), (NEW_AMD64, NEW_ARM64))
        self.assertTrue(mostro["integration_status"].startswith("signatures-and-checksums-verified-"))
        for dockerfile in ("docker/Dockerfile", "docker/Dockerfile.mostro"):
            text = (self.root / dockerfile).read_text()
            self.assertIn(f"/releases/download/v{self.new}/", text)
            self.assertIn(f"ENV MOSTROD_VERSION={self.new}\n", text)
            self.assertIn(NEW_AMD64, text)
            self.assertIn(NEW_ARM64, text)
            self.assertNotIn(self.old["linux_amd64_sha256"], text)
        self.assertIn(f'MOSTRO_VERSION: &str = "{self.new}"', (self.root / "api/src/daemon.rs").read_text())
        for script in ("scripts/container-smoke.sh", "scripts/verify-mostro-image.sh"):
            self.assertIn(f"mostro p2p {self.new}", (self.root / script).read_text())
        notice = (self.root / "config/upstream/NOTICE-mostrod.md").read_text()
        self.assertIn(f"Version: {self.new}", notice)
        self.assertIn(f"/tree/v{self.new}", notice)
        self.assertEqual((self.root / "config/upstream/settings.tpl.toml").read_text(), self.template.read_text())
        self.assertEqual((self.root / "config/upstream/admin.proto").read_text(), self.proto.read_text())
        for relative in PINNED_FILES:
            self.assertNotIn(f"v{self.old['version']}/", (self.root / relative).read_text(), relative)

    def test_an_unverified_bump_says_so(self):
        self.assertEqual(self.bump().returncode, 0)
        mostro = json.loads((self.root / "config/versions.json").read_text())["mostro"]
        self.assertEqual(mostro["integration_status"], "unverified")

    def test_the_pinned_release_is_refused(self):
        self.new = self.old["version"]
        result = self.bump()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already pinned", result.stderr)


if __name__ == "__main__":
    unittest.main()
