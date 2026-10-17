import { useEffect, useState } from 'react';
import type { PanelData } from '../hooks/usePanelData';
import type { CardPublication, CardRelayReport, ConnectionInfo } from '../types';
import type { Navigate } from '../lib/navigation';
import { copyToClipboard } from '../lib/api';
import { formatAge, formatDateTime, formatWhen } from '../lib/format';
import { Icon, Toggle } from '../components/ui';
import { type Tone, Badge, Callout, CopyField, EmptyState, LinkButton, PageHeader, Panel } from '../components/layout';

type QrFormat = 'card' | 'uri' | 'json';

const GUIDE_URL = 'https://github.com/ManDeBitcoin/mostro-community-manager/blob/main/docs/INTEGRACION-APPS.md';

const FORMATS: { id: QrFormat; label: string; caption: string }[] = [
  { id: 'card', label: 'Tarjeta', caption: 'La tarjeta de la comunidad firmada por el nodo: nombre, relays, moneda y métodos de pago. Es la opción recomendada.' },
  { id: 'uri', label: 'Identidad', caption: 'La clave pública y los relays (nprofile). La app no recibe moneda ni métodos de pago.' },
  { id: 'json', label: 'JSON', caption: 'La misma tarjeta firmada, como texto JSON para copiar y pegar.' },
];

const RELAY_OUTCOME: Record<CardRelayReport['outcome'], { tone: Tone; card: string; deletion: string }> = {
  accepted: { tone: 'good', card: 'La tiene', deletion: 'Borrado aceptado' },
  rejected: { tone: 'bad', card: 'La rechazó', deletion: 'Rechazó el borrado' },
  no_answer: { tone: 'warn', card: 'Sin respuesta', deletion: 'Sin respuesta' },
  unreachable: { tone: 'warn', card: 'Sin conexión', deletion: 'Sin conexión' },
  pending: { tone: 'neutral', card: 'Pendiente', deletion: 'Pendiente' },
};

/** What the publication of the card is doing, in a title, a sentence and a tone. */
function publicationSummary(publication: CardPublication): { tone: Tone; title: string; detail: string } | null {
  const total = publication.relays.length;
  const accepted = publication.relays.filter((relay) => relay.outcome === 'accepted').length;
  const resend = publication.resend_every_secs > 0 ? ` El panel la vuelve a enviar igual cada ${formatAge(publication.resend_every_secs)}, y publica una nueva cuando guardas un cambio que la afecta.` : '';
  const retry = publication.next_attempt_at ? ` Lo vuelve a intentar solo, ${formatWhenAhead(publication.next_attempt_at)}.` : ' Lo vuelve a intentar solo.';
  switch (publication.state) {
    case 'published':
      return { tone: 'good', title: total === 1 ? 'Publicada en el relay del nodo' : `Publicada en los ${total} relays del nodo`, detail: `La tarjeta cambió por última vez el ${formatDateTime(publication.card_changed_at)}: es la fecha que ven las apps.${resend}` };
    case 'partial':
      return { tone: 'warn', title: `Publicada en ${accepted} de ${total} relays`, detail: `Una app que solo use los demás no la encontrará.${retry}` };
    case 'failed':
      return { tone: 'bad', title: 'La tarjeta no está publicada', detail: publication.reason_text || `Ningún relay la aceptó.${retry}` };
    case 'blocked':
      return { tone: 'warn', title: 'No se publica nada', detail: publication.reason_text || 'El nodo no puede emitir la tarjeta.' };
    case 'withdrawing':
      return publication.reason_text
        ? { tone: 'warn', title: 'La tarjeta aún no se ha podido retirar', detail: publication.reason_text }
        : { tone: 'info', title: 'Retirando la tarjeta', detail: `El panel ya no la envía y ha pedido a los relays que la borren. Aún no lo han aceptado todos.${retry}` };
    case 'withdrawn':
      return { tone: 'neutral', title: `Tarjeta retirada el ${formatDateTime(publication.withdrawn_at)}`, detail: 'Los relays que la recibieron aceptaron la petición de borrado. Una app que ya la hubiera leído la conserva.' };
    case 'withdrawal_incomplete':
      return { tone: 'warn', title: 'La tarjeta no se pudo retirar de todos los relays', detail: 'El panel dejó de insistir tras una semana. Los relays de abajo pueden conservar una copia. Si vuelves a publicarla y a retirarla, el panel lo pide de nuevo.' };
    default:
      return publication.reason_text ? { tone: 'warn', title: 'La tarjeta no se pudo retirar', detail: publication.reason_text } : null;
  }
}

