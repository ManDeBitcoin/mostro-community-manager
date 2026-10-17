"""The Docker installations in deploy/ name the release they belong to.

An installation is a clone of this repository at a release tag, so the
Manager's .env.example must point at the image of that same release: the
version of the first heading of docs/release-notes.md. The node (bitcoind and
LND) is a project apart, with its own images pinned by digest.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IMAGE = "ghcr.io/mandebitcoin/mostro-community-manager"


class DeployTests(unittest.TestCase):
    def setUp(self):
        notes = (ROOT / "docs/release-notes.md").read_text()
        self.version = re.match(r"# Mostro Community Manager v(\d+\.\d+\.\d+)\n", notes).group(1)

    def test_the_manager_starts_from_this_release(self):
        env = (ROOT / "deploy/compose/.env.example").read_text()
        self.assertIn(f"\nMCM_IMAGE={IMAGE}:{self.version}\n", env)

    def test_the_manager_services_share_the_image_of_env(self):
        compose = (ROOT / "deploy/compose/docker-compose.yml").read_text()
        self.assertEqual(compose.count("image: ${MCM_IMAGE:?falta MCM_IMAGE en .env}"), 3)

    def test_the_node_is_apart_and_pinned_by_digest(self):
        compose = (ROOT / "deploy/nodo/docker-compose.yml").read_text()
        self.assertNotIn("MCM_IMAGE", compose)
        images = [line.strip() for line in compose.splitlines() if line.strip().startswith("image:")]
        self.assertEqual(len(images), 2)
        for line in images:
            self.assertRegex(line, r"^image: \S+:\S+@sha256:[0-9a-f]{64}$")

    def test_the_guides_clone_this_release(self):
        for guide in ("instalar-docker.md", "instalar-nodo-completo.md"):
            text = (ROOT / "docs" / guide).read_text()
            self.assertIn(f"--branch v{self.version} ", text, guide)


if __name__ == "__main__":
    unittest.main()
