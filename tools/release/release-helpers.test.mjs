import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
	RELEASE_CHANNEL,
	SOFTWARE_RELEASE_KIND,
	buildBlossomAuth,
	buildReleaseEvent,
	compareVersions,
	isAnsweringFrameType,
	isCanonicalBlossomUrl,
	parseAppImageVersion,
	verifyReleaseEvent
} from './release-helpers.mjs';

const SHA = 'a'.repeat(64);
const PUBKEY = 'c'.repeat(64);
const ok = {
	channel: RELEASE_CHANNEL,
	version: '0.2.21',
	sha256: SHA,
	sizeBytes: 21699064,
	urls: [`https://blossom.primal.net/${SHA}.AppImage`],
	notes: 'notes',
	createdAt: 1757000000
};

test('buildReleaseEvent emits the tags in the agreed order (no version_code, no cert)', () => {
	const ev = buildReleaseEvent(ok);
	assert.equal(ev.kind, SOFTWARE_RELEASE_KIND);
	assert.deepEqual(ev.tags, [
		['d', 'axenstax-appimage'],
		['version', '0.2.21'],
		['x', SHA],
		['size', '21699064'],
		['url', `https://blossom.primal.net/${SHA}.AppImage`]
	]);
	assert.equal(ev.content, 'notes');
});

test('buildReleaseEvent refuses anything malformed', () => {
	for (const bad of [
		{ channel: 'axenstax-apk' },
		{ version: '' },
		{ version: 'v0.2.21' },
		{ version: 'not-a-version' },
		{ sha256: 'short' },
		{ sizeBytes: 0 },
		{ urls: [] },
		{ urls: [`http://h/${SHA}`] },
		{ urls: [`https://h/uploads/${SHA}`] },
		{ createdAt: 0 }
	]) {
		assert.throws(() => buildReleaseEvent({ ...ok, ...bad }), /.*/, JSON.stringify(bad));
	}
});

test('isCanonicalBlossomUrl accepts only the blob root address', () => {
	assert.equal(isCanonicalBlossomUrl(`https://h/${SHA}`, SHA), true);
	assert.equal(isCanonicalBlossomUrl(`https://h/${SHA}.AppImage`, SHA), true);
	assert.equal(isCanonicalBlossomUrl(`https://h/${SHA}?x=1`, SHA), false);
	assert.equal(isCanonicalBlossomUrl(`https://h/a/b/${SHA}`, SHA), false);
	assert.equal(isCanonicalBlossomUrl(`http://h/${SHA}`, SHA), false);
});

test('parseAppImageVersion reads the version out of the artifact filename', () => {
	assert.equal(parseAppImageVersion('axenstax-engine_0.2.21_x86_64.AppImage'), '0.2.21');
	assert.equal(parseAppImageVersion('axenstax-engine_1.0.0.3_x86_64.AppImage'), '1.0.0.3');
	assert.throws(() => parseAppImageVersion('axenstax-engine_x86_64.AppImage'));
});

test('parseAppImageVersion does not mistake the arch suffix for the version', () => {
	// A bare `\d+` would match the "64" in `_x86_64` as a one-component
	// "version" — requiring at least two dotted numeric components rules
	// that out, so a filename with no real dotted version finds nothing.
	assert.throws(() => parseAppImageVersion('axenstax-engine_3_x86_64.AppImage'));
});

test('compareVersions orders numerically, not lexicographically', () => {
	assert.ok(compareVersions('0.2.10', '0.2.9') > 0);
	assert.ok(compareVersions('0.2.9', '0.2.10') < 0);
	assert.equal(compareVersions('0.2.21', '0.2.21'), 0);
	assert.equal(compareVersions('0.2', '0.2.0'), 0);
	assert.throws(() => compareVersions('0.2.x', '0.2.1'));
});

test('CLOSED is a refusal, not an answer', () => {
	assert.equal(isAnsweringFrameType('EVENT'), true);
	assert.equal(isAnsweringFrameType('EOSE'), true);
	assert.equal(isAnsweringFrameType('CLOSED'), false);
	assert.equal(isAnsweringFrameType('NOTICE'), false);
});

test('buildBlossomAuth is a BUD-02 upload template that expires', () => {
	const a = buildBlossomAuth({ sha256: SHA, createdAt: 1757000000 });
	assert.equal(a.kind, 24242);
	assert.deepEqual(a.tags, [
		['t', 'upload'],
		['x', SHA],
		['expiration', String(1757000000 + 600)]
	]);
	assert.throws(() => buildBlossomAuth({ sha256: 'short', createdAt: 1 }));
});

test('verifyReleaseEvent rejects everything while RELEASE_PUBKEY_HEX is unpinned', () => {
	// release-helpers.mjs pins RELEASE_PUBKEY_HEX = '' until the real ceremony
	// runs (see its comment) — so no event can verify yet, by construction.
	const ev = { kind: SOFTWARE_RELEASE_KIND, pubkey: PUBKEY, tags: [['d', RELEASE_CHANNEL]] };
	assert.equal(
		verifyReleaseEvent(ev, () => true),
		null
	);
});

test('verifyReleaseEvent rejects the wrong kind or channel even with a permissive verifier', () => {
	const wrongKind = { kind: 1, pubkey: PUBKEY, tags: [['d', RELEASE_CHANNEL]] };
	const wrongChannel = { kind: SOFTWARE_RELEASE_KIND, pubkey: PUBKEY, tags: [['d', 'something-else']] };
	assert.equal(
		verifyReleaseEvent(wrongKind, () => true),
		null
	);
	assert.equal(
		verifyReleaseEvent(wrongChannel, () => true),
		null
	);
});
