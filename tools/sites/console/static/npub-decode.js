// Axe'n'Stax — bech32 DECODE for Nostr npub (sibling of bech32.js, per its own
// instruction to write a decoder here rather than expand the encoder).
//
// NIP-19: npub = bech32(hrp="npub", data=convertBits(32-byte pubkey, 8→5, pad)).
// This is the inverse: npub bech32 → 32-byte hex. We verify the checksum and the
// hrp ("npub") and reject anything malformed (return null) — a permissive decoder
// is a security bug (you'd follow / adopt from the wrong pubkey).
//
// Exposes:
//   window.AxeBech32Decode = { npubToHex(npub): string|null }   (lowercase 64-hex)
// and the two wasm-extern targets the engine binds:
//   window.axenstax_npub_to_hex(s): string|null
//   window.axenstax_npub_encode(hex): string|null   (delegates to AxeBech32)

(function () {
    'use strict';

    const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
    const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];

    function polymod(values) {
        let chk = 1;
        for (let p = 0; p < values.length; p++) {
            const top = chk >>> 25;
            chk = ((chk & 0x1ffffff) << 5) ^ values[p];
            for (let i = 0; i < 5; i++) {
                if ((top >>> i) & 1) chk ^= GEN[i];
            }
        }
        return chk;
    }

    function hrpExpand(hrp) {
        const out = [];
        for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) >> 5);
        out.push(0);
        for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) & 31);
        return out;
    }

    // Verify the 6-symbol bech32 (not bech32m) checksum over hrp + data.
    function verifyChecksum(hrp, data) {
        return polymod(hrpExpand(hrp).concat(data)) === 1;
    }

    // Inverse of bech32.js's convertBits: 5-bit groups → 8-bit bytes, dropping the
    // zero pad. Returns null if the pad is non-zero or the leftover bit count is
    // too large (both indicate a malformed payload).
    function convertBits(data, fromBits, toBits, pad) {
        let acc = 0;
        let bits = 0;
        const ret = [];
        const maxv = (1 << toBits) - 1;
        for (let p = 0; p < data.length; p++) {
            const value = data[p];
            if (value < 0 || (value >>> fromBits) !== 0) return null;
            acc = (acc << fromBits) | value;
            bits += fromBits;
            while (bits >= toBits) {
                bits -= toBits;
                ret.push((acc >>> bits) & maxv);
            }
        }
        if (pad) {
            if (bits > 0) ret.push((acc << (toBits - bits)) & maxv);
        } else if (bits >= fromBits || ((acc << (toBits - bits)) & maxv)) {
            return null;
        }
        return ret;
    }

    // Decode a bech32 string into { hrp, data(5-bit symbols) } or null.
    function bech32Decode(str) {
        if (typeof str !== 'string') return null;
        // No mixed case (bech32 forbids it). Normalise to lowercase for lookup.
        const lower = str.toLowerCase();
        const upper = str.toUpperCase();
        if (str !== lower && str !== upper) return null;
        const s = lower;
        const pos = s.lastIndexOf('1');
        // hrp ≥ 1, separator, then ≥ 6 checksum symbols. Whole thing ≤ 90 (bech32 cap).
        if (pos < 1 || pos + 7 > s.length || s.length > 90) return null;
        const hrp = s.slice(0, pos);
        for (let i = 0; i < hrp.length; i++) {
            const c = s.charCodeAt(i);
            if (c < 33 || c > 126) return null;
        }
        const data = [];
        for (let i = pos + 1; i < s.length; i++) {
            const d = CHARSET.indexOf(s.charAt(i));
            if (d === -1) return null;
            data.push(d);
        }
        if (!verifyChecksum(hrp, data)) return null;
        // Strip the 6 checksum symbols.
        return { hrp: hrp, data: data.slice(0, data.length - 6) };
    }

    function npubToHex(npub) {
        const dec = bech32Decode(npub);
        if (!dec || dec.hrp !== 'npub') return null;
        const bytes = convertBits(dec.data, 5, 8, false);
        if (!bytes || bytes.length !== 32) return null;
        let hex = '';
        for (let i = 0; i < bytes.length; i++) {
            hex += (bytes[i] & 0xff).toString(16).padStart(2, '0');
        }
        return hex;
    }

    window.AxeBech32Decode = {
        npubToHex: npubToHex,
    };

    // wasm-extern targets bound by game/engine/src/npub.rs.
    window.axenstax_npub_to_hex = function (s) {
        try { return window.AxeBech32Decode.npubToHex(s) || null; } catch (e) { return null; }
    };
    window.axenstax_npub_encode = function (hex) {
        try { return (window.AxeBech32 && window.AxeBech32.npubEncode(hex)) || null; } catch (e) { return null; }
    };
})();
