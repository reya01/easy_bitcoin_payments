/* Minimal QR Code generator (byte mode, error-correction level L, versions
 * 1-40, automatic mask choice). Original implementation for Easy Bitcoin
 * Payments, MIT-licensed with the rest of the repo, written to avoid a
 * third-party dependency (CSP is script-src 'self'; no build step).
 * Follows ISO/IEC 18004. Verified in CI-independent checks by decoding
 * generated codes with OpenCV for every version (see docs/REVIEW_*.md R3).
 * API: window.ebpQrSvg(text, {margin, dark, light}) -> SVG string.
 */
(function () {
  "use strict";
  // [ecCodewordsPerBlock, g1Blocks, g1DataCw, g2Blocks, g2DataCw] for level L.
  var L = [null,
    [7,1,19,0,0],[10,1,34,0,0],[15,1,55,0,0],[20,1,80,0,0],[26,1,108,0,0],
    [18,2,68,0,0],[20,2,78,0,0],[24,2,97,0,0],[30,2,116,0,0],[18,2,68,2,69],
    [20,4,81,0,0],[24,2,92,2,93],[26,4,107,0,0],[30,3,115,1,116],[22,5,87,1,88],
    [24,5,98,1,99],[28,1,107,5,108],[30,5,120,1,121],[28,3,113,4,114],[28,3,107,5,108],
    [28,4,116,4,117],[28,2,111,7,112],[30,4,121,5,122],[30,6,117,4,118],[26,8,106,4,107],
    [28,10,114,2,115],[30,8,122,4,123],[30,3,117,10,118],[30,7,116,7,117],[30,5,115,10,116],
    [30,13,115,3,116],[30,17,115,0,0],[30,17,115,1,116],[30,13,115,6,116],[30,12,121,7,122],
    [30,6,121,14,122],[30,17,122,4,123],[30,4,122,18,123],[30,20,117,4,118],[30,19,118,6,119]
  ];
  var EXP = [], LOG = [];
  (function () {
    var x = 1;
    for (var i = 0; i < 255; i++) { EXP[i] = x; LOG[x] = i; x <<= 1; if (x & 256) x ^= 0x11d; }
    for (var j = 255; j < 512; j++) EXP[j] = EXP[j - 255];
  })();
  function gmul(a, b) { return a && b ? EXP[LOG[a] + LOG[b]] : 0; }
  function rsGen(n) {
    var g = [1];
    for (var i = 0; i < n; i++) {
      var ng = new Array(g.length + 1).fill(0);
      for (var j = 0; j < g.length; j++) { ng[j] ^= g[j]; ng[j + 1] ^= gmul(g[j], EXP[i]); }
      g = ng;
    }
    return g;
  }
  function rsRem(data, n) {
    var g = rsGen(n), r = new Array(n).fill(0);
    for (var i = 0; i < data.length; i++) {
      var f = data[i] ^ r.shift(); r.push(0);
      if (f) for (var j = 0; j < n; j++) r[j] ^= gmul(g[j + 1], f);
    }
    return r;
  }
  function dataCap(v) { var t = L[v]; return t[1] * t[2] + t[3] * t[4]; }
  function utf8(s) { return Array.prototype.slice.call(new TextEncoder().encode(s)); }

  function encodeData(bytes, v) {
    var bits = [];
    function put(val, len) { for (var i = len - 1; i >= 0; i--) bits.push((val >>> i) & 1); }
    put(4, 4); put(bytes.length, v < 10 ? 8 : 16);
    bytes.forEach(function (b) { put(b, 8); });
    var cap = dataCap(v) * 8;
    put(0, Math.min(4, cap - bits.length));
    while (bits.length % 8) bits.push(0);
    var out = [];
    for (var i = 0; i < bits.length; i += 8) {
      var b = 0; for (var k = 0; k < 8; k++) b = (b << 1) | bits[i + k]; out.push(b);
    }
    for (var pad = 0xec; out.length < dataCap(v); pad ^= 0xec ^ 0x11) out.push(pad);
    return out;
  }
  function interleave(data, v) {
    var t = L[v], ecLen = t[0], blocks = [], pos = 0, i;
    for (i = 0; i < t[1] + t[3]; i++) {
      var n = i < t[1] ? t[2] : t[4];
      var d = data.slice(pos, pos + n); pos += n;
      blocks.push({ d: d, e: rsRem(d, ecLen) });
    }
    var out = [], maxD = t[3] ? t[4] : t[2];
    for (i = 0; i < maxD; i++) blocks.forEach(function (b) { if (i < b.d.length) out.push(b.d[i]); });
    for (i = 0; i < ecLen; i++) blocks.forEach(function (b) { out.push(b.e[i]); });
    return out;
  }
  function alignPos(v) {
    if (v === 1) return [];
    var n = Math.floor(v / 7) + 2, size = v * 4 + 17;
    var step = v === 32 ? 26 : Math.ceil((v * 4 + 4) / (n * 2 - 2)) * 2;
    var r = [6];
    for (var p = size - 7; r.length < n; p -= step) r.splice(1, 0, p);
    return r;
  }
  function bch(val, poly, bitsLen, shift) {
    var r = val << shift;
    for (var i = bitsLen; i >= shift; i--) if ((r >>> i) & 1) r ^= poly << (i - shift);
    return r;
  }
  function build(v, codewords) {
    var size = v * 4 + 17, m = [], fn = [], i, j;
    for (i = 0; i < size; i++) { m.push(new Array(size).fill(false)); fn.push(new Array(size).fill(false)); }
    function set(x, y, dark) { m[y][x] = dark; fn[y][x] = true; }
    function finder(cx, cy) {
      for (var dy = -4; dy <= 4; dy++) for (var dx = -4; dx <= 4; dx++) {
        var x = cx + dx, y = cy + dy; if (x < 0 || y < 0 || x >= size || y >= size) continue;
        var d = Math.max(Math.abs(dx), Math.abs(dy)); set(x, y, d !== 2 && d !== 4);
      }
    }
    finder(3, 3); finder(size - 4, 3); finder(3, size - 4);
    for (i = 8; i < size - 8; i++) { set(6, i, i % 2 === 0); set(i, 6, i % 2 === 0); }
    var ap = alignPos(v);
    ap.forEach(function (ay) { ap.forEach(function (ax) {
      if ((ax === 6 && ay === 6) || (ax === 6 && ay === size - 7) || (ax === size - 7 && ay === 6)) return;
      for (var dy = -2; dy <= 2; dy++) for (var dx = -2; dx <= 2; dx++)
        set(ax + dx, ay + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
    }); });
    // Reserve format + version areas (drawn properly later).
    function drawFormat(mask, target) {
      var keep = m; if (target) m = target;
      var data = (1 << 3) | mask; // level L = 01
      var f = ((data << 10) | bch(data, 0x537, 14, 10)) ^ 0x5412;
      function bit(k) { return ((f >>> k) & 1) !== 0; }
      for (var k = 0; k <= 5; k++) set(8, k, bit(k));
      set(8, 7, bit(6)); set(8, 8, bit(7)); set(7, 8, bit(8));
      for (k = 9; k < 15; k++) set(14 - k, 8, bit(k));
      for (k = 0; k < 8; k++) set(size - 1 - k, 8, bit(k));
      for (k = 8; k < 15; k++) set(8, size - 15 + k, bit(k));
      set(8, size - 8, true);
      m = keep;
    }
    drawFormat(0);
    if (v >= 7) {
      var vi = (v << 12) | bch(v, 0x1f25, 17, 12);
      for (i = 0; i < 18; i++) {
        var b = ((vi >>> i) & 1) !== 0, a = size - 11 + (i % 3), c = Math.floor(i / 3);
        set(a, c, b); set(c, a, b);
      }
    }
    // Place data bits in the zigzag.
    var bits = [];
    codewords.forEach(function (cw) { for (var k = 7; k >= 0; k--) bits.push((cw >>> k) & 1); });
    var idx = 0, up = true;
    for (var x = size - 1; x >= 1; x -= 2) {
      if (x === 6) x = 5;
      for (var n = 0; n < size; n++) {
        var y = up ? size - 1 - n : n;
        for (var s = 0; s < 2; s++) {
          var xx = x - s;
          if (!fn[y][xx]) { m[y][xx] = idx < bits.length ? bits[idx] === 1 : false; idx++; }
        }
      }
      up = !up;
    }
    return { m: m, fn: fn, size: size, drawFormat: drawFormat };
  }
  function maskBit(k, x, y) {
    switch (k) {
      case 0: return (x + y) % 2 === 0; case 1: return y % 2 === 0; case 2: return x % 3 === 0;
      case 3: return (x + y) % 3 === 0; case 4: return (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0;
      case 5: return ((x * y) % 2) + ((x * y) % 3) === 0; case 6: return (((x * y) % 2) + ((x * y) % 3)) % 2 === 0;
      default: return (((x + y) % 2) + ((x * y) % 3)) % 2 === 0;
    }
  }
  function penalty(m, size) {
    var p = 0, x, y, run, dark = 0;
    function lines(get) {
      for (var a = 0; a < size; a++) {
        run = 1;
        for (var b = 1; b < size; b++) {
          if (get(a, b) === get(a, b - 1)) { run++; if (run === 5) p += 3; else if (run > 5) p++; } else run = 1;
        }
        for (b = 0; b + 10 < size; b++) {
          var s = ""; for (var k = 0; k < 11; k++) s += get(a, b + k) ? "1" : "0";
          if (s === "10111010000" || s === "00001011101") p += 40;
        }
      }
    }
    lines(function (a, b) { return m[a][b]; });
    lines(function (a, b) { return m[b][a]; });
    for (y = 0; y < size - 1; y++) for (x = 0; x < size - 1; x++) {
      var c = m[y][x]; if (c === m[y][x + 1] && c === m[y + 1][x] && c === m[y + 1][x + 1]) p += 3;
    }
    for (y = 0; y < size; y++) for (x = 0; x < size; x++) if (m[y][x]) dark++;
    p += Math.floor(Math.abs(dark * 20 - size * size * 10) / (size * size)) * 10;
    return p;
  }
  function encode(text, forceMask) {
    var bytes = utf8(text), v = 1;
    while (v <= 40) {
      var need = 4 + (v < 10 ? 8 : 16) + bytes.length * 8;
      if (need <= dataCap(v) * 8) break; v++;
    }
    if (v > 40) throw new Error("text too long for a QR code");
    var qr = build(v, interleave(encodeData(bytes, v), v)), best = null, bestP = Infinity;
    for (var k = 0; k < 8; k++) {
      var t = qr.m.map(function (r, y) { return r.map(function (c, x) { return qr.fn[y][x] ? c : (c !== maskBit(k, x, y)); }); });
      qr.drawFormat(k, t);
      var pen = penalty(t, qr.size);
      if (forceMask != null ? k === forceMask : pen < bestP) { bestP = pen; best = t.map(function (r) { return r.slice(); }); }
    }
    return { modules: best, size: qr.size, version: v };
  }
  window.ebpQrSvg = function (text, opts) {
    opts = opts || {};
    var q = encode(text), mg = opts.margin == null ? 4 : opts.margin, n = q.size + mg * 2, d = [];
    for (var y = 0; y < q.size; y++) for (var x = 0; x < q.size; x++)
      if (q.modules[y][x]) d.push("M" + (x + mg) + " " + (y + mg) + "h1v1h-1z");
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + n + " " + n +
      '" shape-rendering="crispEdges" role="img" aria-label="QR code"><rect width="100%" height="100%" fill="' +
      (opts.light || "#fff") + '"/><path fill="' + (opts.dark || "#000") + '" d="' + d.join("") + '"/></svg>';
  };
  window.ebpQrEncode = encode; // exposed for tests
})();
