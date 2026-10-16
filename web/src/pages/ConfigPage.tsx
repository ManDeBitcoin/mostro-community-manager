import { useEffect, useMemo, useRef, useState } from 'react';
import { CURRENCY_CODES, getCurrencyDisplayName } from '../currencies';
import type { CommunityEditor } from '../hooks/useCommunityEditor';
import type { Navigate } from '../lib/navigation';
import type { CommissionBondPreset } from '../types';
import { COMMISSION_BOND_PRESETS, POPULAR_CURRENCIES, configNavSections } from '../lib/constants';
import { formatBps, formatNumber } from '../lib/format';
import { Field, Icon, Toggle } from '../components/ui';
import { Callout, LinkButton, PageHeader } from '../components/layout';

type ConfigSection = (typeof configNavSections)[number]['id'];

const BOND_PAYER: Record<CommissionBondPreset['bondApplyTo'], string> = { make: 'quien publica', take: 'quien toma', both: 'quien publica y quien toma' };

/** Everything a template sets, spelled out: a template must not change a rule its card does not name. */
const presetFacts = (preset: CommissionBondPreset) =>
  [
    `Comisión ${formatBps(preset.feeBps)}`,
    `aporte al desarrollo ${formatBps(preset.devFeeBps)}`,
    `enrutamiento máximo ${formatBps(preset.maxRoutingFeeBps)}`,
    preset.bondEnabled ? `garantía ${formatBps(preset.bondBps)} con mínimo de ${formatNumber(preset.baseBondSats)} sats, la paga ${BOND_PAYER[preset.bondApplyTo]}` : 'sin garantía',
    preset.automaticTimeoutSlash ? 'penalización automática por vencimiento activada' : 'sin penalización automática',
  ].join(' · ');

const normalize = (text: string) => text.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();

