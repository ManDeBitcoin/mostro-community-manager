#!/usr/bin/env python3
"""What changes for this Manager between two mostrod releases, as Markdown.

Compares upstream's settings.tpl.toml (active keys and commented examples),
the database migrations, the admin gRPC contract and the mostro-core version
between the two tags, for the body of the pull request that pins NEW.

Usage: scripts/mostrod_release_summary.py OLD NEW [--signers a,b] [--build OUTCOME] [--smoke OUTCOME]
"""
import argparse
import json
import re
import tomllib
import urllib.request

REPO = "MostroP2P/mostro"
# Sections the Manager writes keys into (api/src/config.rs render_settings).
MANAGED_SECTIONS = {"lightning", "nostr", "mostro", "rpc", "anti_abuse_bond"}


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "mostro-community-manager"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read()


def raw(tag: str, path: str) -> str:
    return fetch(f"https://raw.githubusercontent.com/{REPO}/{tag}/{path}").decode()


def template_keys(template: str) -> tuple[dict, dict]:
    """(active, commented): {(section, key): value} for each kind of key."""
    active = {}
    for section, table in tomllib.loads(template).items():
        if isinstance(table, dict):
            for key, value in table.items():
                if not isinstance(value, dict):
                    active[(section, key)] = value
    commented, section = {}, ""
    for line in template.splitlines():
        line = line.strip().lstrip("#").strip()
        header = re.fullmatch(r"\[+([^\]]+)\]+", line)
        if header:
            section = header.group(1)
            continue
        try:
            entry = tomllib.loads(line)
        except tomllib.TOMLDecodeError:
            continue
        if len(entry) == 1:
            key, value = next(iter(entry.items()))
            if not isinstance(value, dict) and (section, key) not in active:
                commented.setdefault((section, key), value)
    return active, commented


def describe(keys) -> str:
    return ", ".join(
        f"`[{s}] {k}`" + (" (**lo escribe el Manager**)" if s in MANAGED_SECTIONS else "")
        for s, k in sorted(keys)
    )


def summary(old: str, new: str, signers: str, build: str, smoke: str) -> str:
    old_tag, new_tag = f"v{old}", f"v{new}"
    lines = [
        f"Fija **mostrod {new_tag}** (antes {old_tag}). PR abierto por `.github/workflows/mostrod-upstream.yml`.",
        "",
        f"- Notas de la release: https://github.com/{REPO}/releases/tag/{new_tag}",
        f"- Cambios: https://github.com/{REPO}/compare/{old_tag}...{new_tag}",
        "",
        "## Comprobado por el workflow",
        "",
        f"- Firmas de `manifest.txt`: {signers or 'ninguna de confianza'} (claves de `config/upstream/keys/`).",
        "- SHA-256 de los binarios musl amd64 y arm64 iguales a los del manifiesto.",
        f"- Build de la imagen (pruebas Rust, contrato con la plantilla, clippy, panel): **{build or 'sin ejecutar'}**.",
        f"- `container-smoke.sh` y `daemon-contract-smoke.sh` (mostrod nuevo contra la configuración generada y las bases de versiones anteriores): **{smoke or 'sin ejecutar'}**.",
        "- arm64: lo corre `image-check.yml`, lanzado sobre esta rama.",
        "",
    ]

    old_active, old_commented = template_keys(raw(old_tag, "settings.tpl.toml"))
    new_active, new_commented = template_keys(raw(new_tag, "settings.tpl.toml"))
    added = new_active.keys() - old_active.keys()
    removed = old_active.keys() - new_active.keys()
    changed = {k for k in new_active.keys() & old_active.keys() if new_active[k] != old_active[k]}
    examples_added = new_commented.keys() - old_commented.keys() - old_active.keys()
    examples_removed = old_commented.keys() - new_commented.keys() - new_active.keys()
    lines += ["## Plantilla de configuración (`settings.tpl.toml`)", ""]
    if not (added or removed or changed or examples_added or examples_removed):
        lines.append("Sin cambios en las claves.")
    if added:
        lines.append(f"- Claves activas nuevas (llegan con el valor de upstream): {describe(added)}")
    if removed:
        lines.append(f"- Claves activas que desaparecen: {describe(removed)}")
    for key in sorted(changed):
        lines.append(f"- Valor por defecto cambiado: {describe([key])}: `{old_active[key]!r}` → `{new_active[key]!r}`")
    if examples_added:
        lines.append(f"- Ejemplos comentados nuevos (opcionales): {describe(examples_added)}")
    if examples_removed:
        lines.append(f"- Ejemplos comentados que desaparecen: {describe(examples_removed)}")
    lines.append("")

    compare = json.loads(fetch(f"https://api.github.com/repos/{REPO}/compare/{old_tag}...{new_tag}"))
    migrations = sorted(
        f["filename"].split("/", 1)[1]
        for f in compare.get("files", [])
        if f["filename"].startswith("migrations/") and f["status"] == "added"
    )
    lines += ["## Base de datos", ""]
    if migrations:
        lines.append(f"{len(migrations)} migración(es) nueva(s). Solo van hacia adelante: volver a {old_tag} exige la copia que el contenedor `mostro` guarda en `db-snapshots/` antes del primer arranque.")
        lines += [f"- `{m}`" for m in migrations]
    else:
        lines.append("Sin migraciones nuevas.")
    if len(compare.get("files", [])) >= 300:
        lines.append("- (La comparación de GitHub tiene más de 300 archivos: la lista puede estar incompleta.)")
    lines.append("")

    def rpcs(tag: str) -> set:
        return set(re.findall(r"^\s*rpc\s+(\w+)\(", raw(tag, "proto/admin.proto"), re.M))

    old_rpcs, new_rpcs = rpcs(old_tag), rpcs(new_tag)
    lines += ["## Contrato gRPC (`admin.proto`)", ""]
    if old_rpcs == new_rpcs:
        lines.append("Sin RPC nuevas ni retiradas (la API solo usa `GetVersion`).")
    for name in sorted(new_rpcs - old_rpcs):
        lines.append(f"- RPC nueva: `{name}`")
    for name in sorted(old_rpcs - new_rpcs):
        lines.append(f"- RPC retirada: `{name}`" + (" (**la usa la API**)" if name == "GetVersion" else ""))
    lines.append("")

    def core(tag: str) -> str:
        found = re.search(r'^mostro-core\s*=\s*(?:\{[^}]*version\s*=\s*)?"([^"]+)"', raw(tag, "Cargo.toml"), re.M)
        return found.group(1) if found else "?"

    old_core, new_core = core(old_tag), core(new_tag)
    lines += ["## Protocolo", ""]
    lines.append(f"mostro-core {old_core} → {new_core}." if old_core != new_core else f"mostro-core sin cambios ({new_core}).")
    lines += [
        "",
        "## Antes de fusionar",
        "",
        "- Leer las notas de la release y escribir las de esta versión del Manager en `docs/release-notes.md`.",
        "- Registrar lo verificado en `docs/validation.md`.",
        "",
        "🤖 Generated with [Claude Code](https://claude.com/claude-code)",
    ]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("old")
    parser.add_argument("new")
    parser.add_argument("--signers", default="")
    parser.add_argument("--build", default="")
    parser.add_argument("--smoke", default="")
    args = parser.parse_args()
    print(summary(args.old, args.new, args.signers, args.build, args.smoke), end="")


if __name__ == "__main__":
    main()
