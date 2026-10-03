// Axe'n'Stax — minimal bech32 encoder for Nostr npub.
//
// NIP-19: npub = bech32(hrp="npub", data=convertBits(32-byte pubkey, 8→5, pad=true)).
// Encode-only — we never need to decode npubs (we store and pass hex everywhere
// internally; npub is purely a presentation format). If a decoder is needed
// later, write it in a sibling file rather than expanding this one.
//
// Exposes: window.AxeBech32 = { npubEncode(hex64): string|null }

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

    function createChecksum(hrp, data) {
        const values = hrpExpand(hrp).concat(data).concat([0, 0, 0, 0, 0, 0]);
        const mod = polymod(values) ^ 1; // bech32 (not bech32m); NIP-19 uses bech32
        const ret = [];
        for (let i = 0; i < 6; i++) ret.push((mod >>> (5 * (5 - i))) & 31);
        return ret;
    }

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

    function bech32Encode(hrp, data) {
        const combined = data.concat(createChecksum(hrp, data));
        let ret = hrp + '1';
        for (let i = 0; i < combined.length; i++) ret += CHARSET.charAt(combined[i]);
        return ret;
    }

    function hexToBytes(hex) {
        if (typeof hex !== 'string' || hex.length !== 64) return null;
        const out = new Array(32);
        for (let i = 0; i < 32; i++) {
            const b = parseInt(hex.substr(i * 2, 2), 16);
            if (!Number.isFinite(b)) return null;
            out[i] = b;
        }
        return out;
    }

    function npubEncode(hex) {
        const bytes = hexToBytes(hex);
        if (!bytes) return null;
        const data = convertBits(bytes, 8, 5, true);
        if (!data) return null;
        return bech32Encode('npub', data);
    }

    function npubShort(hex) {
        const npub = npubEncode(hex);
        if (!npub) return null;
        // npub1<52 chars>; show first 12 + last 6 with ellipsis.
        if (npub.length <= 22) return npub;
        return npub.slice(0, 12) + '…' + npub.slice(-6);
    }

    window.AxeBech32 = {
        npubEncode: npubEncode,
        npubShort: npubShort,
    };
})();
