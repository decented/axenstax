// The public feedback status board (spec docs/foundations/2026-10-01-feedback-
// status-board.md, S2/S3/S8). ONE addressable kind-30078 event, d =
// "axenstax-feedback-status", signed by the official AxeNStax key, mapping
// SCRAMBLED ticket hashes to a status. No free text, no npubs, no titles —
// nobody is ever messaged and no entry is linkable to a person without the
// ticket (the report-id), which only the sender's own game holds.
//
// The board is kept in a LOCAL file (owner-only, 0600) so each update is a
// read-modify-publish of the whole list; the ledger stays owner-local and is
// never published.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, renameSync, mkdirSync } from 'node:fs';
import path from 'node:path';

export const BOARD_D = 'axenstax-feedback-status';
export const BOARD_KIND = 30078;
export const MAX_AGE_SECS = 180 * 24 * 3600;
export const MAX_ENTRIES = 2000;
export const STATUSES = ['received', 'fixed', 'wontfix'];
// The project's feedback INBOX relays — the same fixed set the native game
// publishes reports to and reads this board from
// (game/engine/src/native_mailbox/mod.rs FEEDBACK_INBOX_RELAYS, S4). Public
// relays that serve kind 1059 without NIP-42 auth (read-only probe,
// 2026-10-02); relay.damus.io is NOT here because it demands auth for 1059.
// No AxeNStax-operated relay is a default (CLAUDE.md red line 2).
export const INBOX_RELAYS = [
  'wss://nos.lol',
  'wss://relay.primal.net',
  'wss://offchain.pub',
];
export const DEFAULT_RELAYS = INBOX_RELAYS;

// Builds up to v0.2.27 send /bug only to relay.trotters.cc (and read their
// status board there), so the owner-local reader keeps listening on it until
// those installs age out. Reader-side only — never a player default.
export const LEGACY_RELAYS = ['wss://relay.trotters.cc'];
export const READ_RELAYS = [...INBOX_RELAYS, ...LEGACY_RELAYS];

/** `RELAY` / `RELAYS` env override (comma list) or the inbox + legacy set. */
export function relaysFromEnv(env = process.env) {
  const raw = env.RELAYS || env.RELAY;
  if (!raw) return READ_RELAYS;
  const list = raw.split(',').map((s) => s.trim()).filter(Boolean);
  return list.length ? list : READ_RELAYS;
}

const VERSION_RE = /^[0-9A-Za-z.+-]{1,24}$/;

// S2: sha256("axenstax-feedback-ticket:" + ticket) hex, first 32 chars.
export function ticketKey(ticket) {
  return createHash('sha256').update('axenstax-feedback-ticket:' + ticket).digest('hex').slice(0, 32);
}

export function emptyBoard() {
  return { v: 1, updated: 0, t: {} };
}

// A missing file is an empty board; a corrupt one is an ERROR (never silently
// overwrite the owner's board with an empty one).
export function loadBoard(file) {
  let raw;
  try { raw = readFileSync(file, 'utf8'); }
  catch (e) { if (e.code === 'ENOENT') return emptyBoard(); throw e; }
  const b = JSON.parse(raw);
  if (!b || b.v !== 1 || typeof b.t !== 'object' || b.t === null) {
    throw new Error(`board file ${file} is not a v1 status board`);
  }
  return { v: 1, updated: Number(b.updated) || 0, t: b.t };
}

export function saveBoard(file, board) {
  mkdirSync(path.dirname(path.resolve(file)), { recursive: true });
  const tmp = `${file}.tmp`;
  writeFileSync(tmp, JSON.stringify(board, null, 1) + '\n', { mode: 0o600 });
  renameSync(tmp, file);
}

// Set one ticket's status. `onlyIfAbsent` is for auto-marking `received`: it
// must never downgrade a hand-set fixed/wontfix. Returns true if it changed.
export function setStatus(board, ticket, status, { version, now = Math.floor(Date.now() / 1000), onlyIfAbsent = false } = {}) {
  if (!STATUSES.includes(status)) throw new Error(`status must be one of ${STATUSES.join('|')}`);
  if (typeof ticket !== 'string' || !ticket) throw new Error('a report id is required');
  if (version !== undefined && !VERSION_RE.test(version)) {
    throw new Error('version must be 1-24 chars of [0-9A-Za-z.+-]');
  }
  const key = ticketKey(ticket);
  if (onlyIfAbsent && board.t[key]) return false;
  const entry = { s: status, at: now };
  if (status === 'fixed' && version) entry.v = version;
  board.t[key] = entry;
  return true;
}

// S3: keep entries <=180 days old and <=2000 total, dropping oldest first.
// Returns how many entries were removed.
export function prune(board, now = Math.floor(Date.now() / 1000)) {
  const before = Object.keys(board.t).length;
  for (const [k, e] of Object.entries(board.t)) {
    if (!e || typeof e.at !== 'number' || now - e.at > MAX_AGE_SECS) delete board.t[k];
  }
  const keys = Object.keys(board.t);
  if (keys.length > MAX_ENTRIES) {
    keys.sort((a, b) => board.t[a].at - board.t[b].at); // oldest first
    for (const k of keys.slice(0, keys.length - MAX_ENTRIES)) delete board.t[k];
  }
  return before - Object.keys(board.t).length;
}

// The unsigned event template for the CURRENT board. `created_at` is strictly
// greater than the previous publish so relays always keep the newest version
// of the addressable event, even for two updates in the same second.
export function boardTemplate(board, now = Math.floor(Date.now() / 1000)) {
  const at = Math.max(now, (board.updated || 0) + 1);
  const content = JSON.stringify({ v: 1, updated: at, t: board.t });
  return { kind: BOARD_KIND, created_at: at, tags: [['d', BOARD_D]], content };
}

// Prune, sign with the official signer, publish, and only then persist (so a
// failed publish leaves the on-disk board untouched, ready to retry).
export async function publishBoard({ board, file, signer, relay, now = Math.floor(Date.now() / 1000) }) {
  prune(board, now);
  const tmpl = boardTemplate(board, now);
  const event = signer.signEvent(tmpl);
  await relay.publish(event);
  board.updated = tmpl.created_at;
  if (file) saveBoard(file, board);
  return event;
}

// `read.mjs --auto-received` / `live.mjs --auto-received`: mark newly ingested
// reports `received` (never overriding a status already set) and publish once
// per burst. The ledger is not touched; only hashes reach the board.
export function makeAutoReceiver({ file, signer, relay, debounceMs = 3000, now = () => Math.floor(Date.now() / 1000), log = () => {} }) {
  const pending = new Set();
  let timer = null;
  async function flush() {
    if (timer) { clearTimeout(timer); timer = null; }
    if (!pending.size) return 0;
    const ids = [...pending];
    pending.clear();
    const board = loadBoard(file);
    let changed = 0;
    for (const id of ids) if (setStatus(board, id, 'received', { now: now(), onlyIfAbsent: true })) changed++;
    if (changed) {
      await publishBoard({ board, file, signer, relay, now: now() });
      log(`board: marked ${changed} report(s) received`);
    }
    return changed;
  }
  return {
    add(id) {
      pending.add(id);
      if (debounceMs > 0 && !timer) {
        timer = setTimeout(() => { timer = null; flush().catch((e) => log('board: auto-received failed: ' + e.message)); }, debounceMs);
      }
    },
    flush,
  };
}