/** `dentro de 4 min`, for a moment the server gave as a unix timestamp. */
const formatWhenAhead = (unixSecs: number) => {
  const ahead = Math.round(unixSecs - Date.now() / 1000);
  return ahead <= 5 ? 'enseguida' : `dentro de ${formatAge(ahead)}`;
};

/** The switch that puts the community card on the node's relays, and what the relays answered. */
function PublishCardPanel({ data, connection, navigate }: { data: PanelData; connection: ConnectionInfo | null; navigate: Navigate }) {
  const publication = data.cardPublication;
  const { refreshCardPublication, setCardPublicationSwitch } = data;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  // The server answers a change at once and talks to the relays afterwards.
  const working = publication?.working ?? false;
  useEffect(() => {
    if (!working) return;
    const timer = setInterval(() => void refreshCardPublication(), 1500);
    return () => clearInterval(timer);
  }, [working, refreshCardPublication]);

  const change = async (enabled: boolean, ask: boolean) => {
    if (ask) {
      const card = connection?.card;
      const relays = card?.relays.length ?? connection?.relays.length ?? 0;
      const question = enabled
        ? `¿Publicar la tarjeta de la comunidad en ${relays === 1 ? 'el relay' : `los ${relays} relays`} del nodo?\n\nQuedarán a la vista de cualquiera, firmados con la clave del nodo: el nombre, la moneda, ${card ? (card.payment_methods.length === 1 ? 'el método de pago' : `los ${card.payment_methods.length} métodos de pago`) : 'los métodos de pago'}, la web y el contacto.\n\nPodrás retirarla después, pero quien ya la haya leído puede conservarla.`
        : '¿Dejar de publicar la tarjeta?\n\nEl panel dejará de enviarla y pedirá a los relays que la borren. Un relay puede no hacerlo, y las apps que ya la leyeron la conservan.';
      if (!window.confirm(question)) return;
    }
    setBusy(true);
    setError('');
    try {
      await setCardPublicationSwitch(enabled);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'No se pudo cambiar la publicación de la tarjeta.');
    } finally {
      setBusy(false);
    }
  };

  const summary = publication && !working ? publicationSummary(publication) : null;
  const withdrawing = publication ? !publication.enabled : false;
  const canRetry = publication && !working && ['partial', 'failed', 'withdrawing'].includes(publication.state);
  const fix = publication?.state === 'blocked' ? (publication.reason === 'no_identity' || publication.reason === 'identity_unreadable' ? (['node', 'Ir al nodo'] as const) : (['config', 'Abrir configuración'] as const)) : null;

  return (
    <Panel
      id="connect-publish"
      title="Publicar la tarjeta en los relays"
      description="Opcional. Con la tarjeta publicada, una app encuentra sola los métodos de pago y el contacto de tu comunidad, sin código ni enlace."
      aside={publication ? <Badge tone={working ? 'info' : publication.enabled ? (publication.state === 'published' ? 'good' : 'warn') : 'neutral'}>{working ? 'Actualizando…' : publication.enabled ? (publication.state === 'published' ? 'Publicada' : 'Activada') : 'Desactivada'}</Badge> : undefined}
    >
      {!publication ? (
        <p className="panel-note">{data.settled ? 'El panel no ha podido leer si la tarjeta se publica. Lo vuelve a intentar solo.' : 'Consultando…'}</p>
      ) : (
        <>
          <Toggle
            checked={publication.enabled}
            disabled={busy}
            onChange={(value) => void change(value, true)}
            label="Publicar la tarjeta de la comunidad"
            note={publication.enabled ? 'El panel la envía a los relays del nodo y la mantiene al día.' : 'Apagado, la tarjeta solo sale de aquí cuando tú compartes el código o el enlace.'}
          />
          <ul className="publish-facts">
            <li>
              <strong>Qué queda a la vista.</strong> El nombre de la comunidad, la moneda, los métodos de pago activos, la web y el contacto, con la comisión y la garantía como referencia. Cualquiera que consulte los relays puede leerlo, copiarlo y saber que es de este nodo, porque va firmado con su clave.
            </li>
            <li>
              <strong>Qué no cambia.</strong> No dice si el nodo está en marcha ni fija sus reglas: eso lo anuncia el propio nodo, y es lo que las apps aplican.
            </li>
            <li>
              <strong>Si lo apagas.</strong> El panel deja de enviarla y pide a los relays que la borren. Es una petición: un relay puede no atenderla, y una app que ya la leyó la conserva.
            </li>
          </ul>
          {error && <p className="form-message error" role="alert">{error}</p>}
          {working && <Callout tone="info" title="Hablando con los relays…">Cada relay tiene unos segundos para responder.</Callout>}
          {summary && (
            <Callout
              tone={summary.tone}
              title={summary.title}
              action={
                fix ? (
                  <LinkButton onClick={() => navigate(fix[0])}>{fix[1]}</LinkButton>
                ) : canRetry ? (
                  <button type="button" className="button button-secondary" disabled={busy} onClick={() => void change(publication.enabled, false)}>
                    Reintentar ahora
                  </button>
                ) : undefined
              }
            >
              {summary.detail}
            </Callout>
          )}
          {!working && publication.enabled && (publication.former_relays?.length ?? 0) > 0 && (
            <p className="panel-note">
              Ya no están en tu configuración y pueden conservar una versión anterior de la tarjeta: {publication.former_relays?.join(', ')}. El panel no les envía las nuevas; si apagas la publicación, les pide también que la borren.
            </p>
          )}
          {!working && publication.relays.length > 0 && (
            <ul className="publish-relays" aria-label={withdrawing ? 'Respuesta de cada relay a la petición de borrado' : 'Respuesta de cada relay a la tarjeta'}>
              {publication.relays.map((relay) => {
                const outcome = RELAY_OUTCOME[relay.outcome] ?? RELAY_OUTCOME.no_answer;
                return (
                  <li key={relay.url}>
                    <code>{relay.url}</code>
                    <Badge tone={outcome.tone}>{withdrawing ? outcome.deletion : outcome.card}</Badge>
                    <small>
                      {relay.detail ? `${relay.detail} · ` : ''}
                      {formatWhen(relay.at)}
                    </small>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
    </Panel>
  );
}

export function ConnectPage({ data, navigate }: { data: PanelData; navigate: Navigate }) {
  const { connection, settled } = data;
  const [qrFormat, setQrFormat] = useState<QrFormat>('card');
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const copy = (text: string, label: string) =>
    copyToClipboard(text, () => {
      setCopiedField(label);
      setTimeout(() => setCopiedField(null), 2000);
    });

  const ready = connection?.status === 'ready';
  const format = FORMATS.find((item) => item.id === qrFormat) ?? FORMATS[0];
  const svg = !connection ? null : qrFormat === 'card' ? connection.qr_card_svg : qrFormat === 'uri' ? connection.qr_svg : connection.qr_json_svg;
  const link = !connection ? null : qrFormat === 'card' ? connection.card_uri : qrFormat === 'uri' ? connection.nostr_uri : connection.json_uri;
  const linkLabel = qrFormat === 'card' ? 'Enlace de la tarjeta firmada' : qrFormat === 'uri' ? 'Enlace con la identidad y los relays' : 'Tarjeta firmada en JSON';

  return (
    <section className="content connect-page">
      <PageHeader
        group="Nodo"
        title="Conexión de apps"
        description="Lo que una app necesita para operar con tu comunidad: la clave pública del nodo y sus relays."
        actions={
          connection?.app_download_url ? (
            <a href={connection.app_download_url} target="_blank" rel="noreferrer" className="button button-secondary">
              Mostro App <Icon name="external" size={14} />
            </a>
          ) : undefined
        }
      />

      {!ready ? (
        <Panel>
          {connection ? (
            <EmptyState icon="qr" title="Aún no hay nada que compartir" action={<LinkButton onClick={() => navigate('node')}>Ir al nodo</LinkButton>}>
              La conexión se genera a partir de la identidad del nodo. Créala o impórtala primero.
            </EmptyState>
          ) : (
            // Not read yet, or not readable: neither means the node has no identity.
            <EmptyState icon="qr" title={settled ? 'No se pudo leer la conexión' : 'Consultando…'}>
              {settled ? 'El panel no ha recibido los datos de conexión de su servidor. Lo vuelve a intentar solo.' : 'Leyendo la identidad del nodo.'}
            </EmptyState>
          )}
        </Panel>
      ) : (
        <Panel id="connect-share" title="Compartir con una app" description="Muestra el código para escanearlo o copia el enlace. La tarjeta lleva la comisión y el porcentaje de garantía como referencia; los valores vigentes y los límites los lee la app del propio nodo. Si cambias la comisión o la garantía, vuelve a compartirla.">
          <div className="connection-body">
            <div className="connection-qr-pane">
              <div className="qr-toggle-group" role="group" aria-label="Formato del código">
                {FORMATS.map((item) => (
                  <button key={item.id} type="button" aria-pressed={qrFormat === item.id} className={`qr-toggle-btn ${qrFormat === item.id ? 'active' : ''}`} onClick={() => setQrFormat(item.id)}>
                    {item.label}
                  </button>
                ))}
              </div>
              <div className="qr-box-wrapper">
                {svg ? (
                  <div className="qr-box" dangerouslySetInnerHTML={{ __html: svg }} title="Código QR de conexión" />
                ) : (
                  // With a link and no code, the link is too long for one. Without either, the server
                  // says why there is no card: missing rules and rules it cannot carry are fixed differently.
                  <span className="empty-hint">{link ? `Es demasiado largo para un código QR. Cópialo del campo «${linkLabel}».` : connection.card_unavailable || 'El panel no ha recibido la tarjeta de su servidor.'}</span>
                )}
              </div>
              <span className="qr-hint-caption">{format.caption}</span>
            </div>

            <div className="connection-details">
              {link && <CopyField label={linkLabel} value={link} copied={copiedField === 'link'} onCopy={() => copy(link, 'link')} />}
              <CopyField label="Clave pública del nodo (npub)" value={connection.npub || ''} copied={copiedField === 'npub'} onCopy={() => copy(connection.npub || '', 'npub')} />
              <CopyField label="La misma clave en hexadecimal" value={connection.pubkey_hex || ''} copied={copiedField === 'hex'} onCopy={() => copy(connection.pubkey_hex || '', 'hex')} />
              <div className="connection-relays-row">
                <span className="field-label">Relays</span>
                {connection.relays.length > 0 ? (
                  <div className="chips">
                    {connection.relays.map((relay) => (
                      <span key={relay} className="chip">{relay}</span>
                    ))}
                  </div>
                ) : (
                  <span className="empty-hint">Sin relays guardados. Añádelos en Configuración.</span>
                )}
              </div>
            </div>
          </div>
        </Panel>
      )}

      <PublishCardPanel data={data} connection={connection} navigate={navigate} />

      <Panel id="connect-how" title="Cómo se conecta una app" description="El panel no habla con las apps. Todo pasa por los relays de Nostr.">
        <ol className="step-list">
          <li>
            <strong>La app recibe la clave del nodo y sus relays.</strong>
            <span>Con el código, el enlace o pegando la clave a mano.</span>
          </li>
          <li>
            <strong>Lee en los relays lo que el nodo anuncia.</strong>
            <span>Versión, comisión, límites, monedas y garantía. Puedes ver lo mismo en la página del nodo. Si publicas la tarjeta, la app lee ahí también los métodos de pago y el contacto.</span>
          </li>
          <li>
            <strong>Publica y toma órdenes con mensajes cifrados al nodo.</strong>
            <span>El nodo responde por los mismos relays. Si app y nodo no comparten ninguno, no se ven.</span>
          </li>
        </ol>
        <p className="panel-note">
          <LinkButton onClick={() => navigate('node')}>Ver lo que anuncia el nodo</LinkButton>
        </p>
      </Panel>

      <Panel id="connect-rules" title="Reglas para crear órdenes" description="Valen para cualquier app. Si una orden se rechaza, casi siempre es por una de estas.">
        <p className="panel-note">Una orden lleva un importe fijo en sats o un precio de mercado con prima, nunca ambos. Mostro rechaza la combinación con «invalid_parameters».</p>
        <details className="help-details">
          <summary>Cómo resolver «Invalid parameters» o «prima no válida»</summary>
          <ul>
            <li><strong>Precio de mercado:</strong> Amount = 0, Fiat = 5 USD y Premium = 10 para +10 %. Mostro calcula los sats con su cotización cuando alguien toma la orden.</li>
            <li><strong>Importe fijo:</strong> Amount = 10000, Fiat = 5 USD y Premium = 0.</li>
            <li><strong>Rango:</strong> mínimo y máximo en fiat, Amount = 0 y Fiat = 0. La prima es opcional.</li>
          </ul>
          <p>El importe fiat y la prima son números enteros. Los sats resultantes deben estar entre el mínimo y el máximo del nodo y la moneda debe estar admitida. La prima de una orden no es la comisión del nodo.</p>
          <p>Si una app envía los sats estimados junto con una prima, el fallo está en la app: debe enviar Amount = 0.</p>
          <p>Si la app muestra otra versión o límites distintos de los configurados aquí, compara la clave pública y los relays, y revisa el anuncio en la página del nodo.</p>
        </details>
        <p className="panel-note">
          Para quien desarrolla una app:{' '}
          <a className="text-link" href={GUIDE_URL} target="_blank" rel="noreferrer">
            guía técnica de integración <Icon name="external" size={12} />
          </a>
        </p>
      </Panel>
    </section>
  );
}
