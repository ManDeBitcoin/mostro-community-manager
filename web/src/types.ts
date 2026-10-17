// Shapes of the API responses the panel reads.
export type Configuration = {
  community: { name: string; about: string; website: string; picture: string; contact: string; language: string };
  market: { fiat_currencies: string[]; min_trade_sats: number; max_trade_sats: number; fee_bps: number; dev_fee_bps: number; max_routing_fee_bps: number };
  safety: { bond_enabled: boolean; bond_bps: number; base_bond_sats: number; bond_apply_to: 'make' | 'take' | 'both'; automatic_timeout_slash: boolean; pow: number; pow_first_contact: number };
  nostr: { relays: string[] };
  payment_methods: { id: string; label: string; category: string; active: boolean }[];
};
export type ServiceInfo = {
  status: string;
  detail: string;
  version?: string;
  alias?: string | null;
  synced_to_chain?: boolean;
  synced_to_graph?: boolean | null;
  network?: string | null;
  /** `null` when LND did not say: unknown is not zero. */
  num_active_channels?: number | null;
  num_pending_channels?: number | null;
  num_inactive_channels?: number | null;
  block_height?: number | null;
  liquidity?: { status: 'available' | 'unavailable'; local_balance_sats: string | null; remote_balance_sats: string | null; detail: string };
};
export type Dashboard = {
  mostro: ServiceInfo;
  lightning: ServiceInfo;
  bitcoin: ServiceInfo;
  market_started: boolean;
};
export type CommunityCard = {
  version: number;
  name: string;
  pubkey: string;
  relays: string[];
  currency: string;
  payment_methods: string[];
  fee_bps: number;
  bond_percent: number;
  website: string;
  contact: string;
  signature: string;
};

export type ConnectionInfo = {
  status: string;
  npub?: string | null;
  pubkey_hex?: string | null;
  relays: string[];
  nprofile?: string | null;
  nostr_uri?: string | null;
  qr_svg?: string | null;
  qr_json_svg?: string | null;
  json_uri?: string | null;
  card_uri?: string | null;
  qr_card_svg?: string | null;
  card?: CommunityCard | null;
  /** Why there is no card, when the node has an identity and still has none. */
  card_unavailable?: string | null;
  app_download_url: string;
  instructions: string;
};
export type CardRelayReport = {
  url: string;
  /** What the relay did with the event: only `accepted` means it has it. */
  outcome: 'accepted' | 'rejected' | 'no_answer' | 'unreachable' | 'pending';
  /** The relay's own words, or the connection error. */
  detail?: string | null;
  at: number;
};
/** The community card as an event on the node's relays, and what the relays answered. */
export type CardPublication = {
  /** The switch as saved. */
  enabled: boolean;
  /** The server has not acted on the last change yet: the rest is about the previous one. */
  working: boolean;
  state: 'off' | 'blocked' | 'published' | 'partial' | 'failed' | 'withdrawing' | 'withdrawn' | 'withdrawal_incomplete';
  reason?: string | null;
  reason_text?: string | null;
  event_id?: string | null;
  /** `created_at` of the event: when the card last changed. */
  card_changed_at?: number | null;
  withdrawn_at?: number | null;
  relays: CardRelayReport[];
  /** Relays out of the configuration that may still hold an earlier revision. */
  former_relays?: string[];
  last_attempt_at?: number | null;
  next_attempt_at?: number | null;
  resend_every_secs: number;
};
export type DaemonReport = {
  state: 'unconfigured' | 'configured_standby' | 'active_ready' | 'active_running';
  identity_present: boolean;
  npub?: string | null;
  draft_revision?: number | null;
  active_revision?: number | null;
  active_settings_hash?: string | null;
  active_settings_path?: string | null;
  lnd_channel_count: number | null;
  lnd_synced: boolean | null;
  can_activate: boolean;
  warnings: string[];
  /** The same warnings with a stable code each. Absent on an older server. */
  notices?: DaemonNotice[];
  /** Seconds the current mostrod process has been running, when known. */
  running_for_secs?: number | null;
  mostro_version?: string;
  version_source?: 'announced' | 'binary' | 'pinned' | string;
  packaged_version?: string;
  protocol_version?: number;
  announced?: NodeInfo | null;
  announced_age_secs?: number | null;
  /** Recent and, when the age of the running process is known, published by it. */
  announced_fresh?: boolean;
  /** Recent, but older than the process running now: the previous one published it. */
  announced_before_start?: boolean;
  last_exit?: { at_unix: number; code: number; uptime_secs: number } | null;
  /** Orders not finished in the daemon's database; null without a database. */
  open_orders?: { in_flight: number; pending: number; by_status: Record<string, number> } | null;
};
export type DaemonNotice = { code: string; text: string };
export type NodeInfo = {
  event_id: string;
  created_at: number;
  name?: string | null;
  mostro_version?: string | null;
  protocol_version?: string | null;
  tags: Record<string, string>;
};
export type DisputeSummary = {
  id: string;
  event_id: string;
  status: string;
  initiator?: string | null;
  published_at?: number | null;
  updated_at: number;
};
export type DisputeView = DisputeSummary & { order_id?: string | null; is_open: boolean };
export type SimulationScenarioInfo = {
  id: string;
  label: string;
  description: string;
};
export type SimulationStep = {
  step_number: number;
  action_code: string;
  title: string;
  actor: 'seller' | 'buyer' | 'mostro' | 'solver';
  order_status: string;
  description: string;
  nostr_event?: {
    kind: number;
    event_id: string;
    sender: string;
    recipient?: string | null;
    summary: string;
  } | null;
  lightning_action?: {
    action: string;
    amount_sats: number;
    payment_hash: string;
    status: string;
  } | null;
};
export type SimulationReport = {
  scenario: string;
  order_id: string;
  bot_npub: string;
  payment_method: string;
  financials: {
    trade_amount_sats: number;
    fiat_currency: string;
    fiat_amount: string;
    seller_bond_sats: number;
    buyer_bond_sats: number;
    total_mostro_fee_sats: number;
    fee_per_side_sats: number;
    dev_fee_sats: number;
    fee_sats?: number;
    seller_hold_invoice_sats?: number;
    buyer_receives_sats?: number;
    seller_total_locked_sats: number;
    buyer_total_locked_sats: number;
  };
  steps: SimulationStep[];
  final_status: string;
  is_success: boolean;
  duration_simulated_ms: number;
  timestamp_unix: number;
  simulation_mode?: string;
  disclaimer?: string;
};
export type CommunityReply = { revision: number; config: Configuration | null };

