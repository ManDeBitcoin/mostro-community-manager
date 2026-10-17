"""Every place that names the pinned mostrod release agrees with config/versions.json.

The Rust test `every_pin_of_the_daemon_release_agrees` checks the same, but CI
runs the Rust tests inside the image build, where only `api/` and `config/`
exist. This one runs on the runner, where the Dockerfiles and scripts are.
"""
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class DaemonPinTests(unittest.TestCase):
    def setUp(self):
        self.mostro = json.loads((ROOT / "config/versions.json").read_text())["mostro"]
        self.version = self.mostro["version"]
        self.checksums = (self.mostro["linux_amd64_sha256"], self.mostro["linux_arm64_sha256"])

    def test_versions_file_is_consistent(self):
        self.assertEqual(self.mostro["tag"], f"v{self.version}")
        for checksum in self.checksums:
            self.assertRegex(checksum, r"^[0-9a-f]{64}$")
        self.assertNotEqual(*self.checksums)

    def test_dockerfiles_download_the_pinned_release(self):
        for name in ("docker/Dockerfile", "docker/Dockerfile.mostro"):
            text = (ROOT / name).read_text()
            self.assertIn(f"/releases/download/v{self.version}/", text, name)
            self.assertIn(f"ENV MOSTROD_VERSION={self.version}\n", text, name)
            for checksum in self.checksums:
                self.assertIn(checksum, text, name)

    def test_smoke_scripts_expect_the_pinned_version(self):
        for name in ("scripts/container-smoke.sh", "scripts/verify-mostro-image.sh"):
            self.assertIn(f"mostro p2p {self.version}", (ROOT / name).read_text(), name)

    def test_api_and_notice_name_the_pinned_version(self):
        self.assertIn(f'MOSTRO_VERSION: &str = "{self.version}"', (ROOT / "api/src/daemon.rs").read_text())
        notice = (ROOT / "config/upstream/NOTICE-mostrod.md").read_text()
        self.assertIn(f"Version: {self.version}", notice)
        self.assertIn(f"/tree/v{self.version}", notice)


if __name__ == "__main__":
    unittest.main()
