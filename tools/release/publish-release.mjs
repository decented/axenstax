#!/usr/bin/env node
// Publish one AxeNStax AppImage release: upload it to the Blossom mirrors,
// verify each mirror the way a PLAYER'S UPDATER will use it, then announce a
// signed kind-30063 event on the release relays. Lifted from Vitark Train's
// tools/release/publish-train-release.mjs (2026-09-05) — see release-helpers.mjs
// for the itemised list of what differs and why.
//
//   node tools/release/publish-release.mjs \
//     --artifact <path-to-AppImage> [--notes "…" | --notes-file <path>] \
//     [--dry-run] [--force]
//
// Version is NOT a flag: it's read from the artifact's own filename
// (axenstax-engine_<version>_x86_64.AppImage) and cross-checked against
// game/engine/Cargo.toml, so the event can never announce a version the
// artifact doesn't actually claim to be, or one that drifted from the source
// tree that built it.
//
// Env: AXENSTAX_BLOSSOM_SERVERS (comma-separated, default below),
//      AXENSTAX_RELEASE_KEY_FILE (default ~/.axenstax-release/release-key.hex)
//
// Exit non-zero when: the filename version disagrees with Cargo.toml; the
// local key is not the pinned release key (once one is pinned — see
// release-helpers.mjs); the new version is not strictly newer than the live
// one and --force was not passed; NO mirror served the bytes back at their
// canonical address with a direct 200 and a matching sha256; or every relay
// refused the event. --dry-run signs and prints the event but never touches
// the network at all — no upload, no publish, and (deliberately, see below)
// no live-version query either.
import { finalizeEvent, getPublicKey, verifyEvent } from 'nostr-tools/pure';
import { createHash } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	RELEASE_CHANNEL,
	RELEASE_PUBKEY_HEX,
	RELEASE_RELAYS,
	SOFTWARE_RELEASE_KIND,
	buildBlossomAuth,
	buildReleaseEvent,
	compareVersions,
	isCanonicalBlossomUrl,
	parseAppImageVersion,
	verifyReleaseEvent
} from './release-helpers.mjs';
import { publishToRelay, queryAllRelays } from './release-relay.mjs';

// Live-verified against Fathom/Vitark's release pipeline (2026-08-12, 10 MB
// artifact) — blossom.band REFUSES non-media uploads with HTTP 415;
// blossom.sovbit.host was unreachable. Re-verify before adding servers.
const DEFAULT_BLOSSOM = 'https://blossom.primal.net,https://nostr.download';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const CARGO_TOML = join(REPO_ROOT, 'game', 'engine', 'Cargo.toml');

function parseArgs(argv) {
	const args = {};
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === '--dry-run') args.dryRun = true;
		else if (a === '--force') args.force = true;
		else if (a === '--artifact') args.artifact = argv[++i];
		else if (a === '--notes') args.notes = argv[++i];
		else if (a === '--notes-file') args.notesFile = argv[++i];
		else throw new Error(`unknown argument: ${a}`);
	}
	if (!args.artifact) throw new Error('--artifact is required');
	if (args.notes !== undefined && args.notesFile !== undefined)
		throw new Error('pass --notes or --notes-file, not both');
	return args;
}

function resolveNotes(args) {
	if (args.notesFile !== undefined) return readFileSync(args.notesFile, 'utf8');
	return args.notes ?? '';
}

/**
 * The version game/engine/Cargo.toml declares — the source-of-truth build
 * this artifact is SUPPOSED to be, independent of what its filename claims.
 */
function cargoTomlVersion() {
	const toml = readFileSync(CARGO_TOML, 'utf8');
	const m = /^version\s*=\s*"([^"]+)"/m.exec(toml);
	if (!m) throw new Error(`could not find a version = "..." line in ${CARGO_TOML}`);
	return m[1];
}

/**
 * Load the release secret. If RELEASE_PUBKEY_HEX is pinned (see
 * release-helpers.mjs), assert this key IS that one — publish-release.mjs
 * must never sign with any other key once a pin exists, since no updater
 * would trust the result. Before a pin exists (bootstrapping, see that
 * file's comment) this only warns: whatever key is in the file becomes the
 * de-facto release key the first time this actually publishes.
 */
