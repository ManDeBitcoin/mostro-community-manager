import { useState } from 'react';
import type { PanelData } from '../hooks/usePanelData';
import { api } from '../lib/api';
import { formatDateTime } from '../lib/format';
import { Badge, Callout, EmptyState, Fact, FactGrid, PageHeader, Panel } from '../components/layout';

// On Umbrel the API lives in the app's `web` container; this is how one of its commands is run there.
const UMBREL_CONTAINER = 'mandebitcoin-mostro-manager_web_1';
const DOCKER_EXEC = `sudo docker exec -it --user 1000:1000 ${UMBREL_CONTAINER} mostro-community-api`;

export function BackupsPage({ data }: { data: PanelData }) {
  const { backups, refreshBackups } = data;
  const backupDir = (backups?.target_dir || '/data/backup').replace(/\/$/, '');
  const [running, setRunning] = useState(false);
  const [passphrase, setPassphrase] = useState('');
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const tooShort = passphrase.length > 0 && passphrase.length < 16;

  const runBackup = async (event: React.FormEvent) => {
    event.preventDefault();
    setRunning(true);
    setResult(null);
    try {
      const res = await api<{ status: string; path: string; revision: number }>('/api/backup/trigger', {
        method: 'POST',
        body: JSON.stringify({ passphrase: passphrase || undefined }),
      });
      setResult({ ok: true, text: `Respaldo creado y verificado. Contiene la revisión ${res.revision} de la configuración.` });
      setPassphrase('');
      await refreshBackups();
    } catch (err) {
      setResult({ ok: false, text: err instanceof Error ? err.message : 'No se pudo crear el respaldo.' });
    } finally {
      setRunning(false);
    }
  };

  return (
    <section className="content backups-page">
      <PageHeader group="Ajustes" title="Respaldos" description="Copias cifradas de la identidad del nodo y de las reglas de la comunidad." />

      <Callout tone="info" title="Qué guarda un respaldo y qué no">
        Guarda la clave privada del nodo y la configuración guardada en un archivo, y la base de datos del daemon (órdenes, usuarios, garantías y reputación) en otro, <code>mostro-db-…</code>, con la misma fecha. Los dos se cifran con tu frase y se comprueban antes de darlos por buenos; si el nodo aún no tiene base, solo se guarda el primero.
      </Callout>

      <Panel id="backup-now" title="Crear un respaldo ahora" description="El archivo se cifra con la frase que escribas y se comprueba antes de darlo por bueno. Sin esa frase no se puede abrir: guárdala aparte.">
        <form className="inline-form" onSubmit={runBackup}>
          <label className="field">
            <span className="field-label">Frase de cifrado</span>
            <input type="password" autoComplete="new-password" placeholder="Mínimo 16 caracteres" value={passphrase} onChange={(event) => setPassphrase(event.target.value)} />
            <span className="field-hint">
              {tooShort ? `Faltan ${16 - passphrase.length} caracteres.` : backups?.enabled ? 'Déjala vacía para usar la frase del respaldo automático.' : 'El panel no la guarda ni la vuelve a mostrar.'}
            </span>
          </label>
          <button type="submit" className="button button-primary" disabled={running || tooShort || (!passphrase && !backups?.enabled)}>
            {running ? 'Cifrando…' : 'Crear respaldo'}
          </button>
        </form>
        {result && (
          <div className={`form-message ${result.ok ? 'success' : 'error'}`} role="status">
            {result.text}
          </div>
        )}
      </Panel>

      <Panel
        id="backup-auto"
        title="Respaldo automático"
        description="Repite la copia cada cierto tiempo, sin que tengas que acordarte."
        aside={backups ? <Badge tone={backups.enabled ? 'good' : 'neutral'}>{backups.enabled ? 'Activo' : 'Desactivado'}</Badge> : undefined}
      >
        {backups ? (
          <>
            {!backups.enabled && (
              <p className="panel-note">
                Se activa cuando el servicio arranca con la frase de cifrado en la variable de entorno <code>BACKUP_PASSPHRASE</code>. El paquete de Umbrel no la define: en Umbrel los respaldos se crean a mano.
              </p>
            )}
            {backups.last_error && backups.enabled && <Callout tone="warn" title="El último intento falló">{backups.last_error}</Callout>}
            <FactGrid>
              <Fact label="Carpeta de destino" value={backups.target_dir} mono />
              <Fact label="Cada cuánto" value={`${Math.round(backups.interval_secs / 3600)} h`} />
              <Fact label="Cuántos se conservan" value={`Los ${backups.retention_count} más recientes`} />
              <Fact
                label="Último respaldo"
                value={backups.last_run_timestamp ? formatDateTime(backups.last_run_timestamp) : 'Ninguno todavía'}
                hint={backups.last_run_timestamp ? (backups.last_run_success ? 'Terminó bien' : 'Terminó con error') : undefined}
              />
            </FactGrid>
          </>
        ) : (
          <p className="panel-note">Consultando el estado de los respaldos…</p>
        )}
      </Panel>

      <Panel
        id="backup-list"
        title="Respaldos guardados"
        description={`Los archivos que hay ahora en la carpeta de destino. Se conservan los ${backups?.retention_count ?? 7} más recientes: cada respaldo nuevo, manual o automático, borra los que sobran.`}
      >
        {backups && backups.backups.length > 0 ? (
          <>
            <ul className="file-list">
              {backups.backups.map((entry) => (
                <li key={entry.filename}>
                  <code>{entry.filename}</code>
                  <span>{entry.kind === 'database' ? 'Base del daemon' : 'Identidad y reglas'}</span>
                  <span>{(entry.size_bytes / 1024).toFixed(1)} KB</span>
                  <time dateTime={new Date(entry.modified_timestamp * 1000).toISOString()}>{formatDateTime(entry.modified_timestamp)}</time>
                </li>
              ))}
            </ul>
            <p className="panel-note">Un respaldo que vive en el mismo disco no protege de perderlo. En Umbrel, para copiar uno a la carpeta actual por SSH:</p>
            <code className="command">sudo docker cp {UMBREL_CONTAINER}:{backupDir}/{backups.backups[0].filename} .</code>
          </>
        ) : backups ? (
          <EmptyState icon="save" title="No hay ningún respaldo en esta carpeta">
            Crea el primero arriba. Después copia el archivo fuera de este equipo: un respaldo que vive en el mismo disco no protege de perderlo. Los que se crean por terminal con <code>export-backup</code> se guardan en otra carpeta y no aparecen aquí.
          </EmptyState>
        ) : (
          <p className="panel-note">{data.settled ? 'El panel no ha podido leer la carpeta de respaldos. Lo vuelve a intentar solo.' : 'Consultando los respaldos…'}</p>
        )}
      </Panel>

      <Panel id="backup-restore" title="Restaurar" description="La restauración no se hace desde el panel. Se ejecuta por SSH en el equipo, dentro del contenedor de la aplicación, y siempre sobre una carpeta nueva para no pisar una instalación en uso.">
        <ol className="step-list">
          <li>
            <strong>Comprueba que el archivo se abre con tu frase.</strong>
            <code className="command">{DOCKER_EXEC} verify-backup {backupDir}/&lt;archivo.age&gt;</code>
          </li>
          <li>
            <strong>Restaura en una carpeta que aún no exista.</strong>
            <code className="command">{DOCKER_EXEC} restore-backup {backupDir}/&lt;archivo.age&gt; {backupDir}/restaurado</code>
          </li>
          <li>
            <strong>La base del daemon se restaura aparte</strong>, en un archivo que aún no exista; se comprueba su integridad antes de escribirlo.
            <code className="command">{DOCKER_EXEC} restore-db {backupDir}/mostro-db-&lt;…&gt;.age {backupDir}/mostro.db</code>
          </li>
        </ol>
        <p className="panel-note">
          Los dos comandos piden la frase en la terminal. El destino tiene que colgar de una carpeta privada, como la de respaldos. La carpeta restaurada contiene las reglas y la clave privada del nodo sin cifrar: úsala y bórrala. Aplicarla a una instalación es un paso manual.
        </p>
      </Panel>
    </section>
  );
}
