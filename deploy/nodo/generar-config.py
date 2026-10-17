#!/usr/bin/env python3
"""Genera data/bitcoind/bitcoin.conf y data/lnd/lnd.conf con una contraseña RPC nueva.

Uso: generar-config.py <mainnet|regtest> <carpeta data> [alias de LND]

La contraseña se crea aquí y va directa a lnd.conf (en claro) y a bitcoin.conf
(solo el hash rpcauth). No se imprime ni pasa por argumentos. No sobrescribe
archivos que ya existan. Lo usa preparar.sh.
"""
import hmac
import os
import secrets
import sys

red, data = sys.argv[1], sys.argv[2]
alias = sys.argv[3] if len(sys.argv) > 3 else ""
if red not in ("mainnet", "regtest"):
    sys.exit("La red tiene que ser mainnet o regtest")
rpcport = {"mainnet": 8332, "regtest": 18443}[red]

password = secrets.token_urlsafe(32)
salt = secrets.token_hex(16)
digest = hmac.new(salt.encode(), password.encode(), "SHA256").hexdigest()

# prune: 20 GB de bloques en vez de toda la cadena; LND funciona con un nodo
# podado. Para la cadena entera, prune=0. dbcache=300 cabe en el mem_limit
# del compose; más acelera la primera sincronización.
comun = f"""server=1
prune=20000
listen=0
maxmempool=100
par=2
dbcache=300
rpcauth=lnd:{salt}${digest}
zmqpubrawblock=tcp://0.0.0.0:28332
zmqpubrawtx=tcp://0.0.0.0:28333
"""
# El puerto RPC no se publica fuera de Docker: solo lo alcanzan los
# contenedores del proyecto, y además pide la contraseña.
rpc = "rpcbind=0.0.0.0\nrpcallowip=10.0.0.0/8\nrpcallowip=172.16.0.0/12\nrpcallowip=192.168.0.0/16\n"
if red == "mainnet":
    bitcoin_conf = comun + rpc
else:
    # En regtest, rpcbind solo vale dentro de la sección [regtest].
    bitcoin_conf = "regtest=1\n" + comun.replace("prune=20000\n", "") + "fallbackfee=0.0002\n[regtest]\n" + rpc

lnd_conf = f"""[Application Options]
{f"alias={alias}" if alias else "# alias=Mi comunidad"}
# Para que otros nodos te encuentren: externalip=<tu IP pública>
tlsextradomain=lnd
tlscertpath=/export/ro/tls.cert
readonlymacaroonpath=/export/ro/readonly.macaroon
adminmacaroonpath=/export/admin.macaroon
rpclisten=0.0.0.0:10009
restlisten=0.0.0.0:8080
listen=0.0.0.0:9735
wallet-unlock-password-file=/secrets/wallet-password
wallet-unlock-allow-create=true

[Bitcoin]
bitcoin.{red}=true
bitcoin.node=bitcoind

[Bitcoind]
bitcoind.rpchost=bitcoind:{rpcport}
bitcoind.rpcuser=lnd
bitcoind.rpcpass={password}
bitcoind.zmqpubrawblock=tcp://bitcoind:28332
bitcoind.zmqpubrawtx=tcp://bitcoind:28333
"""

for ruta, texto in (
    (os.path.join(data, "bitcoind", "bitcoin.conf"), bitcoin_conf),
    (os.path.join(data, "lnd", "lnd.conf"), lnd_conf),
):
    try:
        fd = os.open(ruta, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        print("ya existe, no se toca:", ruta)
        continue
    except PermissionError:
        # Tras el primer arranque la carpeta es del usuario 1000.
        print("ya preparada (carpeta de otro usuario), no se toca:", ruta)
        continue
    with os.fdopen(fd, "w") as f:
        f.write(texto)
    print("escrito", ruta)
