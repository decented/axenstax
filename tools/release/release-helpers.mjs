// Pure builders for the AxeNStax release pipeline — no network, no
// filesystem, so `node --test` covers them exactly. Lifted from Vitark
// Train's tools/release/release-helpers.mjs (2026-09-05), which itself ported
// Fathom's client/scripts/release/release-helpers.mjs. Differences from the
// Vitark case, all deliberate (see README "Where this differs from Vitark"):
//   - No `version_code` / `parseBadging` — an AppImage carries no Android
//     build number. The version comes from the artifact FILENAME
//     (`axenstax-engine_<version>_x86_64.AppImage`), cross-checked against
//     `game/engine/Cargo.toml` by publish-release.mjs (filesystem work, so it
//     lives there, not in this pure module).
//   - No `cert` tag — there is no AppImage signing certificate yet. Add it
//     back (buildReleaseEvent + the tag order below) when signing lands.
//   - `compareVersions` / `verifyReleaseEvent` are new here: AxeNStax's
//     monotonicity gate lives INSIDE publish-release.mjs (not a separate
//     manual step the operator runs first, as query-latest.mjs is for
//     Vitark), so both publish-release.mjs and query-latest.mjs need a
//     shared, pure way to compare semver-ish version strings and to
//     authenticate a relay-returned event.
export const SOFTWARE_RELEASE_KIND = 30063;
// The app reads the release feed from the player's own relay list, whose
// default is server_resolve::PUBLIC_DEFAULT_RELAYS (damus, nos.lol, primal) —
// so those three come first and MUST stay in step with that list.
// relay.trotters.cc is kept only as an extra publish/query target (it holds
// the pre-2026-10 release history the monotonicity gate reads); the app never
// reads it by default.
export const RELEASE_RELAYS = [
	'wss://relay.damus.io',
	'wss://nos.lol',
	'wss://relay.primal.net',
	'wss://relay.trotters.cc',
];
export const RELEASE_CHANNEL = 'axenstax-appimage';
/** BUD-02 Blossom upload authorization kind. */
export const BLOSSOM_AUTH_KIND = 24242;

/**
 * The release key ceremony (new-release-key.mjs) has not been run for real
 * yet on any machine that maintains this repo — this session only ran it
 * into a throwaway HOME to prove the mechanics. Pin the printed pubkey here
 * the first time it IS run for real, and commit that change: from that
 * moment on, loadReleaseKey() in publish-release.mjs treats a mismatch as
 * fatal (never sign a release with any key but this one), and query-latest.mjs
 * / publish-release.mjs's own monotonicity check use it to filter which
 * relay events are even candidates. Until it is pinned (empty string),
 * both scripts skip author verification and warn loudly instead — see the
 * README's "Key ceremony" section.
 */
export const RELEASE_PUBKEY_HEX =
  'ca43e7218d03a50e20b07f54fad58a7cf4ef9ac1a883e3a0104255a46600e502';

const HEX64 = /^[0-9a-f]{64}$/;

/**
 * Does a raw Nostr relay frame TYPE ("EVENT" | "EOSE" | "CLOSED" | "NOTICE" |
 * …) count as the relay having ANSWERED a REQ — used by the monotonicity
 * query's per-relay "answered" tracking (query-latest.mjs and
 * publish-release.mjs's own pre-publish check). EVENT and EOSE do: the relay
 * actually responded (some relays never send EOSE for a REQ they immediately
 * satisfy with EVENTs, so EVENT alone must count too). CLOSED does NOT: it is
 * how a relay reports a REFUSAL of the subscription itself
 * (`auth-required:`, `error:`, `rate-limited:`, …), not an answer to it — all
 * relays refusing must read as "couldn't determine the latest release",
 * never as "none exists."
 */
export function isAnsweringFrameType(type) {
	return type === 'EVENT' || type === 'EOSE';
}

// axenstax-engine_0.2.21_x86_64.AppImage -> "0.2.21". Same convention (and
// same regex shape) as INSTALLER_VERSION_RE in
// tools/sites/docs/versioning.py — kept in step by comment, not by import,
// since that's a separate Python runtime for a separate site. At least two
// numeric segments are required so a bare `\d+` can't match the "64" in
// `_x86_64.AppImage`.
const APPIMAGE_VERSION_RE = /_(\d+(?:\.\d+){1,3})(?=_|\.[A-Za-z])/;

/** Extract the version from an AppImage filename. Throws if none is found. */
export function parseAppImageVersion(filename) {
	const m = APPIMAGE_VERSION_RE.exec(filename);
	if (!m) throw new Error(`could not read a version from filename: ${filename}`);
	return m[1];
}

/**
 * Compare two numeric dotted version strings (`"0.2.21"` vs `"0.2.9"`)
 * component-wise so 0.2.10 sorts ABOVE 0.2.9 — string/lexicographic
 * comparison gets that backwards. Missing trailing components compare as 0
 * (`"0.2"` == `"0.2.0"`). Returns <0, 0, or >0 like a normal comparator.
 */
export function compareVersions(a, b) {
	const pa = String(a).split('.');
	const pb = String(b).split('.');
	const len = Math.max(pa.length, pb.length);
	for (let i = 0; i < len; i++) {
		const na = Number(pa[i] ?? '0');
		const nb = Number(pb[i] ?? '0');
		if (!Number.isInteger(na) || !Number.isInteger(nb) || na < 0 || nb < 0) {
			throw new Error(`not a plain numeric version: ${a} vs ${b}`);
		}
		if (na !== nb) return na - nb;
	}
	return 0;
}