function loadReleaseKey() {
	const file =
		process.env.AXENSTAX_RELEASE_KEY_FILE ?? join(homedir(), '.axenstax-release', 'release-key.hex');
	const hex = readFileSync(file, 'utf8').trim();
	if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error(`${file} is not a 64-hex secret`);
	const sk = Uint8Array.from(Buffer.from(hex, 'hex'));
	const pk = getPublicKey(sk);
	if (/^[0-9a-f]{64}$/.test(RELEASE_PUBKEY_HEX)) {
		if (pk !== RELEASE_PUBKEY_HEX) {
			console.error(
				`this key is not the one pinned as RELEASE_PUBKEY_HEX: got pubkey ${pk}, expected ${RELEASE_PUBKEY_HEX}`
			);
			process.exit(1);
		}
	} else {
		console.error(
			`WARNING: RELEASE_PUBKEY_HEX is not pinned yet — signing with ${pk} unchecked. ` +
				'Pin it in release-helpers.mjs after this run (see that file and the README).'
		);
	}
	return sk;
}

async function uploadToBlossom(server, bytes, sha256, sk) {
	const auth = finalizeEvent(buildBlossomAuth({ sha256, createdAt: Math.floor(Date.now() / 1000) }), sk);
	const header = `Nostr ${Buffer.from(JSON.stringify(auth)).toString('base64')}`;
	const res = await fetch(`${server}/upload`, {
		method: 'PUT',
		headers: {
			Authorization: header,
			'Content-Type': 'application/octet-stream',
			'X-SHA-256': sha256
		},
		body: bytes
	});
	if (!res.ok) throw new Error(`${server}: upload HTTP ${res.status}`);
}

async function fetchDirect(url) {
	// redirect: 'manual' — a device's updater refuses redirects, so this
	// simulates exactly what a player's build will experience.
	const res = await fetch(url, { redirect: 'manual' });
	if (res.status === 200) return { bytes: Buffer.from(await res.arrayBuffer()) };
	if ([301, 302, 307, 308].includes(res.status)) return { location: res.headers.get('location') };
	return { error: `HTTP ${res.status}` };
}

/**
 * Verify a mirror the way a PLAYER'S UPDATER will use it: GET, no redirects,
 * full-body sha256. The event may only ever carry the blob's CANONICAL root
 * address (isCanonicalBlossomUrl) — never a redirect target. A redirecting
 * server still counts as an upload success (the blob is there), but it
 * contributes no URL: "blob present, not updater-servable."
 */
async function verifyBlossom(server, sha256) {
	for (const url of [`${server}/${sha256}.AppImage`, `${server}/${sha256}`]) {
		const r = await fetchDirect(url);
		if (r.bytes) {
			const got = createHash('sha256').update(r.bytes).digest('hex');
			if (got !== sha256) {
				console.error(`mirror ${url}: serves WRONG bytes (${got})`);
				return null;
			}
			return url;
		}
		if (r.location) {
			console.error(`mirror ${url}: blob present, not updater-servable (redirects to ${r.location})`);
			continue;
		}
		console.error(`mirror ${url}: ${r.error}`);
	}
	return null;
}

/**
 * The monotonicity gate: refuse to publish a version that is not strictly
 * newer than what's already live, UNLESS --force is passed. Unlike Vitark
 * (where this is a separate manual step, query-latest.mjs, the operator runs
 * by hand before publishing), it lives IN publish-release.mjs here — a
 * downgrade silently strands every player who auto-updates off this
 * channel, so the check runs automatically rather than depending on the
 * operator remembering a step. query-latest.mjs still exists standalone for
 * "what's live right now?" — this reuses the same query/verify machinery.
 *
 * Skipped entirely under --dry-run: dry-run's contract is "no network at
 * all" (see the top-of-file comment and Vitark's identical contract for its
 * own --dry-run), and a dry-run event is never actually announced, so there
 * is nothing here for the gate to protect.
 */
