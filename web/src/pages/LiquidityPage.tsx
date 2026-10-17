import { useCallback, useEffect, useState } from 'react';
import type { LndChannelsReport, ServiceInfo } from '../types';
import { copyToClipboard, api } from '../lib/api';
import { formatNumber } from '../lib/format';
import { Icon } from '../components/ui';
import type { Navigate } from '../lib/navigation';
import { type RulesRead, rulesNotReadNote } from '../lib/overview';
import { Badge, Callout, EmptyState, Fact, FactGrid, LinkButton, PageHeader, Panel, Stat } from '../components/layout';

const SYNC = (value: boolean | null | undefined) => (value === true ? 'Sincronizado' : value === false ? 'Pendiente' : 'Desconocido');
const COUNT = (value: number | null | undefined) => (typeof value === 'number' ? String(value) : 'Desconocido');
const SERVICE_STATUS: Record<string, string> = { online: 'Conectado', offline: 'Sin conexión', unconfigured: 'Sin configurar', unknown: 'Desconocido', warning: 'Revisar' };

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

export function LiquidityOperationsPage({
  lightning,
  maxTradeSats,
  rulesRead,
  navigate,
}: {
  lightning?: ServiceInfo;
  /** The largest trade the saved rules allow. */
  maxTradeSats?: number;
  /** Until the saved rules have been read, a missing maximum is not "no rules". */
  rulesRead: RulesRead;
  navigate: Navigate;
}) {
  const [report, setReport] = useState<LndChannelsReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [useMock, setUseMock] = useState(false);
  const [error, setError] = useState('');

  const fetchChannels = useCallback(async (mockMode: boolean) => {
    setLoading(true);
    setError('');
    try {
      const data = await api<LndChannelsReport>(`/api/lnd/channels${mockMode ? '?mock=true' : ''}`);
      setReport(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Error al consultar canales LND');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchChannels(useMock);
  }, [useMock, fetchChannels]);

  const copyText = (text: string, label: string) => {
    copyToClipboard(text, () => {
      setCopiedKey(label);
      setTimeout(() => setCopiedKey(null), 2000);
    });
  };

  const totalCap = Number(report?.total_capacity_sats || 0);
  const totalLocal = Number(report?.total_local_balance_sats || 0);
  const totalRemote = Number(report?.total_remote_balance_sats || 0);

  // The channel report is "online" only when LND answered; otherwise its
  // zeros are placeholders, not balances.
  const unreadable = Boolean(report) && report?.status !== 'online';

  // How many trades of the largest size the inbound side of the active
  // channels can hold at once. An estimate: it ignores how the balance is split
  // between channels, the channel reserves, the bonds and the fees.
  const activeInbound = (report?.channels ?? []).filter((channel) => channel.active).reduce((sum, channel) => sum + Number(channel.remote_balance_sats || 0), 0);
  const simultaneous = maxTradeSats && maxTradeSats > 0 ? Math.floor(activeInbound / maxTradeSats) : 0;

  const localPct = totalCap > 0 ? Math.round((totalLocal / totalCap) * 100) : 0;
  const remotePct = totalCap > 0 ? Math.round((totalRemote / totalCap) * 100) : 0;

  return (
    <section className="content liquidity-page">
      <PageHeader
        group="Nodo"
        title="Lightning"
        description="Los canales y la liquidez del nodo LND que retiene y paga los sats de cada operación."
        actions={
          <button type="button" className="button button-secondary" onClick={() => void fetchChannels(useMock)} disabled={loading}>
            <Icon name="refresh" size={14} /> Actualizar
          </button>
        }
      />

      {unreadable && report?.status === 'unconfigured' && (
        <Callout
          tone="warn"
          title="El panel no tiene acceso de lectura a LND"
          action={<button type="button" className="button button-secondary" onClick={() => setUseMock(true)}>Ver un ejemplo</button>}
        >
          En Umbrel la conexión se configura sola al instalar la aplicación junto a Lightning. Fuera de Umbrel hacen falta las variables LND_REST_URL, LND_TLS_CERT y LND_READONLY_MACAROON.
        </Callout>
      )}
      {unreadable && report?.status !== 'unconfigured' && (
        <Callout
          tone="warn"
          title="El panel no pudo leer LND"
          action={<button type="button" className="button button-secondary" onClick={() => setUseMock(true)}>Ver un ejemplo</button>}
        >
          La consulta de los canales falló. Puede ser la conexión, el certificado o los permisos del macaroon de solo lectura. Comprueba que Lightning está en marcha.
        </Callout>
      )}

      {lightning && (
        <Panel
          id="lnd-state"
          title="Estado del nodo LND"
          description={lightning.detail || 'Lo que informa LND a través de su conexión de solo lectura.'}
          aside={<Badge tone={lightning.status === 'online' ? 'good' : lightning.status === 'unconfigured' || lightning.status === 'unknown' ? 'neutral' : 'warn'}>{SERVICE_STATUS[lightning.status] || 'Desconocido'}</Badge>}
        >
          {lightning.status === 'online' || lightning.status === 'warning' ? (
            <>
              {lightning.num_active_channels === 0 && <p className="lightning-capacity-note" role="status">LND está conectado, pero no tiene canales activos. El mercado no puede mover sats por Lightning.</p>}
              <FactGrid>
                {lightning.alias && <Fact label="Alias" value={lightning.alias} />}
                {lightning.network && <Fact label="Red" value={lightning.network} />}
                <Fact label="Cadena de bloques" value={SYNC(lightning.synced_to_chain)} />
                <Fact label="Grafo de la red" value={SYNC(lightning.synced_to_graph)} />
                <Fact label="Canales activos" value={COUNT(lightning.num_active_channels)} />
                <Fact label="Canales pendientes" value={COUNT(lightning.num_pending_channels)} />
                <Fact label="Canales inactivos" value={COUNT(lightning.num_inactive_channels)} />
              </FactGrid>
              <p className="panel-note">Tener canales y liquidez no garantiza que exista una ruta para cada pago.</p>
            </>
          ) : (
            <p className="panel-note">Cuando el panel pueda leer LND verás aquí la sincronización y los canales del nodo.</p>
          )}
        </Panel>
      )}

      {error && <div className="form-message error">{error}</div>}

      <Panel
        id="lnd-liquidity"
        title="Liquidez"
        description="Mostro necesita las dos: entrante para retener el depósito del vendedor y saliente para pagar al comprador."
        aside={
          <div className="filter-chips" role="group" aria-label="Origen de los datos">
            <button type="button" className={`filter-chip ${!useMock ? 'active' : ''}`} aria-pressed={!useMock} onClick={() => setUseMock(false)}>Tu nodo</button>
            <button type="button" className={`filter-chip ${useMock ? 'active' : ''}`} aria-pressed={useMock} onClick={() => setUseMock(true)}>Ejemplo</button>
          </div>
        }
      >
        {report?.is_mock && <Callout tone="info" title="Estás viendo un ejemplo">Estos canales y saldos son de demostración, no los de tu nodo.</Callout>}
        {!report ? (
          <p className="panel-note">{loading ? 'Consultando los canales…' : 'No se pudieron consultar los canales.'}</p>
        ) : unreadable ? (
          <EmptyState icon="bolt" title="Sin datos de liquidez">
            El panel no puede leer los canales de tu nodo, así que no sabe cuánta liquidez tiene. Pulsa Ejemplo para ver cómo se muestra.
          </EmptyState>
        ) : (
        <>
        <div className="stat-grid">
          <Stat label="Capacidad total" value={formatNumber(totalCap)} hint={`sats en ${plural(report.channels.length, 'canal', 'canales')}`} />
          <Stat label="Saliente" value={formatNumber(totalLocal)} hint={`${localPct} % · para pagar al comprador`} tone="good" />
          <Stat label="Entrante" value={formatNumber(totalRemote)} hint={`${remotePct} % · para retener al vendedor`} tone="info" />
          {maxTradeSats && maxTradeSats > 0 ? (
            <Stat
              label="Operaciones máximas a la vez"
              value={`≈ ${formatNumber(simultaneous)}`}
              hint={`de ${formatNumber(maxTradeSats)} sats, según la liquidez entrante de los canales activos. No cuenta garantías, comisiones ni reservas`}
              tone={simultaneous === 0 ? 'bad' : simultaneous < 3 ? 'warn' : 'good'}
            />
          ) : (
            <Stat
              label="Para recibir depósitos"
              value={totalCap === 0 ? '—' : report.inbound_sufficient ? 'Hay entrante' : 'Falta entrante'}
              hint={rulesRead === 'loaded' ? 'Sin reglas guardadas no se puede comparar con tu operación máxima' : rulesNotReadNote(rulesRead)}
              tone={totalCap === 0 ? 'neutral' : report.inbound_sufficient ? 'good' : 'bad'}
            />
          )}
        </div>
        {totalCap > 0 && (
          <div className="liquidity-bar" role="img" aria-label={`Saliente ${localPct} %, entrante ${remotePct} %`}>
            <div className="liquidity-bar-track">
              <div style={{ width: `${localPct}%`, background: '#76e0a1' }} />
              <div style={{ width: `${remotePct}%`, background: '#67c8f5' }} />
            </div>
            <div className="liquidity-bar-legend">
              <span style={{ color: '#76e0a1' }}>Saliente {localPct} %</span>
              <span style={{ color: '#67c8f5' }}>Entrante {remotePct} %</span>
            </div>
          </div>
        )}
        </>
        )}
      </Panel>

      <Panel id="lnd-channels" title="Canales" description="Cada canal con su reparto de saldo. Importes en sats." aside={report && !unreadable ? <Badge tone="neutral">{`${plural(report.num_active_channels, 'activo', 'activos')} · ${plural(report.num_inactive_channels, 'inactivo', 'inactivos')}`}</Badge> : undefined}>
        {!report || unreadable ? (
          <EmptyState icon="bolt" title="Sin datos de canales">
            Cada canal aparecerá aquí cuando el panel pueda leer LND.
          </EmptyState>
        ) : report.channels.length === 0 ? (
          <EmptyState icon="bolt" title="No hay canales abiertos">
            La guía de abajo explica cómo conseguir liquidez entrante y saliente.
          </EmptyState>
        ) : (
          <div className="table-wrap">
            <table className="data-table" aria-label="Canales Lightning">
              <thead>
                <tr>
                  <th>Nodo</th>
                  <th>Capacidad</th>
                  <th>Saliente</th>
                  <th>Entrante</th>
                  <th>Estado</th>
                  <th><span className="sr-only">Punto del canal</span></th>
                </tr>
              </thead>
              <tbody>
                {report.channels.map((c, idx) => {
                  const cap = Number(c.capacity_sats || 0);
                  const loc = Number(c.local_balance_sats || 0);
                  const rem = Number(c.remote_balance_sats || 0);
                  return (
                    <tr key={idx}>
                      <td style={{ maxWidth: '220px' }}>
                        <strong>{c.alias || (c.remote_pubkey ? `${c.remote_pubkey.slice(0, 10)}…` : 'Nodo sin identificar')}</strong>
                        <small style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}><code>{c.remote_pubkey}</code></small>
                      </td>
                      <td><strong>{formatNumber(cap)}</strong></td>
                      <td style={{ color: '#76e0a1' }}>{formatNumber(loc)}</td>
                      <td style={{ color: '#67c8f5' }}>{formatNumber(rem)}</td>
                      <td><Badge tone={c.active ? 'good' : 'neutral'}>{c.active ? 'Activo' : 'Inactivo'}</Badge></td>
                      <td className="cell-action">
                        <button type="button" className="copy-button" onClick={() => copyText(c.channel_point, `cp-${idx}`)} title={c.channel_point}>
                          {copiedKey === `cp-${idx}` ? 'Copiado ✓' : 'Copiar'}
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel id="lnd-guide" title="Cómo preparar los canales" description="Qué liquidez necesita un mercado y cómo conseguirla.">
        <ol className="step-list">
          <li>
            <strong>Entiende las dos direcciones.</strong>
            <span>
              En cada operación el vendedor paga un depósito a tu nodo: esos sats entran, y para recibirlos hace falta liquidez entrante. Cuando la operación termina, tu nodo paga al comprador: esos sats salen, y para enviarlos hace falta liquidez saliente y una ruta hasta su monedero. Con solo saldo de tu lado, los depósitos de los vendedores no llegan.
            </span>
          </li>
          <li>
            <strong>Consigue liquidez entrante.</strong>
            <span>
              Es la que suele faltar en un nodo nuevo. Puedes comprar un canal entrante a un proveedor de liquidez o en un mercado como Amboss Magma, o pedir a otro nodo que abra un canal hacia el tuyo. El canal tiene que abrirse en el nodo LND que usa Mostro: una app con su propio nodo Lightning no sirve para esto. Como referencia, ten varias veces el importe de tu operación máxima: cada operación en curso ocupa su importe hasta que se cierra, y las garantías ocupan el suyo.
            </span>
          </li>
          <li>
            <strong>Abre canales salientes hacia nodos bien conectados.</strong>
            <span>
              Dos o tres canales con nodos grandes y estables dan rutas para pagar a casi cualquier monedero. Si un pago al comprador no encuentra ruta, revisa también la comisión máxima de enrutamiento.{' '}
              <LinkButton onClick={() => navigate('config')}>Abrir configuración</LinkButton>
            </span>
          </li>
          <li>
            <strong>Comprueba y abre el mercado.</strong>
            <span>
              Un canal nuevo tarda unas confirmaciones en quedar activo. Cuando esta página muestre canales activos y la cadena sincronizada, ensaya una operación y activa Mostro.{' '}
              <LinkButton onClick={() => navigate('simulator')}>Ir al simulador</LinkButton>{' '}
              <LinkButton onClick={() => navigate('node')}>Ir al nodo</LinkButton>
            </span>
          </li>
        </ol>
      </Panel>
    </section>
  );
}
