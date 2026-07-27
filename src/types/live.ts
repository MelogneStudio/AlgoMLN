// Wire types for Phase-7 live execution. Mirrors the `#[serde(rename_all =
// "camelCase")]` shapes in `src/commands/live.rs` and `src/live/session.rs`.
// Keep these in sync — there is no codegen pipeline, so a Rust field rename
// without a TS-side update will surface as `undefined` at runtime.

export interface RequestLiveStartResult {
  /** Opaque single-use confirmation token. Pass back to confirmLiveStart
   *  within its TTL (90 s). */
  token: string;
  /** True when the user has not yet acknowledged live-trading risks.
   *  Surface an ack dialog and call acknowledgeLiveTrading before
   *  confirmLiveStart will return. */
  requiresAck: boolean;
  /** The symbol the strategy will trade. Echoed back so the UI can show it
   *  in the confirmation dialog alongside the strategy name. */
  symbol: string;
}

export interface StopResult {
  stopped: boolean;
  /** Warning about state the user must handle manually.
   *  e.g. "5 open positions and 2 pending orders remain in NIFTY —
   *  close them manually in your broker app." */
  openPositionsWarning: string | null;
}

export type LiveStatus =
  | 'Starting'
  | 'Running'
  | 'Paused'
  | 'Stopped'
  | 'Failed';

export interface LiveStatusWire {
  strategyId: string;
  strategyName: string;
  symbol: string;
  status: LiveStatus;
  /** Populated only when `status === 'Failed'`. */
  failReason: string | null;
  /** ISO-8601 timestamp from `chrono::Utc::now().to_rfc3339()`. */
  startTime: string;
  positionCount: number;
  /** Non-negative magnitude of session-realized losses in rupees. */
  realizedLoss: number;
  /** True when the broker's realized-loss cache has failed too many
   *  consecutive refreshes. The session pauses until the cache recovers. */
  lossTrackingStale: boolean;
}

/** One entry in the immutable JSONL trade log. The Rust side appends one
 *  per successful order placement (from DhanBroker::execute_with_meta). */
export interface TradeLogEntry {
  id: string;
  /** ISO-8601 timestamp from `chrono::Utc::now().to_rfc3339()`. */
  timestamp: string;
  strategyId: string;
  strategyName: string;
  symbol: string;
  side: 'BUY' | 'SELL';
  quantity: number;
  price: number;
  orderId: string;
  /** Stringified `OrderStatus` debug representation, e.g. "Traded",
   *  "Pending", "Rejected". */
  orderStatus: string;
  mode: string;
  ruleId: string;
  notes: string;
}

/** Payload of the `"live-session-failed"` Tauri event. Emitted by
 *  `LiveSession` via `SessionEventEmitter::emit_failed` when the tick loop
 *  transitions to `Failed` (e.g. feed channel closed). */
export interface LiveSessionFailedPayload {
  strategyId: string;
  reason: string;
  /** Best-effort open-position count at failure time. `null` when the
   *  failure happened before positions could be fetched (e.g. feed
   *  closed before the first on_candle). */
  openPositionsEstimate: number | null;
}

/** Payload of the `"live-session-stopped-with-positions"` Tauri event.
 *  Emitted by `stop_live_strategy` when the stopped session left open
 *  positions the user must close manually. */
export interface LiveSessionStoppedPayload {
  warning: string;
}