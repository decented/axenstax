// The parseable, LOCAL-ONLY feedback ledger (JSONL, one record per report).
//
// Record: { id, fromNpub, handle, handleSource, type, body, ts, status, verdict,
//           issue, wrapId }. `handle` = the reporter's signed-in name; its
//           `handleSource` is 'embedded' (client-tagged at send) | 'persona'
//           (resolved from the kind-31000 credential) | null.
// Holds persona npubs + raw kid text (possible PII) → gitignored, never committed.
// In-place updates (status new→triaged→resolved, verdict, issue) rewrite the file
// atomically (tmp + rename) so the latest line per id is the current truth.
import { readFileSync, writeFileSync, renameSync, existsSync } from 'node:fs';

export class Ledger {
  constructor(filePath) {
    this.path = filePath;
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
    }
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
    const lines = [...this.records.values()].map((r) => JSON.stringify(r)).join('\n');
    const tmp = this.path + '.tmp';
    writeFileSync(tmp, lines ? lines + '\n' : '', 'utf8');
    renameSync(tmp, this.path);
  }
}
