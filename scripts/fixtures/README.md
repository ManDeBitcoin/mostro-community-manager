# Datos de prueba de los scripts

- `throwaway.nsec`: clave Nostr **desechable**, solo para pruebas. `daemon-contract-smoke.sh` y `make-db-fixture.sh` la pasan a mostrod (`MOSTRO_NSEC_PRIVKEY`) para que arranque sin red. Nunca tuvo fondos ni publicó en un relay. No la uses para nada más.
- `mostro-db/mostro-X.Y.Z.db`: la base que deja el mostrod oficial X.Y.Z, con dos filas sintéticas en cada tabla. `daemon-contract-smoke.sh` comprueba que el mostrod de la imagen las migre todas: cada migración aplicada y exitosa, integridad intacta y ninguna fila perdida. Se generan con `scripts/make-db-fixture.sh X.Y.Z SHA256_AMD64`.

Conviene tener la base de la versión fijada anterior (la que tienen los nodos al actualizar) y alguna más antigua. Hoy: 0.19.2, la anterior a la 0.20.0. El workflow `mostrod-upstream.yml` agrega la de la versión fijada cada vez que propone una nueva.
