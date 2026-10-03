#!/usr/bin/env node
// Print the highest-version VERIFIED axenstax-appimage release event on the
// release relays — the same relays ARE the record here, same as Vitark's
// query-latest.mjs. Run this by hand to answer "what are players actually
// being offered right now?" It's also the read half of the check
// publish-release.mjs runs on its own before announcing (see that file).
//
//   node tools/release/query-latest.mjs            -> one line, or "none"
//   node tools/release/query-latest.mjs --verbose  -> + per-relay detail on stderr
//
// Exit codes: 0 = answered (details printed, or "none" = relays answered and
// there is no verifiable release yet); 3 = no relay answered — a caller MUST
// NOT treat that as "none" and skip its monotonicity gate.
import { verifyEvent } from 'nostr-tools/pure';
import { RELEASE_CHANNEL, RELEASE_PUBKEY_HEX, SOFTWARE_RELEASE_KIND, compareVersions, verifyReleaseEvent, RELEASE_RELAYS } from './release-helpers.mjs';
import { queryAllRelays } from './release-relay.mjs';

const QUERY_TIMEOUT_MS = 8_000;
const verbose = process.argv.includes('--verbose');
const log = (...a) => {
	if (verbose) console.error(...a);
};

async function main() {
	if (!/^[0-9a-f]{64}$/.test(RELEASE_PUBKEY_HEX)) {
		console.error(
			'RELEASE_PUBKEY_HEX is not pinned yet in release-helpers.mjs (run new-release-key.mjs, ' +
				'then paste the printed pubkey in and commit) — nothing can be verified as authentic ' +
				'until then, so this will always report "none".'
		);
	}

	const filter = { kinds: [SOFTWARE_RELEASE_KIND], '#d': [RELEASE_CHANNEL], limit: 10 };
	if (/^[0-9a-f]{64}$/.test(RELEASE_PUBKEY_HEX)) filter.authors = [RELEASE_PUBKEY_HEX];
	const results = await queryAllRelays(RELEASE_RELAYS, filter, { timeoutMs: QUERY_TIMEOUT_MS, log });

	let anyAnswered = false;
	let best = null;
	for (const r of results) {
		if (r.answered) anyAnswered = true;
		log(`${r.url}: ${r.answered ? `answered, ${r.events.length} event(s)` : 'no answer'}`);
		for (const ev of r.events) {
			const release = verifyReleaseEvent(ev, verifyEvent);
			if (release === null) {
				log(`  skipped unverified/mismatched event ${ev?.id ?? '?'}`);
				continue;
			}
			log(`  verified release: version=${release.version}`);
			if (best === null || compareVersions(release.version, best.version) > 0) best = release;
		}
	}

	if (!anyAnswered) {
		// stdout to a pipe is asynchronous in Node — process.exit() right after
		// console.error/console.log can cut the write short before it flushes
		// (a caller capturing this via `$(...)` could see truncated output,
		// which would wrongly pass a monotonicity gate). Write explicitly and
		// exit only once the write callback fires.
		process.stderr.write('no relay answered — cannot determine the latest release\n', () =>
			process.exit(3)
		);
		return;
	}

	if (best === null) {
		process.stdout.write('none\n', () => process.exit(0));
		return;
	}

	const lines = [
		`version: ${best.version}`,
		`sha256:  ${best.sha256}`,
		`size:    ${best.sizeBytes} bytes`,
		...best.urls.map((u, i) => `${i === 0 ? 'mirrors: ' : '         '}${u}`)
	];
	process.stdout.write(lines.join('\n') + '\n', () => process.exit(0));
}

main().catch((err) => {
	process.stderr.write(`${err.message}\n`, () => process.exit(3));
});
