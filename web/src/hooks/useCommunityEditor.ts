import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CommissionBondPreset, CommunityReply, Configuration } from '../types';
import { api } from '../lib/api';
import { blankConfig } from '../lib/constants';
import { percent } from '../lib/format';

export type FeeInputs = { fee: string; devFee: string; routingFee: string; bond: string };

const EMPTY_FEES: FeeInputs = { fee: '', devFee: '', routingFee: '', bond: '' };
const feeInputsOf = (config: Configuration): FeeInputs => ({
  fee: percent(config.market.fee_bps),
  devFee: percent(config.market.dev_fee_bps),
  routingFee: percent(config.market.max_routing_fee_bps),
  bond: percent(config.safety.bond_bps),
});

/**
 * The community rules being edited. The draft lives here, above the pages, so
 * moving around the panel does not lose what the operator has typed.
 */
export function useCommunityEditor(onSaved: () => void | Promise<void>) {
  const [revision, setRevision] = useState(0);
  const [draft, setDraft] = useState<Configuration>(blankConfig);
  /** What the server holds, as last read or saved. */
  const [serverConfig, setServerConfig] = useState<Configuration | null>(null);
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [loaded, setLoaded] = useState(false);
  /** The saved rules have never been read and the last attempt failed. */
  const [loadFailed, setLoadFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState('');
  // Shown next to the templates, where it was caused; `notice` goes next to the save button.
  const [presetNotice, setPresetNotice] = useState('');
  const [error, setError] = useState('');
  const [feeInputs, setFeeInputs] = useState<FeeInputs>(EMPTY_FEES);
  // Read inside `load`, which must not change identity on every keystroke.
  const dirtyRef = useRef(false);
  const loadedRef = useRef(false);

  const markDirty = useCallback(() => { dirtyRef.current = true; setDirty(true); setSaved(false); }, []);

  /**
   * Reads the saved rules. Unsaved edits are kept unless `force` is set: a
   * background refresh must never throw away what is being typed.
   */
  const load = useCallback(async (options?: { force?: boolean }) => {
    try {
      const data = await api<CommunityReply>('/api/community');
      setServerConfig(data.config);
      // Whatever was typed before the saved rules arrived was typed into an
      // empty form with no revision: the first answer always wins.
      const first = !loadedRef.current;
      loadedRef.current = true;
      setLoaded(true);
      setLoadFailed(false);
      if (dirtyRef.current && !options?.force && !first) return;
      dirtyRef.current = false;
      setRevision(data.revision); setSaved(Boolean(data.config)); setDirty(false);
      const config = data.config || blankConfig();
      setDraft(config);
      setFeeInputs(data.config ? feeInputsOf(config) : EMPTY_FEES);
      setError('');
    } catch (err) {
      // A failed refresh does not unload what is already on screen.
      if (!loadedRef.current) { setLoadFailed(true); setError(err instanceof Error ? err.message : 'No se pudo leer la configuración.'); }
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  // Closing or reloading the tab would lose unsaved edits without a word.
  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  const reloadSafely = () => {
    if (dirty && !window.confirm('Hay cambios sin guardar. ¿Descartarlos y recargar la configuración?')) return;
    setNotice(''); setPresetNotice('');
    void load({ force: true });
  };

  const updateCommunity = (key: keyof Configuration['community'], value: string) => { markDirty(); setDraft((old) => ({ ...old, community: { ...old.community, [key]: value } })); };
  const updateMarket = (key: keyof Configuration['market'], value: unknown) => { markDirty(); setDraft((old) => ({ ...old, market: { ...old.market, [key]: value } })); };
  const updateSafety = (key: keyof Configuration['safety'], value: unknown) => { markDirty(); setDraft((old) => ({ ...old, safety: { ...old.safety, [key]: value } })); };

  const toggleCurrency = (code: string) => {
    markDirty();
    const exists = draft.market.fiat_currencies.includes(code);
    if (exists) {
      updateMarket('fiat_currencies', draft.market.fiat_currencies.filter((c) => c !== code));
    } else {
      if (draft.market.fiat_currencies.length >= 30) {
        setError(
          'Máximo 30 monedas fiat permitidas por Mostro'
        );
        return;
      }
      updateMarket('fiat_currencies', [...draft.market.fiat_currencies, code]);
    }
  };

  const removeRelay = (relay: string) => {
    markDirty();
    setDraft((old) => ({ ...old, nostr: { relays: old.nostr.relays.filter((other) => other !== relay) } }));
  };
  const addRelay = (value: string) => {
    const url = value.trim();
    if (url && !draft.nostr.relays.includes(url)) { markDirty(); setDraft((old) => ({ ...old, nostr: { relays: [...old.nostr.relays, url] } })); }
  };

  const updateFee = (key: keyof FeeInputs, marketKey: keyof Configuration['market'] | 'bond_bps', value: string) => {
    markDirty(); setFeeInputs((old) => ({ ...old, [key]: value }));
    const bps = Math.round(Number(value || 0) * 100);
    if (marketKey === 'bond_bps') updateSafety(marketKey, bps); else updateMarket(marketKey, bps);
  };

  const applyCommissionBondPreset = (preset: CommissionBondPreset) => {
    markDirty();
    setDraft((old) => ({
      ...old,
      market: {
        ...old.market,
        fee_bps: preset.feeBps,
        dev_fee_bps: preset.devFeeBps,
        max_routing_fee_bps: preset.maxRoutingFeeBps,
      },
      safety: {
        ...old.safety,
        bond_enabled: preset.bondEnabled,
        bond_bps: preset.bondBps,
        base_bond_sats: preset.baseBondSats,
        bond_apply_to: preset.bondApplyTo,
        automatic_timeout_slash: preset.automaticTimeoutSlash,
      },
    }));
    setFeeInputs({
      fee: percent(preset.feeBps),
      devFee: percent(preset.devFeeBps),
      routingFee: percent(preset.maxRoutingFeeBps),
      bond: percent(preset.bondBps),
    });
    setNotice('');
    setPresetNotice(`Plantilla aplicada: ${preset.nameEs}. Revisa los valores y guarda para conservarla.`);
  };

  const saveConfig = async (event: React.FormEvent) => {
    event.preventDefault(); setSaving(true); setNotice(''); setPresetNotice('');
    if (!loaded) { setSaving(false); setError('Recarga la configuración para obtener una revisión válida antes de guardar.'); return; }
    if ((['fee', 'devFee', 'routingFee'] as const).some((key) => feeInputs[key] === '') || (draft.safety.bond_enabled && feeInputs.bond === '')) {
      setSaving(false); setError('Elige un porcentaje para cada comisión. Usa 0 si no deseas cobrarla.'); return;
    }
    if (draft.community.contact.trim()) {
      try { const contactUrl = new URL(draft.community.contact); if (!['http:', 'https:'].includes(contactUrl.protocol)) throw new Error(); }
      catch { setSaving(false); setError('El contacto debe ser una URL que empiece con http:// o https://.'); return; }
    }
    if (draft.community.picture.trim()) {
      try { if (new URL(draft.community.picture.trim()).protocol !== 'https:') throw new Error(); }
      catch { setSaving(false); setError('La imagen de la comunidad debe ser una URL que empiece con https://.'); return; }
    }
    try {
      const response = await api<CommunityReply>('/api/community', { method: 'PUT', body: JSON.stringify({ revision, config: draft }) });
      const config = response.config || draft;
      dirtyRef.current = false;
      setRevision(response.revision); setDraft(config); setServerConfig(config); setFeeInputs(feeInputsOf(config));
      setSaved(true); setDirty(false); setNotice('Configuración guardada.'); setError('');
      await onSaved();
    } catch (err) { setNotice(''); setError(err instanceof Error ? err.message : 'No se pudo guardar la configuración.'); }
    finally { setSaving(false); }
  };

  const activeCategories = useMemo(() => {
    const seen = new Set<string>();
    draft.payment_methods.forEach((m) => {
      const trimmed = m.category.trim();
      if (trimmed) seen.add(trimmed);
    });
    return Array.from(seen).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  }, [draft.payment_methods]);

  return {
    revision, draft, setDraft, serverConfig, saved, dirty, loaded, loadFailed, saving,
    notice, setNotice, presetNotice, error, setError, feeInputs, activeCategories,
    markDirty, load, reloadSafely, updateCommunity, updateMarket, updateSafety,
    toggleCurrency, addRelay, removeRelay, updateFee, applyCommissionBondPreset, saveConfig,
  };
}

export type CommunityEditor = ReturnType<typeof useCommunityEditor>;