async function checkMonotonic(version, force) {
	const filter = { kinds: [SOFTWARE_RELEASE_KIND], '#d': [RELEASE_CHANNEL], limit: 10 };
	if (/^[0-9a-f]{64}$/.test(RELEASE_PUBKEY_HEX)) filter.authors = [RELEASE_PUBKEY_HEX];
	const results = await queryAllRelays(RELEASE_RELAYS, filter, { timeoutMs: 8_000 });

	const anyAnswered = results.some((r) => r.answered);
	let best = null;
	for (const r of results) {
		for (const ev of r.events) {
			const release = verifyReleaseEvent(ev, verifyEvent);
			if (release && (best === null || compareVersions(release.version, best.version) > 0)) best = release;
		}
	}

	if (!anyAnswered) {
		const msg = 'no relay answered — cannot confirm this is newer than the live release';
		if (force) {
			console.error(`WARNING: ${msg}; proceeding anyway (--force)`);
			return;
		}
		console.error(`${msg}. Use --force to publish anyway.`);
		process.exit(1);
	}

	console.log(`live release: ${best === null ? 'none' : best.version}`);
	if (best !== null && compareVersions(version, best.version) <= 0) {
		const msg = `${version} is not strictly newer than the live release ${best.version}`;
		if (force) {
			console.error(`WARNING: ${msg}; publishing anyway (--force)`);
			return;
		}
		console.error(`${msg}. Refusing — this would downgrade every player who auto-updates. Use --force to override.`);
		process.exit(1);
	}
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const notes = resolveNotes(args);
	const sk = loadReleaseKey();
	console.log(`release key: ${getPublicKey(sk)}`);

	const filenameVersion = parseAppImageVersion(args.artifact);
	const cargoVersion = cargoTomlVersion();
	if (filenameVersion !== cargoVersion) {
		console.error(
			`filename claims version ${filenameVersion} but game/engine/Cargo.toml says ${cargoVersion} — refusing`
		);
		process.exit(1);
	}
	const version = filenameVersion;
	console.log(`version: ${version} (filename and Cargo.toml agree)`);

	if (args.dryRun) {
		console.log('--dry-run: skipping the live-version check (no network)');
	} else {
		await checkMonotonic(version, args.force ?? false);
	}

	const bytes = readFileSync(args.artifact);
	const sha256 = createHash('sha256').update(bytes).digest('hex');
	const sizeBytes = statSync(args.artifact).size;
	console.log(`${args.artifact}: sha256=${sha256} size=${sizeBytes}`);

	const servers = (process.env.AXENSTAX_BLOSSOM_SERVERS ?? DEFAULT_BLOSSOM)
		.split(',')
		.map((s) => s.trim().replace(/\/$/, ''))
		.filter(Boolean);

	const urls = [];
	if (args.dryRun) {
		// A dry-run event must still be VALID: derive mirror URLs without
		// uploading or touching the network at all. Bare form (no extension) —
		// isCanonicalBlossomUrl accepts both, and nothing was actually verified
		// to prefer the extension-bearing one.
		for (const s of servers) urls.push(`${s}/${sha256}`);
	} else {
		let uploaded = 0;
		for (const server of servers) {
			try {
				await uploadToBlossom(server, bytes, sha256, sk);
				uploaded++;
				const verified = await verifyBlossom(server, sha256);
				if (verified) {
					urls.push(verified);
					console.log(`mirror ok: ${verified}`);
				} else {
					console.error(`mirror ${server}: uploaded, but no updater-servable URL`);
				}
			} catch (err) {
				console.error(`mirror FAILED: ${err.message}`);
			}
		}
		if (uploaded === 0) {
			console.error('no Blossom mirror accepted the upload — refusing to announce');
			process.exit(1);
		}
	}

	if (urls.length === 0) {
		console.error(
			'no Blossom mirror serves these bytes at a canonical address; refusing to announce a release nothing can download'
		);
		process.exit(1);
	}

	// Belt and braces: buildReleaseEvent throws on a non-canonical URL too.
	const bad = urls.filter((u) => !isCanonicalBlossomUrl(u, sha256));
	if (bad.length) {
		console.error(`refusing to announce non-canonical mirror url(s): ${bad.join(', ')}`);
		process.exit(1);
	}

	const event = finalizeEvent(
		buildReleaseEvent({
			channel: RELEASE_CHANNEL,
			version,
			sha256,
			sizeBytes,
			urls: [...new Set(urls)],
			notes,
			createdAt: Math.floor(Date.now() / 1000)
		}),
		sk
	);

	if (args.dryRun) {
		console.log('--dry-run: signed event follows; nothing uploaded, queried, or published');
		console.log(JSON.stringify(event, null, 2));
		return;
	}

	const results = await Promise.all(RELEASE_RELAYS.map((r) => publishToRelay(r, event)));
	for (const r of results) {
		console.log(`${r.ok ? 'relay ok  ' : 'relay FAIL'}: ${r.url} ${r.reason ?? ''}`);
	}
	if (!results.some((r) => r.ok)) {
		console.error('every relay refused the release event');
		process.exit(1);
	}
	console.log(`announced ${RELEASE_CHANNEL} ${version}`);
}

main().catch((err) => {
	console.error(err.message);
	process.exit(1);
});
