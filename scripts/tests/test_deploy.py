"""The Docker installations in deploy/ name the release they belong to.

An installation is a clone of this repository at a release tag, so the
.env.example of each one must point at the image of that same release: the
version of the first heading of docs/release-notes.md.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IMAGE = "ghcr.io/mandebitcoin/mostro-community-manager"
INSTALLS = ("compose", "nodo-completo")


class DeployTests(unittest.TestCase):
    def setUp(self):
        notes = (ROOT / "docs/release-notes.md").read_text()
        self.version = re.match(r"# Mostro Community Manager v(\d+\.\d+\.\d+)\n", notes).group(1)

    def test_each_install_starts_from_this_release(self):
        for install in INSTALLS:
            env = (ROOT / "deploy" / install / ".env.example").read_text()
            self.assertIn(f"\nMCM_IMAGE={IMAGE}:{self.version}\n", env, install)

    def test_the_manager_services_share_the_image_of_env(self):
        for install in INSTALLS:
            compose = (ROOT / "deploy" / install / "docker-compose.yml").read_text()
            self.assertEqual(compose.count("image: ${MCM_IMAGE:?falta MCM_IMAGE en .env}"), 3, install)

    def test_the_other_images_are_pinned_by_digest(self):
        compose = (ROOT / "deploy/nodo-completo/docker-compose.yml").read_text()
        others = [line.strip() for line in compose.splitlines()
                  if line.strip().startswith("image:") and "MCM_IMAGE" not in line]
        self.assertEqual(len(others), 2)
        for line in others:
            self.assertRegex(line, r"^image: \S+:\S+@sha256:[0-9a-f]{64}$")

    def test_the_guides_clone_this_release(self):
        for guide in ("instalar-docker.md", "instalar-nodo-completo.md"):
            text = (ROOT / "docs" / guide).read_text()
            self.assertIn(f"git clone --branch v{self.version} ", text, guide)


if __name__ == "__main__":
    unittest.main()