export type NotificationItem = {
  id: string;
  level: string;
  category: string;
  title: string;
  message: string;
  timestamp: number;
  details?: Record<string, unknown>;
};

export type BackupEntry = {
  filename: string;
  path: string;
  size_bytes: number;
  modified_timestamp: number;
  /** `rules` (identity and rules) or `database` (the daemon's database). */
  kind?: 'rules' | 'database' | string;
};

export type AutoBackupState = {
  enabled: boolean;
  target_dir: string;
  interval_secs: number;
  retention_count: number;
  last_run_timestamp: number | null;
  last_run_success: boolean | null;
  last_error: string | null;
  last_backup_path: string | null;
  backups: BackupEntry[];
};

export type OrderSummary = {
  id: string;
  event_id: string;
  kind: string;
  status: string;
  fiat_code: string;
  /** One amount, the two ends of a range, or empty when the node announced none the panel can read. */
  fiat_amount_range: string[];
  amount_sats: number;
  amount_sats_str?: string;
  payment_methods: string[];
  premium: number;
  created_at: number;
  expires_at: number | null;
  published_at?: number | null;
};

export type RelayStatus = {
  url: string;
  state: 'unconfigured' | 'connecting' | 'syncing' | 'live' | 'degraded' | 'disconnected';
  last_error?: string | null;
};

export type OrdersSnapshot = {
  state: 'unconfigured' | 'connecting' | 'syncing' | 'live' | 'degraded' | 'disconnected';
  is_stale: boolean;
  last_update: number;
  source_npub: string | null;
  relays?: RelayStatus[];
  orders: OrderSummary[];
  disputes?: DisputeSummary[];
  node_info?: NodeInfo | null;
  node_info_age_secs?: number | null;
};

export type ChatMessage = {
  id: string;
  order_id: string;
  sender: string;
  recipient?: string | null;
  created_at: number;
  kind: number;
  action?: string | null;
  content: string;
  is_from_me: boolean;
  role?: 'daemon' | 'user' | 'admin' | string;
  variant?: string;
  dispute_id?: string | null;
  acknowledged?: boolean;
};

export type ChatHistory = {
  order_id: string;
  messages: ChatMessage[];
  count: number;
  dispute_id?: string | null;
};

export type CommissionBondPreset = {
  id: string;
  nameEs: string;
  descEs: string;
  feeBps: number;
  devFeeBps: number;
  maxRoutingFeeBps: number;
  bondEnabled: boolean;
  bondBps: number;
  baseBondSats: number;
  bondApplyTo: 'make' | 'take' | 'both';
  automaticTimeoutSlash: boolean;
};

export type LndChannel = {
  channel_point: string;
  remote_pubkey: string;
  alias?: string;
  capacity_sats: string;
  local_balance_sats: string;
  remote_balance_sats: string;
  active: boolean;
  private: boolean;
};

export type LndChannelsReport = {
  status: string;
  detail: string;
  total_capacity_sats: string;
  total_local_balance_sats: string;
  total_remote_balance_sats: string;
  num_active_channels: number;
  num_inactive_channels: number;
  is_mock: boolean;
  inbound_sufficient: boolean;
  channels: LndChannel[];
};