/**
 * A URL a device may be told to fetch `sha256` from: https, and the blob at
 * the server ROOT, addressed by its own hash (`https://host/<sha>` or
 * `https://host/<sha>.<ext>`, BUD-01). That path is the one address a
 * Blossom server promises to keep serving. Anything else — in particular a
 * CDN redirect *target* — is an implementation detail that can vanish
 * (see Vitark's kintrinsic incident, 2026-08-27: every client on the old
 * release sat in a 404-retry loop while a healthy second mirror was never
 * tried).
 */
export function isCanonicalBlossomUrl(url, sha256) {
	if (typeof url !== 'string' || !HEX64.test(sha256 ?? '')) return false;
	let u;
	try {
		u = new URL(url);
	} catch {
		return false;
	}
	if (u.protocol !== 'https:' || u.search || u.hash || u.username || u.password) return false;
	const m = /^\/([0-9a-f]{64})(\.[A-Za-z0-9]{1,8})?$/.exec(u.pathname);
	return m !== null && m[1] === sha256;
}

/**
 * The unsigned kind-30063 template announcing one AppImage build. Throws on
 * any invalid field — a malformed release event must never reach
 * finalizeEvent. Tag order: d, version, x, size, then one url per mirror.
 * No `version_code` (no Android build number here) and no `cert` (no
 * AppImage signing certificate yet — add it back here when signing lands).
 */
export function buildReleaseEvent({ channel, version, sha256, sizeBytes, urls, notes, createdAt }) {
	if (channel !== RELEASE_CHANNEL) throw new Error(`bad channel: ${channel}`);
	if (typeof version !== 'string' || !/^\d+(\.\d+){1,3}$/.test(version))
		throw new Error(`version must be a plain numeric version string: ${version}`);
	if (!HEX64.test(sha256)) throw new Error('sha256 must be 64 lowercase hex chars');
	if (!Number.isInteger(sizeBytes) || sizeBytes <= 0)
		throw new Error('sizeBytes must be a positive integer');
	if (!Array.isArray(urls) || urls.length === 0) throw new Error('at least one url required');
	for (const u of urls) {
		if (typeof u !== 'string' || !u.startsWith('https://'))
			throw new Error(`mirror url must be https: ${u}`);
		if (!isCanonicalBlossomUrl(u, sha256))
			throw new Error(
				`mirror url must be the blob's canonical root address (https://host/<sha>[.ext]): ${u}`
			);
	}
	if (!Number.isInteger(createdAt) || createdAt <= 0) throw new Error('createdAt required');

	const tags = [
		['d', channel],
		['version', version],
		['x', sha256],
		['size', String(sizeBytes)]
	];
	for (const u of urls) tags.push(['url', u]);
	return {
		kind: SOFTWARE_RELEASE_KIND,
		created_at: createdAt,
		tags,
		content: notes ?? ''
	};
}

/**
 * The unsigned BUD-02 upload-authorization template. Signed with the release
 * key and sent as `Authorization: Nostr <base64(signed json)>` on the PUT.
 */
export function buildBlossomAuth({ sha256, createdAt }) {
	if (!HEX64.test(sha256)) throw new Error('sha256 must be 64 lowercase hex chars');
	if (!Number.isInteger(createdAt) || createdAt <= 0) throw new Error('createdAt required');
	return {
		kind: BLOSSOM_AUTH_KIND,
		created_at: createdAt,
		tags: [
			['t', 'upload'],
			['x', sha256],
			['expiration', String(createdAt + 600)]
		],
		content: 'AxeNStax release artifact upload'
	};
}

/**
 * Authenticate one relay-returned event as a genuine axenstax-appimage
 * release: right kind, right `d` tag, a valid signature (verifyEvent), and —
 * once RELEASE_PUBKEY_HEX is pinned (see above) — the pinned pubkey. Before
 * it's pinned this always returns null (nothing verifies), which reads as
 * "no releases yet" — correct for a channel that has never been announced
 * for real. Returns `{ version, sha256, sizeBytes, urls, createdAt }` or
 * null; anything malformed is silently skipped, same fail-quiet posture
 * Vitark's query-latest.mjs uses.
 */
export function verifyReleaseEvent(ev, verifyEventFn) {
	if (typeof ev !== 'object' || ev === null) return null;
	if (ev.kind !== SOFTWARE_RELEASE_KIND) return null;
	const tags = Array.isArray(ev.tags) ? ev.tags : [];
	if (tags.find((t) => t[0] === 'd')?.[1] !== RELEASE_CHANNEL) return null;
	if (!HEX64.test(RELEASE_PUBKEY_HEX) || ev.pubkey !== RELEASE_PUBKEY_HEX) return null;
	if (!verifyEventFn(ev)) return null;
	const version = tags.find((t) => t[0] === 'version')?.[1];
	const sha256 = tags.find((t) => t[0] === 'x')?.[1];
	const sizeBytes = Number(tags.find((t) => t[0] === 'size')?.[1]);
	const urls = tags.filter((t) => t[0] === 'url').map((t) => t[1]);
	if (typeof version !== 'string' || !/^\d+(\.\d+){1,3}$/.test(version)) return null;
	if (!HEX64.test(sha256 ?? '')) return null;
	if (!Number.isInteger(sizeBytes) || sizeBytes <= 0) return null;
	if (urls.length === 0) return null;
	return { version, sha256, sizeBytes, urls, createdAt: ev.created_at };
}
