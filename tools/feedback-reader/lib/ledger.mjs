// The parseable, LOCAL-ONLY feedback ledger (JSONL, one record per report).
//
// Record: { id, fromNpub, handle, handleSource, type, body, ts, status, verdict,
//           issue, wrapId }. `handle` = the reporter's signed-in name; its
//           `handleSource` is 'embedded' (client-tagged at send) | 'persona'
//           (resolved from the kind-31000 credential) | null.
// Holds persona npubs + raw kid text (possible PII) → gitignored, never committed.
// In-place updates (status new→triaged→resolved, verdict, issue) rewrite the file
// atomically (tmp + rename) so the latest line per id is the current truth.
//
// Privacy (public privacy page: bug reports are kept at most 6 months):
//   - the file is always written mode 0600 (and an existing file is chmod'ed to
//     0600 when opened), because it holds possible PII;
//   - rows whose `ts` (unix seconds) is older than RETENTION_DAYS are pruned on
//     load and on every write, and a stale report is never added. Rows with no
//     numeric `ts` are kept (we cannot date them).
import { readFileSync, writeFileSync, renameSync, existsSync, chmodSync } from 'node:fs';

export const RETENTION_DAYS = 180;
const FILE_MODE = 0o600;

export class Ledger {
  /** @param {string} filePath @param {{ now?: () => number }} [opts] now() = unix seconds (tests). */
  constructor(filePath, opts = {}) {
    this.path = filePath;
    this.now = opts.now || (() => Math.floor(Date.now() / 1000));
    this.records = new Map();
    if (existsSync(filePath)) {
      const text = readFileSync(filePath, 'utf8');
      for (const line of text.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        try {
          const r = JSON.parse(trimmed);
          if (r && r.id != null) this.records.set(r.id, r);
        } catch {
          /* skip a corrupt line rather than lose the whole ledger */
        }
      }
      try { chmodSync(filePath, FILE_MODE); } catch { /* best effort */ }
      if (this._prune()) this._persist();
    }
  }

  /** True if the record is older than the retention window. */
  _expired(r) {
    return typeof r.ts === 'number' && r.ts < this.now() - RETENTION_DAYS * 86400;
  }

  /** Drop expired rows. Returns true if any were removed. */
  _prune() {
    let removed = false;
    for (const [id, r] of this.records) {
      if (this._expired(r)) { this.records.delete(id); removed = true; }
    }
    return removed;
  }

  all() { return [...this.records.values()]; }
  get(id) { return this.records.get(id); }
  has(id) { return this.records.has(id); }
  /** Records still needing triage (status 'new'). */
  newReports() { return this.all().filter((r) => r.status === 'new'); }

  /** Add a record. Returns false (no-op) if its id already exists — idempotent. */
  append(rec) {
    if (rec == null || rec.id == null) throw new Error('ledger.append: record needs an id');
    if (this.records.has(rec.id)) return false;
    if (this._expired(rec)) return false; // older than the retention window — never stored
    this.records.set(rec.id, { ...rec });
    this._persist();
    return true;
  }

  /** Merge a patch into an existing record (e.g. status/verdict/issue). */
  update(id, patch) {
    const cur = this.records.get(id);
    if (!cur) return false;
    this.records.set(id, { ...cur, ...patch });
    this._persist();
    return true;
  }

  _persist() {
    this._prune();
    const lines = [...this.records.values()].map((r) => JSON.stringify(r)).join('\n');
    const tmp = this.path + '.tmp';
    writeFileSync(tmp, lines ? lines + '\n' : '', { encoding: 'utf8', mode: FILE_MODE });
    chmodSync(tmp, FILE_MODE); // mode is ignored if a stale .tmp already existed
    renameSync(tmp, this.path);
  }
}
