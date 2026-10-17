#!/usr/bin/env python3
"""Pin another official mostrod release.

Rewrites every place that names the pinned release (the same ones
scripts/tests/test_daemon_pins.py and api/tests/configuration.rs check) and
copies upstream's settings.tpl.toml and proto/admin.proto of that release
into config/upstream/. Verify the release (signatures and checksums) first:
this script trusts the checksums it is given.

Usage:
  scripts/bump-mostrod.py 0.21.0 --amd64 SHA256 --arm64 SHA256 \\
      --template settings.tpl.toml --proto admin.proto [--verified]
"""
import argparse
import datetime
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def replace(root: Path, relative: str, pairs: list[tuple[str, str]]) -> None:
    path = root / relative
    text = path.read_text(encoding="utf-8")
    for old, new in pairs:
        if old not in text:
            sys.exit(f"{relative}: '{old}' not found; update scripts/bump-mostrod.py")
        text = text.replace(old, new)
    path.write_text(text, encoding="utf-8")


def bump(root: Path, version: str, amd64: str, arm64: str, template: Path, proto: Path, verified: bool) -> None:
    versions_path = root / "config/versions.json"
    versions = json.loads(versions_path.read_text(encoding="utf-8"))
    mostro = versions["mostro"]
    old, old_amd64, old_arm64 = mostro["version"], mostro["linux_amd64_sha256"], mostro["linux_arm64_sha256"]
    if old == version:
        sys.exit(f"mostrod {version} is already pinned")

    mostro.update(
        version=version,
        tag=f"v{version}",
        linux_amd64_sha256=amd64,
        linux_arm64_sha256=arm64,
        integration_status=(
            f"signatures-and-checksums-verified-{datetime.date.today().isoformat()}" if verified else "unverified"
        ),
    )
    versions_path.write_text(json.dumps(versions, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    for dockerfile in ("docker/Dockerfile", "docker/Dockerfile.mostro"):
        replace(root, dockerfile, [
            (f"/releases/download/v{old}/", f"/releases/download/v{version}/"),
            (old_amd64, amd64),
            (old_arm64, arm64),
        ])
    replace(root, "api/src/daemon.rs", [(f'MOSTRO_VERSION: &str = "{old}"', f'MOSTRO_VERSION: &str = "{version}"')])
    replace(root, "scripts/container-smoke.sh", [(f"mostro p2p {old}", f"mostro p2p {version}")])
    replace(root, "scripts/verify-mostro-image.sh", [
        (f"mostro p2p {old}", f"mostro p2p {version}"),
        (f"imagen oficial v{old}", f"imagen oficial v{version}"),
    ])
    replace(root, "config/upstream/NOTICE-mostrod.md", [
        (f"Version: {old}", f"Version: {version}"),
        (f"/tree/v{old}", f"/tree/v{version}"),
        (f"/releases/tag/v{old}", f"/releases/tag/v{version}"),
    ])
    replace(root, "config/upstream/README.md", [(f"/tree/v{old}", f"/tree/v{version}")])
    shutil.copyfile(template, root / "config/upstream/settings.tpl.toml")
    shutil.copyfile(proto, root / "config/upstream/admin.proto")
    print(f"mostrod {old} -> {version}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("version")
    parser.add_argument("--amd64", required=True, help="sha256 of mostrod-x86_64-unknown-linux-musl")
    parser.add_argument("--arm64", required=True, help="sha256 of mostrod-aarch64-unknown-linux-musl")
    parser.add_argument("--template", required=True, type=Path, help="settings.tpl.toml of that release")
    parser.add_argument("--proto", required=True, type=Path, help="proto/admin.proto of that release")
    parser.add_argument("--verified", action="store_true", help="signatures and checksums were verified")
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        sys.exit("version must look like 0.21.0")
    if not (SHA256.match(args.amd64) and SHA256.match(args.arm64)) or args.amd64 == args.arm64:
        sys.exit("--amd64 and --arm64 must be two different lowercase sha256 values")
    bump(args.root, args.version, args.amd64, args.arm64, args.template, args.proto, args.verified)


if __name__ == "__main__":
    main()