export function ConfigPage({ editor, loading, mostroActive, navigate }: { editor: CommunityEditor; loading: boolean; mostroActive: boolean; navigate: Navigate }) {
  const {
    revision, draft, setDraft, saved, dirty, loaded, saving, notice, presetNotice, error, feeInputs, activeCategories,
    markDirty, reloadSafely, updateCommunity, updateMarket, updateSafety, toggleCurrency, removeRelay, updateFee, applyCommissionBondPreset, saveConfig,
  } = editor;
  const [activeConfigNav, setActiveConfigNav] = useState<ConfigSection>('identity');
  const [relayInput, setRelayInput] = useState('');
  const [currencySearchQuery, setCurrencySearchQuery] = useState('');
  const [isCurrencyDropdownOpen, setIsCurrencyDropdownOpen] = useState(false);
  const currencyDropdownRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (currencyDropdownRef.current && !currencyDropdownRef.current.contains(event.target as Node)) {
        setIsCurrencyDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
    };
  }, []);

  // The section list on the left follows the scroll position.
  useEffect(() => {
    const handleScroll = () => {
      const scrollPos = window.scrollY + 130;
      for (let i = configNavSections.length - 1; i >= 0; i--) {
        const item = configNavSections[i];
        const el = document.getElementById(item.id);
        if (el && scrollPos >= el.offsetTop) {
          setActiveConfigNav(item.id);
          break;
        }
      }
    };
    window.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll();
    return () => window.removeEventListener('scroll', handleScroll);
  }, []);

  const filteredCurrencies = useMemo(() => {
    const q = normalize(currencySearchQuery.trim());
    if (!q) {
      return CURRENCY_CODES.slice(0, 30);
    }
    return CURRENCY_CODES.filter((item) => item.code.toLowerCase().includes(q) || normalize(item.nameEn).includes(q) || normalize(item.nameEs).includes(q)).slice(0, 50);
  }, [currencySearchQuery]);

  const handleCurrencyKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (filteredCurrencies.length > 0) {
        toggleCurrency(filteredCurrencies[0].code);
        setCurrencySearchQuery('');
      }
    }
  };
  const addRelay = () => { editor.addRelay(relayInput); setRelayInput(''); };

  return (
    <section className="content config-page">
      <PageHeader
        group="Ajustes"
        title="Configuración"
        description="Las reglas de tu comunidad: quién es, qué monedas admite, cuánto cobra y cómo se protege."
        actions={
          <>
            <span className={`save-state ${saved ? 'is-saved' : ''}`}><i />{saved ? `Guardado · revisión ${revision}` : dirty ? 'Cambios sin guardar' : 'Sin guardar todavía'}</span>
            <button type="button" className="button button-secondary" onClick={reloadSafely} disabled={loading || saving}>Recargar</button>
          </>
        }
      />
      <Callout tone={mostroActive ? 'good' : 'info'} title={mostroActive ? 'Mostro está activado' : 'Mostro no está activado'} action={<LinkButton onClick={() => navigate('node')}>Ir al nodo</LinkButton>}>
        {mostroActive
          ? 'Al guardar, las reglas se aplican al nodo. El daemon se reinicia solo cuando cambia lo que lee.'
          : 'Al guardar, las reglas quedan listas. Empiezan a regir cuando actives Mostro desde la página del nodo.'}
      </Callout>
        <div className="preset-commissions-banner">
          <div className="preset-commissions-head">
            <span className="preset-commissions-badge">
              PLANTILLAS
            </span>
            <h3>
              Plantillas de comisiones y garantía
            </h3>
            <p>
              Cada plantilla fija de una vez todo lo que dice su tarjeta. No toca la identidad, los relays ni los métodos de pago, y nada se guarda hasta que pulses Guardar.
            </p>
          </div>
          <div className="preset-commissions-grid">
            {COMMISSION_BOND_PRESETS.map((p) => (
              <button
                key={p.id}
                type="button"
                className="preset-commission-card-btn"
                onClick={() => applyCommissionBondPreset(p)}
              >
                <strong>{p.nameEs}</strong>
                <span>{p.descEs}</span>
                <span className="preset-facts">{presetFacts(p)}</span>
                <span className="preset-apply-label">
                  Usar esta plantilla →
                </span>
              </button>
            ))}
          </div>
        </div>
        {presetNotice && <div className="form-message success" role="status" style={{ marginBottom: '14px' }}>{presetNotice}</div>}
        <div className="config-layout">
          <nav className="config-nav" aria-label="Secciones de configuración">
            {configNavSections.map((sec) => (
              <a
                key={sec.id}
                href={`#${sec.id}`}
                className={activeConfigNav === sec.id ? 'active' : ''}
                onClick={(e) => {
                  e.preventDefault();
                  setActiveConfigNav(sec.id);
                  const el = document.getElementById(sec.id);
                  if (el) {
                    el.scrollIntoView({ behavior: 'smooth', block: 'start' });
                  }
                }}
              >
                {sec.label}
              </a>
            ))}
          </nav>
          {/* Until the saved rules arrive the form is empty and has no revision: it is not for typing yet. */}
          <form className={`config-form ${loaded ? '' : 'is-waiting'}`} aria-busy={!loaded} onSubmit={saveConfig}>
            {!loaded && <p className="panel-note" role="status">{error ? 'No se pudieron leer las reglas guardadas. El panel lo vuelve a intentar solo.' : 'Leyendo las reglas guardadas…'}</p>}
            <section className="config-section" id="identity"><div className="config-section-head"><span className="section-number">01</span><div><h2>Identidad de la comunidad</h2><p>Esta información ayuda a las personas a reconocer tu instancia.</p></div></div><div className="form-grid"><Field label="Nombre de la comunidad"><input value={draft.community.name} onChange={(e) => updateCommunity('name', e.target.value)} placeholder="Ej. Bitcoin Quito" maxLength={80}/></Field><Field label="Idioma"><select value={draft.community.language} onChange={(e) => updateCommunity('language', e.target.value)}><option value="es">Español</option><option value="en">English</option><option value="pt">Português</option></select></Field><Field label="Sitio web"><input type="url" value={draft.community.website} onChange={(e) => updateCommunity('website', e.target.value)} placeholder="https://tu-comunidad.org"/></Field><Field label="Imagen de la comunidad" hint="Enlace HTTPS a una imagen cuadrada (se recomienda 128x128 px). Las apps la muestran junto al nombre del nodo"><input type="url" value={draft.community.picture} onChange={(e) => updateCommunity('picture', e.target.value)} placeholder="https://tu-comunidad.org/logo.png"/></Field><Field label="Contacto" hint="Dirección web pública de contacto"><input type="url" value={draft.community.contact} onChange={(e) => updateCommunity('contact', e.target.value)} placeholder="https://tu-comunidad.org/contacto"/></Field><div className="field full-width"><span className="field-label">Acerca de</span><textarea value={draft.community.about} onChange={(e) => updateCommunity('about', e.target.value)} placeholder="Describe brevemente tu comunidad" rows={3} maxLength={500}/><span className="field-hint">{draft.community.about.length}/500 caracteres</span></div></div></section>
            <section className="config-section" id="market"><div className="config-section-head"><span className="section-number">02</span><div><h2>Reglas del mercado</h2><p>Establece límites de operación y comisiones. Los porcentajes se convierten a puntos base al guardar.</p></div></div><div className="form-grid">
              <div className="field full-width">
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: '6px' }}>
                  <span className="field-label" style={{ margin: 0 }}>
                    Monedas fiat
                  </span>
                  <span className="field-hint" style={{ margin: 0 }}>
                    {draft.market.fiat_currencies.length}/30 seleccionadas
                  </span>
                </div>
                <div className="chips currency-selected-chips">
                  {draft.market.fiat_currencies.map((code) => {
                    const displayName = getCurrencyDisplayName(code, draft.community.language);
                    return (
                      <button
                        className="currency-chip-badge"
                        type="button"
                        key={code}
                        onClick={() => toggleCurrency(code)}
                        title={`Eliminar ${code} - ${displayName}`}
                      >
                        <strong>{code}</strong>
                        <span>{displayName}</span>
                        <b>×</b>
                      </button>
                    );
                  })}
                  {draft.market.fiat_currencies.length === 0 && (
                    <span className="empty-hint">
                      Sin monedas seleccionadas · elige una para habilitar operaciones
                    </span>
                  )}
                </div>
                <div className="currency-search-wrap" ref={currencyDropdownRef}>
                  <input
                    type="text"
                    aria-label="Buscar una moneda"
                    className="currency-search-input"
                    value={currencySearchQuery}
                    onChange={(e) => {
                      setCurrencySearchQuery(e.target.value);
                      setIsCurrencyDropdownOpen(true);
                    }}
                    onFocus={() => setIsCurrencyDropdownOpen(true)}
                    onKeyDown={handleCurrencyKeyDown}
                    placeholder={
                      'Buscar moneda por abreviación (USD, EUR) o nombre (Dólar, Peso)...'
                    }
                  />
                  {currencySearchQuery && (
                    <button
                      type="button"
                      className="currency-search-clear"
                      aria-label="Limpiar búsqueda"
                      onClick={() => {
                        setCurrencySearchQuery('');
                        setIsCurrencyDropdownOpen(true);
                      }}
                    >
                      ✕
                    </button>
                  )}
                  {isCurrencyDropdownOpen && (
                    <div className="currency-dropdown-menu">
                      <div className="currency-dropdown-header">
                        <span>
                          {currencySearchQuery.trim()
                            ? `Resultados para "${currencySearchQuery}" (${filteredCurrencies.length})`
                            : 'Selecciona monedas del catálogo oficial ISO 4217'}
                        </span>
                        <button
                          type="button"
                          className="currency-dropdown-close"
                          onClick={() => setIsCurrencyDropdownOpen(false)}
                        >
                          Cerrar
                        </button>
                      </div>
                      <div className="currency-dropdown-list">
                        {filteredCurrencies.length === 0 ? (
                          <div className="currency-dropdown-empty">
                            {`No se encontraron monedas que coincidan con "${currencySearchQuery}"`}
                          </div>
                        ) : (
                          filteredCurrencies.map((item) => {
                            const isSelected = draft.market.fiat_currencies.includes(item.code);
                            const primaryName = item.nameEs;
                            const secondaryName = item.nameEn;
                            return (
                              <button
                                type="button"
                                key={item.code}
                                className={`currency-dropdown-item ${isSelected ? 'selected' : ''}`}
                                aria-pressed={isSelected}
                                onClick={() => toggleCurrency(item.code)}
                              >
                                <span className="currency-code-tag">{item.code}</span>
                                <div className="currency-item-names">
                                  <span className="currency-primary-name">{primaryName}</span>
                                  <span className="currency-secondary-name">{secondaryName}</span>
                                </div>
                                <span className={`currency-item-action ${isSelected ? 'is-selected' : ''}`}>
                                  {isSelected
                                    ? '✓ Seleccionada'
                                    : '+ Añadir'}
                                </span>
                              </button>
                            );
                          })
                        )}
                      </div>
                    </div>
                  )}
                </div>
                <div className="currency-popular-row">
                  <span className="currency-popular-label">
                    Monedas populares:
                  </span>
                  {POPULAR_CURRENCIES.map((code) => {
                    const isSelected = draft.market.fiat_currencies.includes(code);
                    return (
                      <button
                        key={code}
                        type="button"
                        className={`currency-quick-pill ${isSelected ? 'active' : ''}`}
                        onClick={() => toggleCurrency(code)}
                        title={getCurrencyDisplayName(code, draft.community.language)}
                      >
                        {isSelected ? `✓ ${code}` : `+ ${code}`}
                      </button>
                    );
                  })}
                </div>
              </div>
              <Field label="Operación mínima" hint="Valor expresado en satoshis"><input type="number" min="0" value={draft.market.min_trade_sats} onChange={(e) => updateMarket('min_trade_sats', Number(e.target.value))}/></Field><Field label="Operación máxima" hint="Valor expresado en satoshis"><input type="number" min="0" value={draft.market.max_trade_sats} onChange={(e) => updateMarket('max_trade_sats', Number(e.target.value))}/></Field><Field label="Comisión total por operación" hint="Mostro cobra la mitad al comprador y la mitad al vendedor. 0% si no deseas cobrar"><div className="input-suffix"><input type="number" min="0" step="0.01" value={feeInputs.fee} required onChange={(e) => updateFee('fee', 'fee_bps', e.target.value)}/><span>%</span></div></Field><Field label="Aporte al desarrollo de Mostro" hint="Parte de tu comisión que el nodo envía al fondo de desarrollo, solo en mainnet. Entre 10% y 100%"><div className="input-suffix"><input type="number" min="10" max="100" step="0.01" value={feeInputs.devFee} required onChange={(e) => updateFee('devFee', 'dev_fee_bps', e.target.value)}/><span>%</span></div></Field><Field label="Comisión máxima de enrutamiento" hint="Tope que el nodo acepta pagar a la red al enviar los sats al comprador. Con 0% solo salen pagos por rutas gratuitas; Mostro usa 0,2% por defecto"><div className="input-suffix"><input type="number" min="0" step="0.01" value={feeInputs.routingFee} required onChange={(e) => updateFee('routingFee', 'max_routing_fee_bps', e.target.value)}/><span>%</span></div></Field></div></section>
            <section className="config-section" id="safety"><div className="config-section-head"><span className="section-number">03</span><div><h2>Seguridad y garantías</h2><p>Garantías y prueba de trabajo para frenar el abuso.</p></div></div><Toggle checked={draft.safety.bond_enabled} onChange={(v) => updateSafety('bond_enabled', v)} label="Exigir garantía en las operaciones" note="Cada parte retiene unos sats mientras dura la operación. Se le devuelven si todo va bien."/>{draft.safety.bond_enabled && <div className="form-grid compact-grid"><Field label="Garantía en porcentaje" hint="Porcentaje del valor de la operación. Se aplica el mayor entre este y la garantía mínima"><div className="input-suffix"><input type="number" min="0" step="0.01" value={feeInputs.bond} required onChange={(e) => updateFee('bond', 'bond_bps', e.target.value)}/><span>%</span></div></Field><Field label="Garantía mínima" hint="Suelo en satoshis: no se suma al porcentaje"><input type="number" min="0" value={draft.safety.base_bond_sats} onChange={(e) => updateSafety('base_bond_sats', Number(e.target.value))}/></Field><Field label="Quién la paga"><select value={draft.safety.bond_apply_to} onChange={(e) => updateSafety('bond_apply_to', e.target.value)}><option value="both">Quien publica y quien toma</option><option value="make">Quien publica</option><option value="take">Quien toma</option></select></Field></div>}<div className="toggle-divider"/><Toggle checked={draft.safety.automatic_timeout_slash} onChange={(v) => updateSafety('automatic_timeout_slash', v)} label="Penalización automática por vencimiento" note="Ejecuta la garantía de quien deja vencer un plazo de espera. No se aplica a todos los plazos de una operación."/><div className="form-grid compact-grid pow-grid"><Field label="Prueba de trabajo" hint="Dificultad general"><input type="number" min="0" value={draft.safety.pow} onChange={(e) => updateSafety('pow', Number(e.target.value))}/></Field><Field label="Primera conversación" hint="Dificultad del primer mensaje de cada clave. Si supera la general, los clientes que no la calculan no reciben respuesta, tampoco los de mediación"><input type="number" min="0" value={draft.safety.pow_first_contact} onChange={(e) => updateSafety('pow_first_contact', Number(e.target.value))}/></Field></div></section>
            <section className="config-section" id="nostr"><div className="config-section-head"><span className="section-number">04</span><div><h2>Relays de Nostr</h2><p>Define los relays que usará tu instancia para publicar y recibir eventos.</p></div></div><div className="chips">{draft.nostr.relays.map((relay) => <button className="chip relay-chip" type="button" key={relay} title={`Quitar ${relay}`} onClick={() => removeRelay(relay)}><span>{relay}</span><b aria-hidden="true">×</b><span className="sr-only"> Quitar</span></button>)}{draft.nostr.relays.length === 0 && <span className="empty-hint">Aún no has añadido relays.</span>}</div><div className="inline-add"><input type="url" aria-label="Dirección del relay" value={relayInput} onChange={(e) => setRelayInput(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), addRelay())} placeholder="wss://relay.example.org"/><button type="button" onClick={addRelay}>Añadir relay</button></div></section>
            <section className="config-section" id="payments">
              <div className="config-section-head"><span className="section-number">05</span><div><h2>Métodos de pago</h2><p>Activa las opciones de pago que tu comunidad puede ofrecer.</p></div></div>
              {activeCategories.length > 0 && (
                <div className="payment-summary-bar">
                  <span className="payment-summary-title">Categorías en uso ({activeCategories.length}):</span>
                  <div className="payment-summary-chips">
                    {activeCategories.map((cat) => {
                      const count = draft.payment_methods.filter((m) => m.category.trim().toLowerCase() === cat.toLowerCase()).length;
                      return (
                        <span key={cat} className="payment-summary-badge" title={`${count} método(s) en esta categoría`}>
                          <span className="summary-dot" />
                          {cat} <span className="summary-count">{count}</span>
                        </span>
                      );
                    })}
                  </div>
                </div>
              )}
              {draft.payment_methods.length === 0 && <p className="empty-hint payment-empty">No hay métodos de pago añadidos. Añade solo las opciones que tu comunidad admite.</p>}
              <div className="payment-list">
                {draft.payment_methods.map((method, index) => (
                  <div className="payment-row" key={index}>
                    <span className="payment-mark">{method.label.slice(0, 1) || '·'}</span>
                    <div className="payment-col-main">
                      <div className="payment-edit">
                        <input
                          aria-label="Identificador del método"
                          required
                          value={method.id}
                          placeholder="id"
                          onChange={(e) => {
                            markDirty();
                            setDraft((old) => ({ ...old, payment_methods: old.payment_methods.map((item, i) => i === index ? { ...item, id: e.target.value } : item) }));
                          }}
                        />
                        <input
                          aria-label="Nombre del método"
                          required
                          value={method.label}
                          placeholder="Nombre"
                          onChange={(e) => {
                            markDirty();
                            const newLabel = e.target.value;
                            setDraft((old) => ({
                              ...old,
                              payment_methods: old.payment_methods.map((item, i) => {
                                if (i !== index) return item;
                                const updated = { ...item, label: newLabel };
                                if (!item.id || item.id === item.label.toLowerCase().trim().replace(/\s+/g, '-').replace(/[^a-z0-9-_]/g, '')) {
                                  updated.id = newLabel.toLowerCase().trim().replace(/\s+/g, '-').replace(/[^a-z0-9-_]/g, '');
                                }
                                return updated;
                              }),
                            }));
                          }}
                        />
                        <input
                          aria-label="Categoría del método"
                          required
                          value={method.category}
                          placeholder="Categoría"
                          onChange={(e) => {
                            markDirty();
                            setDraft((old) => ({ ...old, payment_methods: old.payment_methods.map((item, i) => i === index ? { ...item, category: e.target.value } : item) }));
                          }}
                        />
                      </div>
                      {activeCategories.length > 0 && (
                        <div className="payment-category-tags">
                          <span className="payment-category-label">Tags activos:</span>
                          {activeCategories.map((cat) => {
                            const isSelected = method.category.trim().toLowerCase() === cat.toLowerCase();
                            return (
                              <button
                                type="button"
                                key={cat}
                                className={`category-tag-pill ${isSelected ? 'selected' : ''}`}
                                title={isSelected ? `Categoría asignada: ${cat}` : `Asignar "${cat}" a este método`}
                                onClick={() => {
                                  markDirty();
                                  setDraft((old) => ({
                                    ...old,
                                    payment_methods: old.payment_methods.map((item, i) =>
                                      i === index ? { ...item, category: cat } : item
                                    ),
                                  }));
                                }}
                              >
                                {isSelected ? '✓ ' : '+ '}{cat}
                              </button>
                            );
                          })}
                        </div>
                      )}
                    </div>
                    <span className={`payment-state ${method.active ? 'enabled' : ''}`}>{method.active ? 'Activo' : 'Inactivo'}</span>
                    <button
                      type="button"
                      role="switch"
                      aria-label={`${method.active ? 'Desactivar' : 'Activar'} ${method.label || 'método de pago'}`}
                      aria-checked={method.active}
                      className={`toggle small ${method.active ? 'on' : ''}`}
                      onClick={() => {
                        markDirty();
                        setDraft((old) => ({ ...old, payment_methods: old.payment_methods.map((item, i) => i === index ? { ...item, active: !item.active } : item) }));
                      }}
                    >
                      <i/>
                    </button>
                    <button
                      type="button"
                      className="remove-payment"
                      aria-label={`Eliminar ${method.label || 'método de pago'}`}
                      onClick={() => {
                        markDirty();
                        setDraft((old) => ({ ...old, payment_methods: old.payment_methods.filter((_, i) => i !== index) }));
                      }}
                    >
                      ×
                    </button>
                  </div>
                ))}
              </div>
              <button
                type="button"
                className="add-payment"
                onClick={() => {
                  markDirty();
                  setDraft((old) => ({ ...old, payment_methods: [...old.payment_methods, { id: '', label: '', category: '', active: true }] }));
                }}
              >
                + Añadir método de pago
              </button>
            </section>
            {(error || notice) && <div className={`form-message ${error ? 'error' : 'success'}`} role="status">{error || notice}</div>}
            <div className="form-footer"><div className="footer-copy"><span className="secure-icon"><Icon name="check" size={13}/></span><span>Los cambios se guardan en el servidor local.</span></div><button className="button button-primary save-button" type="submit" disabled={saving || loading || !loaded}><Icon name="save" size={16}/>{saving ? 'Guardando…' : 'Guardar configuración'}</button></div>
          </form>
        </div>
    </section>
  );
}
