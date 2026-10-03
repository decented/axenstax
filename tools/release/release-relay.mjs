// Shared WebSocket relay I/O for the AxeNStax release pipeline. Split out
// from release-helpers.mjs on purpose — that module stays pure (no network,
// no filesystem) so `node --test` covers its builders directly; this is the
// network half. New relative to Vitark: there, the relay query
// (query-latest.mjs) and the publish flow (publish-train-release.mjs) never
// needed to share this code, because Vitark's monotonicity check is a
// separate manual step the operator runs first. AxeNStax's publish-release.mjs
// runs the SAME query itself as a built-in gate (see its "Refuse to publish a
// version that is not strictly newer" check), so both scripts import from
// here rather than duplicating ~80 lines of WebSocket handling.
import { isAnsweringFrameType } from './release-helpers.mjs';

/**
 * One relay round-trip for a REQ filter. Resolves { url, answered, events }:
 * `answered` follows isAnsweringFrameType (true on EOSE or any EVENT, false
 * on CLOSED/timeout/socket-error/early-close) — a caller doing a
 * monotonicity check must treat `answered: false` as "couldn't determine",
 * never as "found nothing."
 */
export function queryOneRelay(url, filter, timeoutMs = 8_000, log = () => {}) {
	return new Promise((resolve) => {
		const events = [];
		let answered = false;
		let settled = false;
		let ws;
		const finish = () => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			try {
				ws?.close();
			} catch {
				/* already closed */
			}
			resolve({ url, answered, events });
		};
		const timer = setTimeout(() => {
			log(`${url}: timeout after ${timeoutMs}ms`);
			finish();
		}, timeoutMs);
		try {
			ws = new WebSocket(url);
		} catch (err) {
			log(`${url}: WebSocket ctor threw: ${err.message}`);
			finish();
			return;
		}
		const subId = `q-${Math.random().toString(36).slice(2, 10)}`;
		ws.onopen = () => ws.send(JSON.stringify(['REQ', subId, filter]));
		ws.onmessage = (m) => {
			let msg;
			try {
				msg = JSON.parse(String(m.data));
			} catch {
				return;
			}
			if (msg[1] !== subId) return;
			const type = msg[0];
			if (isAnsweringFrameType(type)) answered = true;
			if (type === 'EVENT') {
				events.push(msg[2]);
			} else if (type === 'EOSE') {
				finish();
			} else if (type === 'CLOSED') {
				log(`${url}: CLOSED ${msg[2] ?? ''}`);
				finish();
			}
		};
		ws.onerror = () => {
			log(`${url}: socket error`);
			finish();
		};
		ws.onclose = () => finish();
	});
}

/** Query every relay in parallel; returns the per-relay results array. */
export async function queryAllRelays(relays, filter, { timeoutMs, log } = {}) {
	return Promise.all(relays.map((r) => queryOneRelay(r, filter, timeoutMs, log)));
}

/** Publish a signed event to one relay, resolving { url, ok, reason }. */
export function publishToRelay(url, event, timeoutMs = 10_000) {
	return new Promise((resolve) => {
		let settled = false;
		let ws;
		const done = (ok, reason) => {
			if (settled) return;
			settled = true;
			try {
				ws?.close();
			} catch {
				/* already closed */
			}
			resolve({ url, ok, reason });
		};
		try {
			ws = new WebSocket(url);
		} catch (err) {
			done(false, err.message);
			return;
		}
		const timer = setTimeout(() => done(false, 'timeout'), timeoutMs);
		ws.onopen = () => ws.send(JSON.stringify(['EVENT', event]));
		ws.onmessage = (m) => {
			try {
				const [type, id, ok, reason] = JSON.parse(m.data);
				if (type === 'OK' && id === event.id) {
					clearTimeout(timer);
					done(Boolean(ok), reason ?? '');
				}
			} catch {
				/* ignore non-JSON frames */
			}
		};
		ws.onerror = () => {
			clearTimeout(timer);
			done(false, 'socket error');
		};
		ws.onclose = () => {
			clearTimeout(timer);
			done(false, 'closed-before-ok');
		};
	});
}
