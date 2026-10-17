import { type MouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from './lib/api';
import { shortKey } from './lib/format';
import { NAV_GROUPS, type PageId, navGroupOf, navItem, pageLink, useRoute } from './lib/navigation';
import { type RulesRead, announcedRules, attentionItems, cardAttention, isActive, isSettingUp, marketStatus, orderStats, savedRules, serviceTiles, setupSteps } from './lib/overview';
import { usePanelData } from './hooks/usePanelData';
import { useCommunityEditor } from './hooks/useCommunityEditor';
import { useSimulator } from './hooks/useSimulator';
import { useIdentity } from './hooks/useIdentity';
import { useDialogFocus } from './hooks/useDialogFocus';
import { useAuth } from './hooks/useAuth';
import { Icon } from './components/ui';
import { Callout } from './components/layout';
import { IdentityModals } from './components/IdentityModals';
import { HomePage } from './pages/HomePage';
import { OrdersPage } from './pages/OrdersPage';
import { MediationConsole } from './pages/MediationPage';
import { NodePage } from './pages/NodePage';
import { LiquidityOperationsPage } from './pages/LiquidityPage';
import { ConnectPage } from './pages/ConnectPage';
import { ConfigPage } from './pages/ConfigPage';
import { BackupsPage } from './pages/BackupsPage';
import { AlertsPage } from './pages/AlertsPage';
import { SimulatorPage } from './pages/SimulatorPage';
import { LoginPage } from './pages/LoginPage';

type Flash = { tone: 'good' | 'bad'; text: string };

const ALERTS_SEEN_KEY = 'mostro_alerts_seen_at';
const readSeenAt = () => {
  try {
    return Number(localStorage.getItem(ALERTS_SEEN_KEY)) || 0;
  } catch {
    return 0;
  }
};

// The panel is only mounted once the server lets it in, so nothing polls behind the login page.
function App() {
  const auth = useAuth();
  if (!auth.session && !auth.unreachable) return null;
  // Unreachable: the panel shows that on its own, and the first answer that asks for a login brings it here.
  if (auth.session && !auth.session.authenticated) return <LoginPage onLogin={auth.login} />;
  return <Panel onLogout={auth.session?.mode === 'password' ? auth.logout : undefined} />;
}

function Panel({ onLogout }: { onLogout?: () => Promise<void> }) {
  const [route, navigate] = useRoute();
  const page = route.page;
  const data = usePanelData();
  const { daemon, dashboard, orders, notifications, backups, health, cardPublication } = data;
  const [flash, setFlash] = useState<Flash | null>(null);
  const [activating, setActivating] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const side = useRef<HTMLElement>(null);
  const main = useRef<HTMLElement>(null);
  // On a narrow screen the menu is a drawer over the page, and behaves like a dialog.
  useDialogFocus(side, menuOpen, () => setMenuOpen(false));
  const [alertsSeenAt, setAlertsSeenAt] = useState(readSeenAt);

  // Saving, activating and deactivating make the server reconnect to the
  // relays and restart the daemon. The state read right after is a transient
  // one, so it is read again a few seconds later instead of at the next tick.
  const { refresh, refreshLive } = data;
  const refreshAndSettle = useCallback(async () => {
    await refresh();
    for (const delay of [4000, 10_000]) window.setTimeout(() => void refreshLive(), delay);
  }, [refresh, refreshLive]);

  const editor = useCommunityEditor(refreshAndSettle);
  const simulator = useSimulator();
  const identity = useIdentity({
    onChanged: refreshAndSettle,
    onError: (text) => setFlash({ tone: 'bad', text }),
    onImported: (npub) => setFlash({ tone: 'good', text: `Identidad importada: ${shortKey(npub)}` }),
  });
  const { loaded: configLoaded, load: loadConfig } = editor;

  // The rules are read once. If that read failed, it is tried again until it works.
  useEffect(() => {
    if (configLoaded) return;
    const timer = setInterval(() => void loadConfig(), 20_000);
    return () => clearInterval(timer);
  }, [configLoaded, loadConfig]);

  const communityName = (editor.serverConfig?.community.name || editor.draft.community.name || '').trim();
  const status = useMemo(() => marketStatus(daemon, data.settled), [daemon, data.settled]);
  const stats = useMemo(() => orderStats(orders), [orders]);
  const rulesRead: RulesRead = configLoaded ? 'loaded' : editor.loadFailed ? 'failed' : 'reading';
  // Until the editor has read the saved rules, the daemon report is what says whether there are any.
  const hasSavedConfig = configLoaded ? editor.serverConfig !== null : typeof daemon?.draft_revision === 'number';
  const steps = useMemo(() => setupSteps({ dashboard, daemon, backups, hasSavedConfig }), [dashboard, daemon, backups, hasSavedConfig]);
  const services = useMemo(() => serviceTiles(dashboard, daemon, orders, status, data.settled), [dashboard, daemon, orders, status, data.settled]);
  // The alerts carry the server's clock, so "seen" is kept in that clock too.
  const newestAlertAt = notifications[0]?.timestamp ?? 0;
  const unseenAlerts = notifications.filter((item) => (item.level === 'critical' || item.level === 'error') && item.timestamp > alertsSeenAt).length;
  // While the setup list is on the summary, the attention list does not repeat its steps.
  const duringSetup = isSettingUp(status);
  const attention = useMemo(
    () => attentionItems({ apiOnline: data.apiOnline || !data.settled, daemon, orders, backups, cardPublication, unseenAlerts, hasUnsavedChanges: editor.dirty, duringSetup }),
    [data.apiOnline, data.settled, daemon, orders, backups, cardPublication, unseenAlerts, editor.dirty, duringSetup],
  );
  // Only a recent announcement is what apps see now; an old one is history.
  const announced = daemon?.announced_fresh ? (daemon.announced ?? null) : null;
  const rules = useMemo(() => (announced ? announcedRules(announced.tags) : editor.serverConfig ? savedRules(editor.serverConfig) : []), [announced, editor.serverConfig]);
  const mostroActive = isActive(daemon);

  // Having the alerts page open is reading them, including the ones that arrive meanwhile.
  useEffect(() => {
    if (page !== 'alerts' || newestAlertAt <= alertsSeenAt) return;
    setAlertsSeenAt(newestAlertAt);
    try {
      localStorage.setItem(ALERTS_SEEN_KEY, String(newestAlertAt));
    } catch {
      // Without storage the badge simply comes back after a reload.
    }
  }, [page, newestAlertAt, alertsSeenAt]);

  useEffect(() => {
    setMenuOpen(false);
  }, [route]);

  // A new page starts with the keyboard at its content, not back at the menu.
  const shownPage = useRef(page);
  useEffect(() => {
    if (shownPage.current === page) return;
    shownPage.current = page;
    const timer = setTimeout(() => main.current?.focus({ preventScroll: true }), 0);
    return () => clearTimeout(timer);
  }, [page]);

  // The drawer only exists on narrow screens: widening the window puts the menu back in its column.
  useEffect(() => {
    const wide = window.matchMedia('(min-width: 861px)');
    const onChange = () => { if (wide.matches) setMenuOpen(false); };
    wide.addEventListener('change', onChange);
    return () => wide.removeEventListener('change', onChange);
  }, []);

  useEffect(() => {
    document.title = `${navItem(page).label} · ${communityName || 'Mostro Community Manager'}`;
  }, [page, communityName]);

  // A confirmation goes away on its own; an error stays until it is read.
  useEffect(() => {
    if (flash?.tone !== 'good') return;
    const timer = setTimeout(() => setFlash(null), 7000);
    return () => clearTimeout(timer);
  }, [flash]);

  const refreshAll = async () => {
    await Promise.all([data.refresh(), editor.load()]);
  };

  // What stopping the daemon would interrupt, as far as the panel can tell. The
  // public state of an order is coarse and the monitor can be behind: say so.
  const openWork = () => {
    const parts: string[] = [];
    if (stats.inProgress > 0) parts.push(stats.inProgress === 1 ? '1 orden en curso' : `${stats.inProgress} órdenes en curso`);
    if (stats.openDisputes > 0) parts.push(stats.openDisputes === 1 ? '1 disputa abierta' : `${stats.openDisputes} disputas abiertas`);
    const seen = parts.length > 0 ? `Según los relays hay ${parts.join(' y ')}. Mientras el daemon esté parado nadie podrá continuarlas.` : '';
    const behind = !orders || orders.state !== 'live' || orders.is_stale;
    const caveat = behind
      ? 'El panel no está al día con los relays: puede haber operaciones que no ve.'
      : 'Una orden «Publicada» puede estar ya tomada: el panel no lo distingue.';
    return [seen, caveat].filter(Boolean).join('\n\n');
  };

  const activate = async () => {
    const question = mostroActive
      ? `¿Aplicar las reglas guardadas y reiniciar el daemon?\n\n${openWork()}`
      : status.state === 'lingering'
        ? '¿Activar Mostro con las reglas guardadas?\n\nEl nodo aún figura anunciado en los relays. Si no acabas de desactivarlo aquí, otra instancia de Mostro usa esta misma clave: no actives una segunda.'
        : '¿Activar Mostro con las reglas guardadas?\n\nEl nodo empezará a anunciarse en los relays y a aceptar órdenes.';
    if (!window.confirm(question)) return;
    setActivating(true);
    try {
      await api('/api/daemon/activate', { method: 'PUT' });
      setFlash({ tone: 'good', text: mostroActive ? 'Reglas aplicadas. El daemon se está reiniciando.' : 'Mostro activado. El nodo tarda unos minutos en anunciarse en los relays.' });
      await refreshAndSettle();
    } catch (err) {
      setFlash({ tone: 'bad', text: err instanceof Error ? err.message : 'No se pudo activar Mostro.' });
    } finally {
      setActivating(false);
    }
  };

  const deactivate = async () => {
    const question = `¿Desactivar Mostro?\n\nEl daemon se detiene y deja de atender a las apps hasta que lo actives de nuevo.\n\n${openWork()}`;
    if (!window.confirm(question)) return;
    setActivating(true);
    try {
      await api('/api/daemon/deactivate', { method: 'PUT' });
      setFlash({ tone: 'good', text: 'Mostro desactivado. Las apps pueden seguir viendo el nodo como activo unos minutos, hasta que caduque su último anuncio.' });
      await refreshAndSettle();
    } catch (err) {
      setFlash({ tone: 'bad', text: err instanceof Error ? err.message : 'No se pudo desactivar Mostro.' });
    } finally {
      setActivating(false);
    }
  };

  const badgeFor = (id: PageId): { text: string; tone: string; label: string } | null => {
    if (id === 'disputes' && stats.openDisputes > 0) return { text: String(stats.openDisputes), tone: 'bad', label: `${stats.openDisputes} abiertas` };
    if (id === 'alerts' && unseenAlerts > 0) return { text: String(unseenAlerts), tone: 'bad', label: `${unseenAlerts} sin leer` };
    if (id === 'node' && (daemon?.warnings.length ?? 0) > 0) return { text: '', tone: 'warn', label: 'con avisos' };
    if (id === 'connect' && cardAttention(cardPublication)) return { text: '', tone: 'warn', label: 'la tarjeta no está publicada como pediste' };
    if (id === 'config' && editor.dirty) return { text: '', tone: 'info', label: 'cambios sin guardar' };
    if (id === 'backups' && backups?.last_error && backups.enabled) return { text: '', tone: 'warn', label: 'el último falló' };
    return null;
  };

  // A link of the menu also closes the drawer, even when it leads to the page that is already open.
  const sideLink = (target: PageId) => {
    const link = pageLink(navigate, target);
    return {
      href: link.href,
      onClick: (event: MouseEvent) => {
        link.onClick(event);
        setMenuOpen(false);
      },
    };
  };

  const group = navGroupOf(page);
  const current = navItem(page);
  const version = health?.version && health.version !== '0.1.0' ? `v${health.version}` : null;

  return (
    <div className={`shell ${menuOpen ? 'menu-open' : ''}`}>
      <a className="skip-link" href="#contenido" onClick={(event) => { event.preventDefault(); main.current?.focus(); }}>Saltar al contenido</a>
      <aside className="side" aria-label="Menú" ref={side}>
        <div className="side-brand">
          <img className="brand-mark" src="/brand-mark.png" alt="" aria-hidden="true" />
          <div className="brand-copy"><b>MOSTRO</b><span>COMMUNITY MANAGER</span></div>
          <button type="button" className="side-close" aria-label="Cerrar el menú" onClick={() => setMenuOpen(false)}><Icon name="x" size={18} /></button>
        </div>
        <a className="side-community" {...sideLink('home')}>
          <span className="side-community-mark">{(communityName || 'MC').slice(0, 2).toUpperCase()}</span>
          <span className="side-community-text">
            <b>{communityName || 'Mi comunidad'}</b>
            <small>{status.label}</small>
          </span>
        </a>
        <nav className="side-nav" aria-label="Navegación principal">
          {NAV_GROUPS.map((navGroup) => (
            <div className="side-group" key={navGroup.id}>
              <span className="side-group-label">{navGroup.label}</span>
              {navGroup.items.map((item) => {
                const badge = badgeFor(item.id);
                return (
                  <a key={item.id} {...sideLink(item.id)} className={`side-link ${page === item.id ? 'active' : ''}`} aria-current={page === item.id ? 'page' : undefined} title={item.description}>
                    <Icon name={item.icon} size={17} />
                    <span>{item.label}</span>
                    {badge && (
                      <span className={`side-badge tone-${badge.tone} ${badge.text ? '' : 'is-dot'}`}>
                        {badge.text}
                        <span className="sr-only"> {badge.label}</span>
                      </span>
                    )}
                  </a>
                );
              })}
            </div>
          ))}
        </nav>
        <div className="side-foot">
          <a className={`side-status tone-${status.tone}`} {...sideLink('node')} title="Ver el nodo">
            <i aria-hidden="true" />
            <span>
              <b>{status.label}</b>
              <small>{data.apiOnline ? 'Panel conectado' : data.settled ? 'Panel sin respuesta' : 'Consultando…'}</small>
            </span>
          </a>
          <span className="side-version">
            {version ? `Manager ${version}` : 'Manager en desarrollo'}
            {health?.mostro_pinned_version ? ` · mostrod ${health.mostro_pinned_version}` : ''}
          </span>
          {onLogout && (
            <button
              type="button"
              className="side-logout"
              onClick={() => void onLogout().catch((err) => setFlash({ tone: 'bad', text: err instanceof Error ? err.message : 'No se pudo cerrar la sesión.' }))}
            >
              Cerrar sesión
            </button>
          )}
        </div>
      </aside>
      <button type="button" className="side-scrim" aria-label="Cerrar el menú" tabIndex={menuOpen ? 0 : -1} onClick={() => setMenuOpen(false)} />

      <main className="main" id="contenido" tabIndex={-1} ref={main}>
        <header className="topbar">
          <button type="button" className="menu-button" aria-label="Abrir el menú" aria-expanded={menuOpen} onClick={() => setMenuOpen(true)}><Icon name="menu" size={19} /></button>
          <nav className="breadcrumb" aria-label="Dónde estás">
            <span>{group.label}</span>
            <Icon name="chevron" size={13} />
            <b>{current.label}</b>
          </nav>
          <div className="top-actions">
            {health && health.mode !== 'release' && <span className="environment-pill"><i /> Desarrollo</span>}
            <a className={`market-pill tone-${status.tone}`} {...pageLink(navigate, 'node')} title="Estado del mercado. Abre la página del nodo.">
              <i aria-hidden="true" /> {status.label}
            </a>
          </div>
        </header>

        {flash && (
          <div className={`flash tone-${flash.tone}`} role={flash.tone === 'bad' ? 'alert' : 'status'}>
            <span>{flash.text}</span>
            <button type="button" aria-label="Cerrar el aviso" onClick={() => setFlash(null)}><Icon name="x" size={15} /></button>
          </div>
        )}
        {data.settled && !data.apiOnline && page !== 'home' && (
          <div className="content content-notice">
            <Callout tone="bad" title="El panel no recibe respuesta de su servidor" action={<button type="button" className="button button-secondary" onClick={() => void refreshAll()}>Reintentar</button>}>
              Lo que ves puede estar desactualizado.
            </Callout>
          </div>
        )}

        {page === 'home' && (
          <HomePage
            data={{ ...data, refresh: refreshAll }}
            communityName={communityName}
            status={status}
            attention={attention}
            steps={steps}
            services={services}
            stats={stats}
            rules={rules}
            rulesAnnounced={Boolean(announced)}
            savedConfig={editor.serverConfig}
            rulesRead={rulesRead}
            activating={activating}
            onActivate={() => void activate()}
            navigate={navigate}
          />
        )}
        {page === 'orders' && <OrdersPage initialStatus={route.param} navigate={navigate} />}
        {page === 'disputes' && <MediationConsole initialOrderId={route.param} onSelectOrder={(orderId, options) => navigate('disputes', orderId, options)} />}
        {page === 'node' && (
          <NodePage data={data} status={status} savedConfig={editor.serverConfig} rulesRead={rulesRead} identity={identity} activating={activating} onActivate={() => void activate()} onDeactivate={() => void deactivate()} navigate={navigate} />
        )}
        {page === 'lightning' && <LiquidityOperationsPage lightning={dashboard?.lightning} maxTradeSats={editor.serverConfig?.market.max_trade_sats} rulesRead={rulesRead} navigate={navigate} />}
        {page === 'connect' && <ConnectPage data={data} navigate={navigate} />}
        {page === 'config' && <ConfigPage editor={editor} loading={data.loading} mostroActive={mostroActive} navigate={navigate} />}
        {page === 'backups' && <BackupsPage data={data} />}
        {page === 'alerts' && <AlertsPage notifications={notifications} read={data.notificationsRead} settled={data.settled} navigate={navigate} />}
        {page === 'simulator' && <SimulatorPage simulator={simulator} />}
      </main>

      <IdentityModals
        identity={identity}
        onContinue={() => {
          setFlash({ tone: 'good', text: 'Identidad lista. Ahora define las reglas de tu comunidad y guárdalas.' });
          navigate('config');
        }}
      />
    </div>
  );
}

export default App;
