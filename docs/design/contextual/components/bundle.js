/* Contextual UI kit: vanilla JS, no dependencies. Exposes window.Contextual. */
(function () {
  'use strict';

  // ---------------------------------------------------------------- utils
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function fmtK(n) { return n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, '') + 'k' : String(n); }
  function fmtN(n) { return n.toLocaleString('en-US'); }
  function dedent(s) {
    var lines = s.replace(/^\n/, '').replace(/\s+$/, '').split('\n');
    var min = Infinity;
    lines.forEach(function (l) { if (l.trim()) min = Math.min(min, l.match(/^ */)[0].length); });
    return lines.map(function (l) { return l.slice(min); }).join('\n');
  }

  // ---------------------------------------------------------------- icons (16px, stroke = currentColor)
  var P = {
    chevronRight: '<path d="M6 4l4 4-4 4"/>',
    chevronDown: '<path d="M4 6l4 4 4-4"/>',
    folder: '<path d="M1.5 4.5a1 1 0 011-1h3.6l1.4 1.5h5.9a1 1 0 011 1v6.5a1 1 0 01-1 1h-10.9a1 1 0 01-1-1z"/>',
    folderOpen: '<path d="M1.5 12.5v-8a1 1 0 011-1h3.6l1.4 1.5h5a1 1 0 011 1v1"/><path d="M1.5 12.5l2-5h11l-2 5z"/>',
    file: '<path d="M4 1.5h5l3.5 3.5v9.5h-8.5z"/><path d="M9 1.5v3.5h3.5"/>',
    copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1"/><path d="M3.5 10.5h-1v-8h8v1"/>',
    wrap: '<path d="M2 4h12M2 8h9.5a2 2 0 010 4H8"/><path d="M9.5 10.5L8 12l1.5 1.5M2 12h3"/>',
    send: '<path d="M2 8l11.5-5.5L11 14l-2.5-4.5z"/><path d="M8.5 9.5l5-7"/>',
    search: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5l3.5 3.5"/>',
    refresh: '<path d="M13 8a5 5 0 11-1.5-3.6"/><path d="M13 2.5v2.5h-2.5"/>',
    warning: '<path d="M8 1.8l6.5 11.7h-13z"/><path d="M8 6.5v3M8 11.5v.2"/>',
    check: '<path d="M3 8.5l3 3 7-7"/>',
    minus: '<path d="M4 8h8"/>',
    plus: '<path d="M8 3v10M3 8h10"/>',
    close: '<path d="M4 4l8 8M12 4l-8 8"/>',
    gear: '<circle cx="8" cy="8" r="2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>',
    undo: '<path d="M5 3L2 6l3 3"/><path d="M2 6h7.5a4 4 0 010 8H6"/>',
    unfold: '<path d="M8 1.5v4M6 3.5l2 2 2-2M8 14.5v-4M6 12.5l2-2 2 2M2 8h12"/>',
    eye: '<path d="M1 8s2.5-5 7-5 7 5 7 5-2.5 5-7 5-7-5-7-5z"/><circle cx="8" cy="8" r="2"/>',
    split: '<rect x="1.5" y="2.5" width="13" height="11" rx="1"/><path d="M8 2.5v11"/>',
    unified: '<rect x="1.5" y="2.5" width="13" height="11" rx="1"/><path d="M4 6h8M4 10h8"/>',
    back: '<path d="M10 3L5 8l5 5"/>',
    history: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.5 1.5"/>',
    sparkle: '<path d="M8 1.5l1.3 4.2L13.5 7 9.3 8.3 8 12.5 6.7 8.3 2.5 7l4.2-1.3z"/>',
    book: '<path d="M2 3h4.5A1.5 1.5 0 018 4.5v9A1.5 1.5 0 006.5 12H2zM14 3H9.5A1.5 1.5 0 008 4.5v9A1.5 1.5 0 019.5 12H14z"/>',
    files: '<path d="M5.5 1.5h4l3 3v7h-7z"/><path d="M9.5 1.5v3h3M3.5 4.5v10h6"/>',
    key: '<circle cx="5" cy="11" r="3"/><path d="M7.2 8.8l6.3-6.3M11.5 4.5l2 2"/>',
    filter: '<path d="M2 3h12l-4.5 5.5v4.5l-3 1.5v-6z"/>',
    external: '<path d="M9 2.5h4.5V7M13.5 2.5L7.5 8.5M12 9.5v4h-9.5v-9.5h4"/>',
    dots: '<circle cx="3.5" cy="8" r=".6"/><circle cx="8" cy="8" r=".6"/><circle cx="12.5" cy="8" r=".6"/>',
    trash: '<path d="M2.5 4h11M6 4V2.5h4V4M4 4l.7 9.5h6.6L12 4"/>',
    cloud: '<path d="M4.5 12.5a3 3 0 01-.4-6 4 4 0 017.7-.9 3.2 3.2 0 01-.3 6.9z"/>',
    branch: '<circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5.5" r="1.5"/><path d="M4.5 5v6M11.5 7c0 3-7 2-7 4"/>'
  };
  function icon(name, cls) {
    return '<svg class="cx-ic' + (cls ? ' ' + cls : '') + '" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + (P[name] || '') + '</svg>';
  }

  // ---------------------------------------------------------------- language detection
  var EXT = { cs: 'csharp', ts: 'typescript', tsx: 'typescript', js: 'typescript', jsx: 'typescript', mjs: 'typescript', py: 'python', json: 'json', yml: 'yaml', yaml: 'yaml', xml: 'xml', csproj: 'xml', props: 'xml', targets: 'xml', sln: 'plain', sql: 'sql', md: 'markdown', mdc: 'markdown', sh: 'shell', bash: 'shell', ps1: 'shell' };
  var LANG_LABEL = { csharp: 'C#', typescript: 'TypeScript', python: 'Python', json: 'JSON', yaml: 'YAML', xml: 'XML', sql: 'SQL', markdown: 'Markdown', shell: 'Shell', plain: 'Text' };
  function langFromPath(p) {
    if (!p) return 'plain';
    var m = /\.([A-Za-z0-9]+)$/.exec(p);
    if (/\.gitignore$/.test(p)) return 'shell';
    return (m && EXT[m[1].toLowerCase()]) || 'plain';
  }
  function fileGlyph(name) {
    var lang = langFromPath(name);
    var map = { csharp: ['C#', 'cs'], json: ['{ }', 'json'], xml: ['&lt;/&gt;', 'xml'], markdown: ['M&#8595;', 'md'], yaml: ['!', 'xml'], typescript: ['TS', 'md'], python: ['py', 'md'], shell: ['$', 'muted'], sql: ['db', 'json'] };
    if (/\.sln$/.test(name)) return '<span class="cx-fg cx-fg-xml">sln</span>';
    if (/\.(png|jpg|jpeg|gif|ico|dll|exe|pdb)$/i.test(name)) return '<span class="cx-fg cx-fg-muted">' + icon('file') + '</span>';
    var g = map[lang];
    return g ? '<span class="cx-fg cx-fg-' + g[1] + '">' + g[0] + '</span>' : '<span class="cx-fg cx-fg-muted">' + icon('file') + '</span>';
  }

  // ---------------------------------------------------------------- tokenizer
  function kwRe(words, flags) { return new RegExp('\\b(?:' + words.split(' ').join('|') + ')\\b', 'y' + (flags || '')); }
  var CS_CTL = 'if else return await throw for foreach while do switch case break continue try catch finally yield when goto';
  var CS_KW = 'using namespace public private protected internal static readonly sealed class record struct interface enum void var new async this base null true false is not and or get set init required override virtual abstract const in out ref params partial where typeof nameof default string int bool decimal long double float object char byte short uint ulong dynamic operator implicit explicit event delegate global with';
  var TS_CTL = 'if else return await throw for while do switch case break continue try catch finally yield of';
  var TS_KW = 'import export from const let var function class interface type extends implements new async this super null undefined true false typeof instanceof in as public private readonly static enum declare keyof void string number boolean any unknown never';
  var PY_CTL = 'if elif else return await raise for while try except finally with yield pass break continue';
  var PY_KW = 'def class import from as None True False and or not in is lambda async global nonlocal self';
  var SQL_KW = 'select from where join inner left right outer on and or not null is insert into values update set delete create table alter add drop index primary key foreign references default as order by group having limit offset top distinct case when then else end exists in like between union all with begin commit transaction varchar nvarchar int bigint uniqueidentifier datetimeoffset decimal bit constraint';

  var TYPE_AFTER = /^(new|typeof|nameof|is|as|class|record|struct|interface|enum|where)$/;
  function ident(lang) {
    return function (src, pos, prev, out) {
      var m = /[A-Za-z_@][A-Za-z0-9_]*/y; m.lastIndex = pos;
      var r = m.exec(src); if (!r) return 0;
      var w = r[0], after = src.slice(pos + w.length), before = src.slice(0, pos).replace(/\s+$/, '');
      var cls = 'var';
      var isCall = /^\s*(<[A-Za-z0-9_, <>?]*>)?\s*\(/.test(after);
      var dotted = /\.$/.test(before);
      var prevWord = (before.match(/([A-Za-z_]+)$/) || [])[1] || '';
      var pascal = /^[A-Z]/.test(w);
      if (lang === 'csharp' || lang === 'typescript') {
        if (/^\s*(using|namespace)\s+[\w.]*$/.test(before) || prevWord === 'namespace' || (prevWord === 'using' && lang === 'csharp' && !/^\s*(var|\()/.test(after) && !/^var$/.test(w))) cls = 'plain';
        else if (isCall) cls = 'fn';
        else if (/^I[A-Z]/.test(w) && !dotted) cls = 'type';
        else if (pascal && dotted) cls = 'var';
        else if (pascal) {
          if (/^\s*\./.test(after) || /^\s*</.test(after) || /^\??\s*\]/.test(after) || /^\??\s+(?!is\b|and\b|or\b|not\b|with\b|when\b)[A-Za-z_@]/.test(after) || /[<:,(\[]$/.test(before) && /^\??\s*[>,)\]\s]/.test(after) || TYPE_AFTER.test(prevWord)) cls = 'type';
          else cls = 'var';
        }
      } else if (lang === 'python') {
        if (prevWord === 'def' || isCall) cls = 'fn';
        else if (prevWord === 'class' || pascal) cls = 'type';
      }
      out.push({ t: w, c: cls });
      return w.length;
    };
  }

  var RULES = {
    csharp: [
      [/\/\/.*/y, 'com'], [/\/\*.*?\*\//y, 'com'],
      [/\$?@?"(?:[^"\\]|\\.)*"/y, 'str'], [/'(?:[^'\\]|\\.)'/y, 'str'],
      [/\b\d[\d_]*(?:\.\d+)?[mMfFdDlLuU]?\b/y, 'num'],
      [/#(?:region|endregion|nullable|if|endif|pragma).*/y, 'kw'],
      [kwRe(CS_CTL), 'ctl'], [kwRe(CS_KW), 'kw'],
      ['ident']
    ],
    typescript: [
      [/\/\/.*/y, 'com'], [/\/\*.*?\*\//y, 'com'],
      [/`(?:[^`\\]|\\.)*`/y, 'str'], [/"(?:[^"\\]|\\.)*"/y, 'str'], [/'(?:[^'\\]|\\.)*'/y, 'str'],
      [/\b\d[\d_]*(?:\.\d+)?\b/y, 'num'],
      [kwRe(TS_CTL), 'ctl'], [kwRe(TS_KW), 'kw'],
      ['ident']
    ],
    python: [
      [/#.*/y, 'com'], [/@[\w.]+/y, 'fn'],
      [/[rbf]?"""[\s\S]*?"""|[rbf]?"(?:[^"\\]|\\.)*"|[rbf]?'(?:[^'\\]|\\.)*'/y, 'str'],
      [/\b\d[\d_]*(?:\.\d+)?\b/y, 'num'],
      [kwRe(PY_CTL), 'ctl'], [kwRe(PY_KW), 'kw'],
      ['ident']
    ],
    json: [
      [/"(?:[^"\\]|\\.)*"(?=\s*:)/y, 'var'], [/"(?:[^"\\]|\\.)*"/y, 'str'],
      [/-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/y, 'num'], [/\b(?:true|false|null)\b/y, 'kw'],
      [/\/\/.*/y, 'com']
    ],
    xml: [
      [/<!--.*?-->/y, 'com'], [/<\?[\w-]+/y, 'tag'],
      [/(<\/?)([\w.:-]+)/y, ['pun', 'tag']], [/([\w.:-]+)(=)/y, ['var', 'pun']],
      [/"[^"]*"/y, 'str'], [/\/?>|\?>/y, 'pun']
    ],
    yaml: [
      [/#.*/y, 'com'], [/^(\s*-?\s*)([\w.\-/]+)(:)/y, ['plain', 'head', 'pun']],
      [/"(?:[^"\\]|\\.)*"|'[^']*'/y, 'str'], [/\b(?:true|false|null|yes|no)\b/y, 'kw'],
      [/\b\d+(?:\.\d+)?\b/y, 'num']
    ],
    sql: [
      [/--.*/y, 'com'], [/'(?:[^']|'')*'/y, 'str'], [/\b\d+(?:\.\d+)?\b/y, 'num'],
      [kwRe(SQL_KW, 'i'), 'kw'], [/@\w+/y, 'var'], [/\[[^\]]+\]/y, 'type']
    ],
    markdown: [
      [/^#{1,6} .*/y, 'head'], [/`[^`]+`/y, 'str'], [/\*\*[^*]+\*\*/y, 'kw'],
      [/^\s*(?:[-*]|\d+\.) /y, 'ctl'], [/\[[^\]]+\]\([^)]+\)/y, 'fn'], [/^```.*/y, 'com'], [/^> .*/y, 'com']
    ],
    shell: [
      [/#.*/y, 'com'], [/"(?:[^"\\]|\\.)*"|'[^']*'/y, 'str'], [/\s--?[\w-]+/y, 'kw'],
      [/\$\w+|\$\{[^}]+\}/y, 'var'], [/^(\s*)(dotnet|git|npm|cd|docker|az|curl|echo|export)\b/y, ['plain', 'fn']]
    ],
    plain: []
  };

  function tokenize(line, lang) {
    var rules = RULES[lang] || [];
    var out = [], pos = 0, buf = '';
    var id = ident(lang);
    function flush() { if (buf) { out.push({ t: buf, c: 'plain' }); buf = ''; } }
    while (pos < line.length) {
      var hit = 0;
      for (var i = 0; i < rules.length && !hit; i++) {
        var r = rules[i];
        if (r[0] === 'ident') {
          var tmp = [];
          var n = /[A-Za-z_@]/.test(line[pos]) && (pos === 0 || !/[A-Za-z0-9_]/.test(line[pos - 1])) ? id(line, pos, null, tmp) : 0;
          if (n) { flush(); out.push.apply(out, tmp); hit = n; }
          continue;
        }
        if (r[0].source.charAt(0) === '^' && pos !== 0) continue;
        r[0].lastIndex = pos;
        var m = r[0].exec(line);
        if (m && m[0].length) {
          flush();
          if (Array.isArray(r[1])) { for (var g = 1; g < m.length; g++) if (m[g]) out.push({ t: m[g], c: r[1][g - 1] }); }
          else out.push({ t: m[0], c: r[1] });
          hit = m[0].length;
        }
      }
      if (!hit) {
        // swallow a plain word at once so keyword rules don't match mid-word
        var w = /[A-Za-z0-9_]+/y; w.lastIndex = pos; var wm = w.exec(line);
        if (wm && rules.length) { buf += wm[0]; pos += wm[0].length; }
        else { buf += line[pos]; pos++; }
      } else pos += hit;
    }
    flush();
    return out;
  }

  // range: [start, end) of changed characters, highlighted as word-diff
  function hlLine(line, lang, range) {
    var toks = tokenize(line, lang), html = '', at = 0;
    toks.forEach(function (tk) {
      var s = at, e = at + tk.t.length;
      var cls = 'cx-t-' + tk.c;
      if (!range || e <= range[0] || s >= range[1]) html += '<span class="' + cls + '">' + esc(tk.t) + '</span>';
      else {
        var a = Math.max(s, range[0]) - s, b = Math.min(e, range[1]) - s;
        if (a > 0) html += '<span class="' + cls + '">' + esc(tk.t.slice(0, a)) + '</span>';
        html += '<span class="' + cls + ' cx-x">' + esc(tk.t.slice(a, b)) + '</span>';
        if (b < tk.t.length) html += '<span class="' + cls + '">' + esc(tk.t.slice(b)) + '</span>';
      }
      at = e;
    });
    return html;
  }

  // ---------------------------------------------------------------- code view (editor)
  function indentLevels(lines) {
    var lv = lines.map(function (l) { return l.trim() ? Math.floor(l.match(/^ */)[0].length / 4) : -1; });
    for (var i = 0; i < lv.length; i++) if (lv[i] < 0) {
      var j = i; while (j < lv.length && lv[j] < 0) j++;
      var prev = i > 0 ? lv[i - 1] : 0, next = j < lv.length ? lv[j] : 0;
      for (var k = i; k < j; k++) lv[k] = Math.min(prev, next);
    }
    return lv;
  }

  function codeLines(code, opts) {
    opts = opts || {};
    var lang = opts.lang || langFromPath(opts.path);
    var lines = code.replace(/\t/g, '    ').split('\n');
    var lv = indentLevels(lines), start = opts.start || 1;
    return lines.map(function (l, i) {
      var n = start + i, cur = n === opts.currentLine;
      var guides = '', rest = l, levels = opts.guides === false ? 0 : lv[i];
      // guides only cover real leading spaces; blank lines get guides only
      var lead = l.match(/^ */)[0].length;
      var g = l.trim() ? Math.min(levels, Math.floor(lead / 4)) : levels;
      for (var k = 0; k < g; k++) guides += '<span class="cx-ig' + (opts.activeGuide === k && cur ? ' is-active' : '') + '">    </span>';
      rest = l.trim() ? l.slice(g * 4) : '';
      var sel = opts.selection && n >= opts.selection[0] && n <= opts.selection[1];
      return '<div class="cx-cl' + (cur ? ' is-current' : '') + (sel ? ' is-sel' : '') + '"><span class="cx-gut">' + n + '</span><span class="cx-src">' + guides + hlLine(rest, lang) + (rest || g ? '' : ' ') + '</span></div>';
    }).join('');
  }

  function codeView(code, opts) {
    opts = opts || {};
    return '<div class="cx-code' + (opts.wrap ? ' is-wrap' : '') + '">' + codeLines(code, opts) + '</div>';
  }

  // chat code block with header bar
  function codeBlock(code, opts) {
    opts = opts || {};
    var lang = opts.lang || langFromPath(opts.path);
    var all = code.split('\n'), shown = all, more = 0;
    if (opts.maxLines && all.length > opts.maxLines) { shown = all.slice(0, opts.maxLines); more = all.length - opts.maxLines; }
    var head = opts.path
      ? fileGlyph(opts.path) + '<span class="cx-cb-path">' + esc(opts.path) + '</span>' + (opts.badge ? opts.badge : '')
      : '<span class="cx-cb-lang">' + LANG_LABEL[lang] + '</span>';
    return '<div class="cx-cb' + (opts.wrap ? ' is-wrap' : '') + '">' +
      '<div class="cx-cb-head">' + head + '<span class="cx-grow"></span>' +
      (opts.path ? '<span class="cx-cb-lang">' + LANG_LABEL[lang] + '</span>' : '') +
      '<button class="cx-btn cx-btn-ghost cx-btn-xs" data-cx="wrap">' + icon('wrap') + 'Wrap lines</button>' +
      '<button class="cx-btn cx-btn-ghost cx-btn-xs" data-cx="copy">' + icon('copy') + 'Copy</button></div>' +
      '<div class="cx-cb-body">' + codeView(shown.join('\n'), { lang: lang, currentLine: opts.currentLine, wrap: opts.wrap }) + '</div>' +
      (more ? '<div class="cx-cb-more">' + icon('unfold') + 'Show ' + more + ' more lines</div>' : '') +
      '</div>';
  }

  function inline(s) { return '<code class="cx-pill">' + esc(s) + '</code>'; }

  // ---------------------------------------------------------------- diff engine
  function lcsOps(a, b) {
    var n = a.length, m = b.length, dp = [];
    for (var i = 0; i <= n; i++) { dp.push(new Array(m + 1).fill(0)); }
    for (i = n - 1; i >= 0; i--) for (var j = m - 1; j >= 0; j--)
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    var ops = []; i = 0; j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && a[i] === b[j]) { ops.push({ k: ' ', a: i + 1, b: j + 1, t: a[i] }); i++; j++; }
      else if (j < m && (i >= n || dp[i][j + 1] >= dp[i + 1][j])) { ops.push({ k: '+', b: j + 1, t: b[j] }); j++; }
      else { ops.push({ k: '-', a: i + 1, t: a[i] }); i++; }
    }
    // normalise: inside each change run put '-' before '+'
    var out = [], run = [];
    function flushRun() { out.push.apply(out, run.filter(function (o) { return o.k === '-'; }).concat(run.filter(function (o) { return o.k === '+'; }))); run = []; }
    ops.forEach(function (o) { if (o.k === ' ') { flushRun(); out.push(o); } else run.push(o); });
    flushRun();
    return out;
  }

  function wordRanges(ops) {
    for (var i = 0; i < ops.length; i++) {
      if (ops[i].k !== '-') continue;
      var d = i; while (d < ops.length && ops[d].k === '-') d++;
      var p = d; while (p < ops.length && ops[p].k === '+') p++;
      var dels = ops.slice(i, d), adds = ops.slice(d, p);
      for (var q = 0; q < Math.min(dels.length, adds.length); q++) {
        var x = dels[q].t, y = adds[q].t, pre = 0, suf = 0;
        while (pre < x.length && pre < y.length && x[pre] === y[pre]) pre++;
        while (suf < x.length - pre && suf < y.length - pre && x[x.length - 1 - suf] === y[y.length - 1 - suf]) suf++;
        var common = pre + suf, longest = Math.max(x.trim().length, y.trim().length);
        if (common - (x.length - x.trimStart().length) > longest * 0.35) {
          dels[q].r = [pre, x.length - suf]; adds[q].r = [pre, y.length - suf];
          dels[q].pair = adds[q]; adds[q].pair = dels[q];
        }
      }
      i = p - 1;
    }
    return ops;
  }

  function makeHunks(ops, ctx) {
    ctx = ctx == null ? 3 : ctx;
    var keep = ops.map(function () { return false; });
    ops.forEach(function (o, i) { if (o.k !== ' ') for (var k = Math.max(0, i - ctx); k <= Math.min(ops.length - 1, i + ctx); k++) keep[k] = true; });
    var items = [], i = 0;
    while (i < ops.length) {
      if (keep[i]) { var h = { lines: [] }; while (i < ops.length && keep[i]) h.lines.push(ops[i++]); items.push(h); }
      else { var g = { gap: 0, from: ops[i] }; while (i < ops.length && !keep[i]) { g.gap++; i++; } items.push(g); }
    }
    return items;
  }

  function hunkHeader(h, newLines) {
    var first = h.lines[0], aS = null, bS = null, aC = 0, bC = 0;
    h.lines.forEach(function (o) { if (o.a) { aS = aS || o.a; aC++; } if (o.b) { bS = bS || o.b; bC++; } });
    var ctxLine = '';
    var startB = first.b || bS || 1;
    for (var k = startB - 2; k >= 0; k--) { if (/^\s*(public|internal|private|protected)\b.*\b(class|record|struct|interface|void|Task|static)\b/.test(newLines[k] || '')) { ctxLine = newLines[k].trim(); break; } }
    return '@@ -' + (aS || 0) + ',' + aC + ' +' + (bS || 0) + ',' + bC + ' @@' + (ctxLine ? ' ' + ctxLine : '');
  }

  function computeDiff(oldText, newText) {
    var a = oldText ? oldText.split('\n') : [], b = newText ? newText.split('\n') : [];
    var ops = wordRanges(lcsOps(a, b));
    var adds = ops.filter(function (o) { return o.k === '+'; }).length, dels = ops.filter(function (o) { return o.k === '-'; }).length;
    return { ops: ops, adds: adds, dels: dels, newLines: b, oldLines: a };
  }

  function statBar(adds, dels) {
    var total = adds + dels || 1, g = Math.round(adds / total * 5), r = Math.min(5 - g, Math.round(dels / total * 5)), html = '';
    for (var i = 0; i < 5; i++) html += '<i class="' + (i < g ? 'is-add' : i < g + r ? 'is-del' : '') + '"></i>';
    return '<span class="cx-stat"><b class="cx-add-ink">+' + adds + '</b><b class="cx-del-ink">&minus;' + dels + '</b><span class="cx-stat-bar">' + html + '</span></span>';
  }

  function statusLetter(s) { return '<span class="cx-st cx-st-' + s + '" title="' + ({ A: 'Added', M: 'Modified', D: 'Deleted' })[s] + '">' + s + '</span>'; }

  function diffBody(d, lang, mode) {
    var items = makeHunks(d.ops, 3);
    var rows = '', split = mode === 'split';
    items.forEach(function (it, idx) {
      if (it.gap) {
        var next = items[idx + 1];
        var hdr = next ? hunkHeader(next, d.newLines) : '';
        rows += '<tr class="cx-d-hunk"><td class="cx-d-no cx-d-exp" colspan="' + (split ? 1 : 2) + '">' + icon('unfold') + '</td><td colspan="' + (split ? 3 : 1) + '"><button class="cx-d-expand">Expand ' + it.gap + ' lines</button>' + (hdr ? '<span class="cx-d-hh">' + esc(hdr) + '</span>' : '') + '</td></tr>';
        return;
      }
      if (idx === 0) rows += '<tr class="cx-d-hunk"><td class="cx-d-no" colspan="' + (split ? 1 : 2) + '"></td><td colspan="' + (split ? 3 : 1) + '"><span class="cx-d-hh">' + esc(hunkHeader(it, d.newLines)) + '</span></td></tr>';
      if (!split) {
        it.lines.forEach(function (o) {
          var cls = o.k === '+' ? 'is-add' : o.k === '-' ? 'is-del' : '';
          rows += '<tr class="cx-d-row ' + cls + '"><td class="cx-d-no">' + (o.a || '') + '</td><td class="cx-d-no">' + (o.b || '') + '</td><td class="cx-d-code"><span class="cx-d-sign">' + (o.k === ' ' ? ' ' : o.k === '-' ? '&minus;' : '+') + '</span>' + hlLine(o.t, lang, o.r) + '</td></tr>';
        });
      } else {
        var L = it.lines, i = 0;
        while (i < L.length) {
          if (L[i].k === ' ') { var o = L[i++]; rows += '<tr class="cx-d-row"><td class="cx-d-no">' + o.a + '</td><td class="cx-d-code">' + hlLine(o.t, lang) + '</td><td class="cx-d-no cx-d-mid">' + o.b + '</td><td class="cx-d-code">' + hlLine(o.t, lang) + '</td></tr>'; continue; }
          var dl = [], ad = [];
          while (i < L.length && L[i].k === '-') dl.push(L[i++]);
          while (i < L.length && L[i].k === '+') ad.push(L[i++]);
          for (var q = 0; q < Math.max(dl.length, ad.length); q++) {
            var x = dl[q], y = ad[q];
            rows += '<tr class="cx-d-row">' +
              (x ? '<td class="cx-d-no is-del">' + x.a + '</td><td class="cx-d-code is-del"><span class="cx-d-sign">&minus;</span>' + hlLine(x.t, lang, x.r) + '</td>' : '<td class="cx-d-no is-empty"></td><td class="cx-d-code is-empty"></td>') +
              (y ? '<td class="cx-d-no cx-d-mid is-add">' + y.b + '</td><td class="cx-d-code is-add"><span class="cx-d-sign">+</span>' + hlLine(y.t, lang, y.r) + '</td>' : '<td class="cx-d-no cx-d-mid is-empty"></td><td class="cx-d-code is-empty"></td>') +
              '</tr>';
          }
        }
      }
    });
    return '<table class="cx-d-table' + (split ? ' is-split' : '') + '">' + (split ? '<colgroup><col class="cx-d-cno"><col><col class="cx-d-cno"><col></colgroup>' : '<colgroup><col class="cx-d-cno"><col class="cx-d-cno"><col></colgroup>') + '<tbody>' + rows + '</tbody></table>';
  }

  // file: {path, status, old, new, conflict, include, collapsed}
  function diffFile(f, opts) {
    opts = opts || {};
    var d = computeDiff(f.old, f.new), lang = langFromPath(f.path);
    var dir = f.path.replace(/[^/]+$/, ''), name = f.path.slice(dir.length);
    var collapsed = f.collapsed;
    var html = '<section class="cx-df' + (collapsed ? ' is-collapsed' : '') + (f.conflict ? ' has-conflict' : '') + '">' +
      '<header class="cx-df-head">' +
      '<button class="cx-iconbtn" data-cx="collapse" aria-label="Collapse">' + icon(collapsed ? 'chevronRight' : 'chevronDown') + '</button>' +
      statusLetter(f.status) +
      '<span class="cx-df-path"><span class="cx-muted">' + esc(dir) + '</span>' + esc(name) + '</span>' +
      (f.status === 'A' ? '<span class="cx-tag">new file</span>' : f.status === 'D' ? '<span class="cx-tag cx-tag-danger">deleted</span>' : '') +
      (f.conflict ? '<span class="cx-tag cx-tag-warn">' + icon('warning') + 'conflict</span>' : '') +
      statBar(d.adds, d.dels) +
      '<span class="cx-grow"></span>' +
      (f.status !== 'D' ? '<button class="cx-btn cx-btn-ghost cx-btn-xs">' + icon('copy') + 'Copy file content</button>' : '') +
      '<button class="cx-btn cx-btn-ghost cx-btn-xs">' + icon('copy') + 'Copy diff</button>' +
      '<label class="cx-check-label">' + checkbox(f.include ? 'on' : 'off') + 'Include in apply</label>' +
      '</header>';
    if (f.conflict && !collapsed) html += '<div class="cx-banner cx-banner-warn">' + icon('warning') + '<div><b>Conflict: file modified on disk since context was loaded.</b> <span class="cx-muted">' + esc(f.conflict) + '</span></div><span class="cx-grow"></span><button class="cx-btn cx-btn-xs">' + icon('eye') + 'Compare with disk</button><button class="cx-btn cx-btn-xs">' + icon('refresh') + 'Reload &amp; resend</button></div>';
    if (!collapsed) html += '<div class="cx-df-body">' + diffBody(d, lang, opts.mode || 'unified') + '</div>';
    return html + '</section>';
  }

  // ---------------------------------------------------------------- controls
  function checkbox(state, label) {
    var inner = state === 'on' ? icon('check') : state === 'mixed' ? icon('minus') : '';
    return '<span class="cx-cbx is-' + state + '" role="checkbox" aria-checked="' + (state === 'on' ? 'true' : state === 'mixed' ? 'mixed' : 'false') + '">' + inner + '</span>' + (label ? '<span>' + label + '</span>' : '');
  }
  function toggle(on) { return '<span class="cx-switch' + (on ? ' is-on' : '') + '" role="switch" aria-checked="' + !!on + '"><i></i></span>'; }
  function segmented(items, active, cls) {
    return '<span class="cx-seg' + (cls ? ' ' + cls : '') + '">' + items.map(function (it, i) {
      var label = typeof it === 'string' ? it : it[1], ic = typeof it === 'string' ? '' : icon(it[0]);
      return '<button class="' + (i === active ? 'is-active' : '') + '">' + ic + label + '</button>';
    }).join('') + '</span>';
  }
  function button(label, opts) {
    opts = opts || {};
    return '<button class="cx-btn' + (opts.primary ? ' cx-btn-primary' : '') + (opts.ghost ? ' cx-btn-ghost' : '') + (opts.size ? ' cx-btn-' + opts.size : '') + '">' + (opts.icon ? icon(opts.icon) : '') + label + (opts.kbd ? '<kbd>' + opts.kbd + '</kbd>' : '') + '</button>';
  }

  // ---------------------------------------------------------------- sample project
  var S = {};
  S.project = { name: 'Inventory.Api', path: 'C:\\src\\inventory-api', branch: 'feature/order-cancel' };

  S.orderOld = dedent(`
    using Inventory.Domain.Common;

    namespace Inventory.Domain.Orders;

    /// <summary>
    /// Aggregate root for a customer order. All state changes go through methods on this type.
    /// </summary>
    public sealed class Order : Entity<Guid>
    {
        private readonly List<OrderLine> _lines = [];

        private Order(Guid id, Guid customerId, DateTimeOffset placedAt) : base(id)
        {
            CustomerId = customerId;
            PlacedAt = placedAt;
            Status = OrderStatus.Pending;
        }

        public Guid CustomerId { get; }
        public DateTimeOffset PlacedAt { get; }
        public DateTimeOffset? ShippedAt { get; private set; }
        public OrderStatus Status { get; private set; }
        public IReadOnlyList<OrderLine> Lines => _lines;

        public decimal Total => _lines.Sum(line => line.UnitPrice * line.Quantity);

        public static Order Place(Guid customerId, DateTimeOffset placedAt) =>
            new(Guid.NewGuid(), customerId, placedAt);

        public void AddLine(Guid productId, int quantity, decimal unitPrice)
        {
            if (Status != OrderStatus.Pending)
                throw new DomainException("Lines can only be added to pending orders.");

            if (quantity <= 0)
                throw new DomainException("Quantity must be positive.");

            _lines.Add(new OrderLine(productId, quantity, unitPrice));
        }

        public void MarkShipped(DateTimeOffset shippedAt)
        {
            Status = OrderStatus.Shipped;
            ShippedAt = shippedAt;
        }
    }`);

  S.orderNew = dedent(`
    using Inventory.Domain.Common;

    namespace Inventory.Domain.Orders;

    /// <summary>
    /// Aggregate root for a customer order. All state changes go through methods on this type.
    /// </summary>
    public sealed class Order : Entity<Guid>
    {
        private readonly List<OrderLine> _lines = [];

        private Order(Guid id, Guid customerId, DateTimeOffset placedAt) : base(id)
        {
            CustomerId = customerId;
            PlacedAt = placedAt;
            Status = OrderStatus.Pending;
        }

        public Guid CustomerId { get; }
        public DateTimeOffset PlacedAt { get; }
        public DateTimeOffset? ShippedAt { get; private set; }
        public DateTimeOffset? CancelledAt { get; private set; }
        public string? CancellationReason { get; private set; }
        public OrderStatus Status { get; private set; }
        public IReadOnlyList<OrderLine> Lines => _lines;

        public decimal Total => _lines.Sum(line => line.UnitPrice * line.Quantity);

        public static Order Place(Guid customerId, DateTimeOffset placedAt) =>
            new(Guid.NewGuid(), customerId, placedAt);

        public void AddLine(Guid productId, int quantity, decimal unitPrice)
        {
            if (Status is not OrderStatus.Pending)
                throw new DomainException("Lines can only be added to pending orders.");

            if (quantity <= 0)
                throw new DomainException("Quantity must be positive.");

            _lines.Add(new OrderLine(productId, quantity, unitPrice));
        }

        public void MarkShipped(DateTimeOffset shippedAt)
        {
            if (Status is not OrderStatus.Pending)
                throw new DomainException($"Order {Id} cannot be shipped from status {Status}.");

            Status = OrderStatus.Shipped;
            ShippedAt = shippedAt;
        }

        /// <summary>
        /// Cancels the order. Cancelling an already cancelled order is a no-op.
        /// </summary>
        /// <exception cref="DomainException">The order has already shipped.</exception>
        public void Cancel(string reason, DateTimeOffset cancelledAt)
        {
            ArgumentException.ThrowIfNullOrWhiteSpace(reason);

            if (Status is OrderStatus.Shipped)
                throw new DomainException($"Order {Id} has shipped and cannot be cancelled.");

            if (Status is OrderStatus.Cancelled)
                return;

            Status = OrderStatus.Cancelled;
            CancelledAt = cancelledAt;
            CancellationReason = reason;
        }
    }`);

  S.cancelCommand = dedent(`
    using MediatR;

    namespace Inventory.Application.Orders.Commands.CancelOrder;

    /// <summary>
    /// Cancels a pending order.
    /// </summary>
    /// <param name="OrderId">The order to cancel.</param>
    /// <param name="Reason">Free-text reason, shown to support staff.</param>
    /// <returns><c>true</c> if the order was cancelled; <c>false</c> if it does not exist.</returns>
    public sealed record CancelOrderCommand(Guid OrderId, string Reason) : IRequest<bool>;`);

  S.cancelHandler = dedent(`
    using Inventory.Application.Abstractions;
    using MediatR;

    namespace Inventory.Application.Orders.Commands.CancelOrder;

    /// <summary>
    /// Handles <see cref="CancelOrderCommand"/> by loading the aggregate and calling
    /// <see cref="Domain.Orders.Order.Cancel"/>. Domain rules stay in the domain.
    /// </summary>
    internal sealed class CancelOrderHandler(
        IOrderRepository orders,
        IUnitOfWork unitOfWork,
        TimeProvider clock) : IRequestHandler<CancelOrderCommand, bool>
    {
        public async Task<bool> Handle(CancelOrderCommand request, CancellationToken cancellationToken)
        {
            var order = await orders.GetByIdAsync(request.OrderId, cancellationToken);
            if (order is null)
            {
                return false;
            }

            order.Cancel(request.Reason, clock.GetUtcNow());
            await unitOfWork.SaveChangesAsync(cancellationToken);

            return true;
        }
    }`);

  S.endpointsOld = dedent(`
    using Inventory.Application.Orders.Commands.CreateOrder;
    using Inventory.Application.Orders.Queries.GetOrderById;
    using MediatR;

    namespace Inventory.Api.Endpoints;

    public static class OrderEndpoints
    {
        public static IEndpointRouteBuilder MapOrderEndpoints(this IEndpointRouteBuilder app)
        {
            var group = app.MapGroup("/orders").WithTags("Orders");

            group.MapGet("/{id:guid}", async (Guid id, ISender sender, CancellationToken ct) =>
            {
                var order = await sender.Send(new GetOrderByIdQuery(id), ct);
                return order is null ? Results.NotFound() : Results.Ok(order);
            })
            .WithName("GetOrderById");

            group.MapPost("/", async (CreateOrderCommand command, ISender sender, CancellationToken ct) =>
            {
                var id = await sender.Send(command, ct);
                return Results.CreatedAtRoute("GetOrderById", new { id }, new { id });
            });

            return app;
        }
    }`);

  S.endpointsNew = dedent(`
    using Inventory.Application.Orders.Commands.CancelOrder;
    using Inventory.Application.Orders.Commands.CreateOrder;
    using Inventory.Application.Orders.Queries.GetOrderById;
    using MediatR;
    using Microsoft.AspNetCore.Mvc;

    namespace Inventory.Api.Endpoints;

    public static class OrderEndpoints
    {
        public static IEndpointRouteBuilder MapOrderEndpoints(this IEndpointRouteBuilder app)
        {
            var group = app.MapGroup("/orders").WithTags("Orders");

            group.MapGet("/{id:guid}", async (Guid id, ISender sender, CancellationToken ct) =>
            {
                var order = await sender.Send(new GetOrderByIdQuery(id), ct);
                return order is null ? Results.NotFound() : Results.Ok(order);
            })
            .WithName("GetOrderById");

            group.MapPost("/", async (CreateOrderCommand command, ISender sender, CancellationToken ct) =>
            {
                var id = await sender.Send(command, ct);
                return Results.CreatedAtRoute("GetOrderById", new { id }, new { id });
            });

            group.MapPost("/{id:guid}/cancel", async (Guid id, [FromQuery] string reason, ISender sender, CancellationToken ct) =>
            {
                var cancelled = await sender.Send(new CancelOrderCommand(id, reason), ct);
                return cancelled ? Results.NoContent() : Results.NotFound();
            })
            .WithName("CancelOrder")
            .ProducesProblem(StatusCodes.Status409Conflict);

            return app;
        }
    }`);

  S.legacyOld = dedent(`
    using Inventory.Application.Abstractions;
    using Inventory.Domain.Orders;

    namespace Inventory.Application.Orders.Commands.LegacyCancel;

    [Obsolete("Use CancelOrderCommand instead.")]
    public sealed class LegacyCancelHandler(IOrderRepository orders, IUnitOfWork unitOfWork)
    {
        public async Task CancelAsync(Guid orderId, CancellationToken cancellationToken = default)
        {
            var order = await orders.GetByIdAsync(orderId, cancellationToken)
                ?? throw new InvalidOperationException($"Order {orderId} not found.");

            // TODO: bypasses the aggregate and ignores shipped orders.
            typeof(Order).GetProperty(nameof(Order.Status))!
                .SetValue(order, OrderStatus.Cancelled);

            await unitOfWork.SaveChangesAsync(cancellationToken);
        }
    }`);

  S.appsettings = dedent(`
    {
      "ConnectionStrings": {
        "Inventory": "Server=localhost,1433;Database=Inventory;User Id=sa;TrustServerCertificate=True"
      },
      "Logging": {
        "LogLevel": {
          "Default": "Information",
          "Microsoft.AspNetCore": "Warning"
        }
      },
      "Orders": {
        "CancellationWindowHours": 24,
        "AllowCancelAfterPicking": false
      },
      "AllowedHosts": "*"
    }`);

  S.csproj = dedent(`
    <Project Sdk="Microsoft.NET.Sdk">

      <PropertyGroup>
        <TargetFramework>net8.0</TargetFramework>
        <Nullable>enable</Nullable>
        <ImplicitUsings>enable</ImplicitUsings>
        <RootNamespace>Inventory.Application</RootNamespace>
      </PropertyGroup>

      <ItemGroup>
        <PackageReference Include="MediatR" Version="12.4.1" />
        <PackageReference Include="FluentValidation" Version="11.10.0" />
      </ItemGroup>

      <ItemGroup>
        <ProjectReference Include="..\\Inventory.Domain\\Inventory.Domain.csproj" />
      </ItemGroup>

    </Project>`);

  S.agentsMd = dedent(`
    # AGENTS.md — Inventory.Api

    ## Architecture
    - Clean architecture: Domain → Application → Infrastructure → Api.
    - Domain has no dependencies. Business rules live on aggregates, not in handlers.
    - One type per file. File name matches the type name.

    ## Conventions
    - Commands and queries are MediatR \`IRequest<T>\` records in \`Application/<Feature>/Commands|Queries/<Name>/\`.
    - Handlers are \`internal sealed\` and use primary constructors.
    - Use \`TimeProvider\`, never \`DateTime.UtcNow\`.
    - Public members get XML doc comments.`);

  S.outputPrompt = dedent(`
    You are helping edit files in a local project. The user reviews every change
    and applies it manually. You cannot write to disk.

    ## Output format
    - Explain the change in a few sentences first.
    - For every file you add or change, return its COMPLETE new contents.
      Never return partial snippets, "..." or "unchanged" placeholders.
    - Put each file in its own fenced block, directly after a header line:

    ### FILE: src/Path/Relative/To/Root.cs
    \`\`\`csharp
    <full file contents>
    \`\`\`

    - To delete a file, write a single line: ### DELETE: src/Path/To/File.cs
    - Only touch files that are included in the context.
    - Follow the project rules provided above this prompt.`);

  S.changes = [
    { path: 'src/Inventory.Domain/Orders/Order.cs', status: 'M', old: S.orderOld, new: S.orderNew, include: true },
    { path: 'src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderCommand.cs', status: 'A', old: '', new: S.cancelCommand, include: true },
    { path: 'src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderHandler.cs', status: 'A', old: '', new: S.cancelHandler, include: true },
    { path: 'src/Inventory.Api/Endpoints/OrderEndpoints.cs', status: 'M', old: S.endpointsOld, new: S.endpointsNew, include: false, conflict: 'Changed 4 min ago (2 lines). The proposal was made against the older version.' },
    { path: 'src/Inventory.Application/Orders/Commands/LegacyCancel/LegacyCancelHandler.cs', status: 'D', old: S.legacyOld, new: '', include: true }
  ];
  S.changes.forEach(function (c) { var d = computeDiff(c.old, c.new); c.adds = d.adds; c.dels = d.dels; });

  // file tree: [name, tokens|children, flags]
  // flags: s = selected, x:reason = excluded, d = changed on disk, open = expanded
  S.tree = [
    ['.claude', [['skills', [['ef-migrations', [['SKILL.md', 610, {}]], {}]], {}]], {}],
    ['.github', [['copilot-instructions.md', 420, { rule: 1 }]], { open: 1 }],
    ['src', [
      ['Inventory.Api', [
        ['bin', null, { x: 'ignored: bin' }],
        ['obj', null, { x: 'ignored: obj' }],
        ['Endpoints', [['OrderEndpoints.cs', 640, { s: 1, d: 1 }], ['ProductEndpoints.cs', 720, {}]], { open: 1 }],
        ['appsettings.Development.json', 140, {}],
        ['appsettings.json', 210, { s: 1 }],
        ['Inventory.Api.csproj', 260, {}],
        ['Program.cs', 980, { s: 1 }]
      ], { open: 1, closed2: 1 }],
      ['Inventory.Application', [
        ['Abstractions', [['IOrderRepository.cs', 230, { s: 1 }], ['IUnitOfWork.cs', 120, { s: 1 }]], { open: 1 }],
        ['Orders', [
          ['Commands', [
            ['CreateOrder', [['CreateOrderCommand.cs', 380, { s: 1 }], ['CreateOrderHandler.cs', 1120, { s: 1 }], ['CreateOrderValidator.cs', 540, {}]], {}],
            ['LegacyCancel', [['LegacyCancelHandler.cs', 410, { s: 1 }]], { open: 1 }]
          ], { open: 1 }],
          ['Queries', [['GetOrderById', [['GetOrderByIdQuery.cs', 160, {}], ['GetOrderByIdHandler.cs', 690, {}], ['OrderDto.cs', 310, {}]], {}]], {}]
        ], { open: 1 }],
        ['DependencyInjection.cs', 520, {}],
        ['Inventory.Application.csproj', 240, {}]
      ], { open: 1, closed2: 1 }],
      ['Inventory.Domain', [
        ['Common', [['DomainException.cs', 150, { s: 1 }], ['Entity.cs', 340, {}]], { open: 1 }],
        ['Orders', [['Order.cs', 1460, { s: 1, active: 1 }], ['OrderLine.cs', 420, {}], ['OrderStatus.cs', 90, { s: 1 }]], { open: 1 }]
      ], { open: 1 }],
      ['Inventory.Infrastructure', [
        ['Persistence', [['InventoryDbContext.cs', 860, { s2: 1 }], ['OrderRepository.cs', 740, { s2: 1 }]], { open2: 1 }],
        ['Migrations', [['20261028_AddOrderStatus.cs', 2400, { s2: 1 }], ['20261028_AddOrderStatus.Designer.cs', 16800, { s2: 1 }], ['InventoryDbContextModelSnapshot.cs', 16900, { s2: 1, big: 1 }]], { open2: 1 }]
      ], { open2: 1 }]
    ], { open: 1 }],
    ['tests', [['Inventory.Domain.Tests', [['OrderTests.cs', 1680, { s: 1 }], ['bin', null, { x: 'ignored: bin' }]], { open: 1 }]], { open: 1 }],
    ['node_modules', null, { x: 'ignored: node_modules' }],
    ['.gitignore', 80, {}],
    ['AGENTS.md', 560, {}],
    ['Inventory.sln', 1900, {}],
    ['Inventory.sln.DotSettings.user', 300, { x: '.gitignore' }],
    ['docs-architecture.png', null, { x: 'binary' }],
    ['packages.lock.json', 48200, { x: 'lock file' }]
  ];

  S.rules = [
    { path: 'AGENTS.md', type: 'Rule', on: true, tokens: 560 },
    { path: '.github/copilot-instructions.md', type: 'Rule', on: true, tokens: 420 },
    { path: '.cursor/rules/csharp-style.mdc', type: 'Rule', on: true, tokens: 380 },
    { path: 'CLAUDE.md', type: 'Rule', on: false, on2: true, tokens: 290 },
    { path: '.claude/skills/ef-migrations/SKILL.md', type: 'Skill', on: false, tokens: 610 }
  ];

  S.models = [
    { name: 'gpt-4.1', label: 'gpt-4.1 (Foundry)', limit: 128000, inP: 2.0, outP: 8.0 },
    { name: 'DeepSeek-V3', label: 'DeepSeek-V3 (Foundry)', limit: 64000, inP: 1.14, outP: 4.56 },
    { name: 'gpt-4.1-mini', label: 'gpt-4.1-mini (Foundry)', limit: 128000, inP: 0.4, outP: 1.6 }
  ];

  // ---------------------------------------------------------------- scenario + totals
  var SCEN = 'base';
  function setScenario(x) { SCEN = x || 'base'; }
  function isSel(f) { return !!(f.s || (SCEN === 'heavy' && f.s2)); }
  function isOpen(f) { return SCEN === 'heavy' ? !!((f.open && !f.closed2) || f.open2) : !!f.open; }
  function ruleOn(r) { return !!(r.on || (SCEN === 'heavy' && r.on2)); }
  var PROMPT_TOK = 168, USER_MSG_TOK = 96, REPLY_TOK = 2140;
  function totals(withHistory) {
    var files = 0, ftok = 0;
    S.tree.forEach(function (n) { var s = nodeStats(n); files += s.sel; ftok += s.selTok; });
    var rules = S.rules.filter(ruleOn), rtok = rules.reduce(function (a, r) { return a + r.tokens; }, 0);
    var tok = ftok + rtok + PROMPT_TOK + (withHistory ? USER_MSG_TOK + REPLY_TOK : USER_MSG_TOK);
    return { files: files, rules: rules.length, fileTokens: ftok, ruleTokens: rtok, tokens: Math.round(tok / 100) * 100 };
  }

  // ---------------------------------------------------------------- file tree
  function nodeStats(n) {
    var ch = n[1], f = n[2];
    if (!Array.isArray(ch)) return { tok: f.x ? 0 : (ch || 0), sel: isSel(f) ? 1 : 0, tot: f.x ? 0 : 1, selTok: isSel(f) ? ch : 0 };
    var r = { tok: 0, sel: 0, tot: 0, selTok: 0 };
    ch.forEach(function (c) { var s = nodeStats(c); r.tok += s.tok; r.sel += s.sel; r.tot += s.tot; r.selTok += s.selTok; });
    return r;
  }
  function treeRows(nodes, depth, opts, path) {
    var html = '';
    function isDirN(n) { return Array.isArray(n[1]) || (n[1] === null && !/\.\w+$/.test(n[0])); }
    nodes.map(function (n, i) { return [n, i]; }).sort(function (a, b) { var fa = isDirN(a[0]), fb = isDirN(b[0]); return fa === fb ? a[1] - b[1] : fa ? -1 : 1; }).map(function (x) { return x[0]; })
      .forEach(function (n) {
        var name = n[0], ch = n[1], f = n[2], isDir = Array.isArray(ch) || (ch === null && !/\.\w+$/.test(name)), full = path + name;
        var st = nodeStats(n);
        var state = f.x ? 'off' : isDir ? (st.sel === 0 ? 'off' : st.sel === st.tot ? 'on' : 'mixed') : (isSel(f) ? 'on' : 'off');
        var open = isDir && isOpen(f) && Array.isArray(ch);
        var pad = 6 + depth * 12;
        var tokens = f.x ? '' : isDir ? (st.selTok ? fmtK(st.selTok) : '') : fmtK(ch);
        var tokCls = (!isDir && isSel(f)) || (isDir && st.selTok) ? (f.big && isSel(f) ? ' is-big' : '') : ' is-dim';
        html += '<div class="cx-tr' + (f.x ? ' is-excluded' : '') + (opts.active === name || f.active && opts.activeFromData ? ' is-active' : '') + (isDir ? ' is-dir' : '') + '" style="padding-left:' + pad + 'px" title="' + esc(f.x ? 'Excluded — ' + f.x : full) + '">' +
          (depth ? '<span class="cx-tr-guides" style="width:' + (depth * 12) + 'px"></span>' : '') +
          '<span class="cx-tr-tw">' + (isDir ? icon(open ? 'chevronDown' : 'chevronRight') : '') + '</span>' +
          checkbox(state) +
          (isDir ? '<span class="cx-fg cx-fg-folder">' + icon(open ? 'folderOpen' : 'folder') + '</span>' : fileGlyph(name)) +
          '<span class="cx-tr-name">' + esc(name) + '</span>' +
          (f.d ? '<span class="cx-disk" title="Changed on disk since it was loaded into context">' + icon('refresh') + '</span>' : '') +
          (f.x ? '<span class="cx-tag cx-tag-x">excluded</span>' : '') +
          '<span class="cx-grow"></span>' +
          (f.x && opts.showReason === name ? '<span class="cx-tip">' + esc(f.x) + '</span>' : '') +
          '<span class="cx-tok' + tokCls + '">' + tokens + '</span>' +
          '</div>';
        if (open) html += treeRows(ch, depth + 1, opts, full + '/');
      });
    return html;
  }
  function fileTree(opts) {
    opts = opts || {};
    var st = { sel: 0, selTok: 0 };
    S.tree.forEach(function (n) { var s = nodeStats(n); st.sel += s.sel; st.selTok += s.selTok; });
    return '<div class="cx-explorer">' +
      '<div class="cx-pane-head"><span class="cx-caption">Explorer</span><span class="cx-grow"></span>' +
      '<button class="cx-iconbtn" title="Refresh changed files">' + icon('refresh') + '</button><button class="cx-iconbtn" title="More">' + icon('dots') + '</button></div>' +
      '<div class="cx-ex-tools">' +
      '<label class="cx-input cx-input-sm">' + icon('search') + '<input placeholder="Filter files" value="' + esc(opts.filter || '') + '"></label>' +
      '<div class="cx-ex-row"><button class="cx-link">Select all</button><button class="cx-link">Clear</button><span class="cx-grow"></span><label class="cx-check-label cx-small">' + toggle(true) + 'Respect .gitignore</label></div>' +
      '</div>' +
      '<div class="cx-ex-root"><span class="cx-tr-tw">' + icon('chevronDown') + '</span><b>' + esc(S.project.name) + '</b><span class="cx-grow"></span><span class="cx-tok">' + st.sel + ' files · ' + fmtK(st.selTok) + '</span></div>' +
      (opts.diskBanner ? '<div class="cx-ex-disk">' + icon('refresh') + '<span>1 file changed on disk</span><span class="cx-grow"></span><button class="cx-link">Refresh</button></div>' : '') +
      '<div class="cx-tree">' + treeRows(S.tree, 0, opts, '') + '</div></div>';
  }

  function rulesPanel(opts) {
    opts = opts || {};
    var on = S.rules.filter(ruleOn);
    return '<div class="cx-rules">' +
      '<div class="cx-pane-head"><span class="cx-tr-tw">' + icon('chevronDown') + '</span><span class="cx-caption">Skills &amp; Rules</span><span class="cx-count">' + on.length + '</span><span class="cx-grow"></span><button class="cx-iconbtn" title="Import from file…">' + icon('plus') + '</button></div>' +
      S.rules.map(function (r) {
        return '<div class="cx-rule' + (opts.preview === r.path ? ' is-active' : '') + (ruleOn(r) ? '' : ' is-off') + '">' + fileGlyph(r.path) +
          '<span class="cx-rule-path" title="' + esc(r.path) + '">' + esc(r.path.split('/').pop()) + (r.path.indexOf('/') > 0 ? ' <span class="cx-muted">' + esc(r.path.replace(/[^/]+$/, '')) + '</span>' : '') + '</span>' +
          '<span class="cx-tag' + (r.type === 'Skill' ? ' cx-tag-skill' : '') + '">' + r.type + '</span>' +
          '<span class="cx-tok">' + fmtK(r.tokens) + '</span>' + toggle(ruleOn(r)) + '</div>';
      }).join('') +
      (opts.preview ? '<div class="cx-rule-preview">' + codeView(S.agentsMd.split('\n').slice(0, opts.previewLines || 9).join('\n'), { lang: 'markdown', guides: false }) + '</div>' : '') +
      '<div class="cx-rules-foot"><button class="cx-btn cx-btn-xs">' + icon('files') + 'Import from file…</button><span class="cx-muted cx-small">Detected: AGENTS.md, CLAUDE.md, .github, .cursor/rules, .claude/skills</span></div>' +
      '</div>';
  }

  // ---------------------------------------------------------------- context bar, composer
  function contextBar(o) {
    o = o || {};
    var m = S.models[o.model || 0];
    var T = totals(o.history !== false);
    var files = o.files == null ? T.files : o.files, rules = o.rules == null ? T.rules : o.rules, tok = o.tokens || T.tokens;
    var pct = tok / m.limit, level = pct >= 0.95 ? 'danger' : pct >= 0.75 ? 'warn' : 'ok';
    var cost = tok / 1e6 * m.inP + 1800 / 1e6 * m.outP;
    return '<div class="cx-ctx is-' + level + '">' +
      '<span class="cx-ctx-label">' + icon('files') + '<b>' + files + ' files</b> · ' + rules + ' rules · ~' + fmtN(tok) + ' tokens</span>' +
      '<span class="cx-ctx-bar" role="meter" aria-valuenow="' + Math.round(pct * 100) + '"><i style="width:' + Math.min(100, pct * 100).toFixed(1) + '%"></i></span>' +
      '<span class="cx-ctx-pct">' + Math.round(pct * 100) + '% of ' + fmtK(m.limit) + '</span>' +
      (level !== 'ok' ? '<span class="cx-ctx-warn">' + icon('warning') + (level === 'warn' ? 'Near limit' : 'Over limit') + '</span>' + (o.hint ? '<span class="cx-ctx-hint">' + o.hint + '</span>' : '') : '') +
      '<span class="cx-grow"></span>' +
      '<span class="cx-ctx-cost" title="Input at $' + m.inP.toFixed(2) + '/1M + ~1.8k output at $' + m.outP.toFixed(2) + '/1M">~$' + cost.toFixed(3) + ' / message</span>' +
      '</div>';
  }

  function composer(o) {
    o = o || {};
    var m = S.models[o.model || 0];
    return '<div class="cx-composer">' + contextBar(o) +
      '<div class="cx-input-box' + (o.focus ? ' is-focus' : '') + '">' +
      '<div class="cx-input-text' + (o.text ? '' : ' is-ph') + '">' + (o.text ? o.text : 'Ask about the selected files…') + (o.focus ? '<span class="cx-caret"></span>' : '') + '</div>' +
      '<div class="cx-input-foot">' +
      '<button class="cx-dd">' + icon('cloud') + esc(m.label) + icon('chevronDown') + '</button>' +
      segmented(['Resend full context', 'Only send new message'], o.mode === 'new' ? 1 : 0, 'cx-seg-sm') +
      '<span class="cx-grow"></span><span class="cx-muted cx-small">Ctrl+Enter</span>' +
      '<button class="cx-btn cx-btn-primary cx-btn-icon" aria-label="Send">' + icon('send') + '</button>' +
      '</div></div></div>';
  }

  // ---------------------------------------------------------------- chat
  function changesBadge() { return '<span class="cx-tag cx-tag-st">' + statusLetter('A') + 'new</span>'; }

  function chatThread(o) {
    o = o || {};
    var T0 = totals(false), m0 = S.models[0], cost0 = T0.tokens / 1e6 * m0.inP + REPLY_TOK / 1e6 * m0.outP;
    var user = '<div class="cx-msg cx-msg-user"><div class="cx-msg-head"><span class="cx-avatar">TL</span><b>You</b><span class="cx-muted">14:31</span><span class="cx-grow"></span><span class="cx-chip">' + icon('files') + T0.files + ' files · ' + T0.rules + ' rules · full context</span></div>' +
      '<div class="cx-prose"><p>Add order cancellation. ' + inline('Order.Cancel()') + ' must throw if the order has shipped, and ' + inline('MarkShipped') + ' should only work from Pending. Add a ' + inline('CancelOrderCommand') + ' with a handler and a ' + inline('POST /orders/{id}/cancel') + ' endpoint. Remove ' + inline('LegacyCancelHandler') + '.</p></div></div>';
    var asst = '<div class="cx-msg cx-msg-ai"><div class="cx-msg-head"><span class="cx-avatar cx-avatar-ai">' + icon('sparkle') + '</span><b>gpt-4.1</b><span class="cx-muted">Foundry · 14:32 · 18.4k in / 2.1k out · $0.054</span><span class="cx-grow"></span>' +
      segmented(['Response', 'Diff'], o.view === 'diff' ? 1 : 0, 'cx-seg-sm') + '</div>' +
      '<div class="cx-prose">' +
      '<p>Cancellation rules now live on the aggregate. ' + inline('Order.Cancel(reason, cancelledAt)') + ' throws a ' + inline('DomainException') + ' for shipped orders and is a no-op when already cancelled. The handler only loads, calls and saves.</p>' +
      '<ul><li>' + statusLetter('M') + ' ' + inline('Order.cs') + ' — adds ' + inline('Cancel') + ', ' + inline('CancelledAt') + ', ' + inline('CancellationReason') + '; guards ' + inline('MarkShipped') + '.</li>' +
      '<li>' + statusLetter('A') + ' ' + inline('CancelOrderCommand.cs') + ', ' + inline('CancelOrderHandler.cs') + ' — one type per file, as AGENTS.md asks.</li>' +
      '<li>' + statusLetter('M') + ' ' + inline('OrderEndpoints.cs') + ' — maps ' + inline('POST /orders/{id:guid}/cancel') + ', 204 / 404 / 409.</li>' +
      '<li>' + statusLetter('D') + ' ' + inline('LegacyCancelHandler.cs') + ' — no remaining references.</li></ul>' +
      '<p class="cx-file-h">### FILE: src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderHandler.cs</p>' +
      codeBlock(S.cancelHandler, { path: 'src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderHandler.cs', maxLines: o.maxLines || 28, badge: '<span class="cx-tag cx-tag-add">' + statusLetter('A') + 'new file</span>' }) +
      '<p class="cx-file-h">### FILE: src/Inventory.Domain/Orders/Order.cs</p>' +
      codeBlock(S.orderNew, { path: 'src/Inventory.Domain/Orders/Order.cs', maxLines: 9, badge: '<span class="cx-tag">' + statusLetter('M') + '+' + S.changes[0].adds + ' &minus;' + S.changes[0].dels + '</span>' }) +
      '<p>Run ' + inline('dotnet test tests/Inventory.Domain.Tests') + ' after applying — ' + inline('OrderTests.MarkShipped_FromCancelled_Throws') + ' will need a new case.</p>' +
      '</div>' +
      '<div class="cx-msg-changes">' + icon('files') + '<b>5 files changed</b><span class="cx-add-ink">+' + total('adds') + '</span><span class="cx-del-ink">&minus;' + total('dels') + '</span><span class="cx-grow"></span>' + button('Review diff', { icon: 'split', size: 'xs' }) + '</div>' +
      '</div>';
    return '<div class="cx-thread">' + '<div class="cx-thread-day">Today</div>' + user + asst + '</div>';
  }
  function total(k) { return S.changes.reduce(function (s, c) { return s + c[k]; }, 0); }

  // ---------------------------------------------------------------- changes panel
  function changesPanel(o) {
    o = o || {};
    var counts = { A: 0, M: 0, D: 0 }; S.changes.forEach(function (c) { counts[c.status]++; });
    var inc = S.changes.filter(function (c) { return c.include; }).length;
    return '<div class="cx-changes">' +
      '<div class="cx-pane-head"><span class="cx-caption">Changes</span><span class="cx-muted cx-small">from response #2 · 14:32</span><span class="cx-grow"></span><button class="cx-iconbtn">' + icon('history') + '</button></div>' +
      '<div class="cx-ch-sum"><div class="cx-ch-title">' + S.changes.length + ' files changed</div>' +
      '<div class="cx-ch-counts"><span>' + statusLetter('A') + counts.A + ' added</span><span>' + statusLetter('M') + counts.M + ' modified</span><span>' + statusLetter('D') + counts.D + ' deleted</span></div>' +
      '<div class="cx-ch-lines"><span class="cx-add-ink">+' + total('adds') + '</span> <span class="cx-del-ink">&minus;' + total('dels') + '</span> lines</div></div>' +
      '<div class="cx-ch-list"><div class="cx-ch-listhead">' + checkbox('mixed') + '<span>' + inc + ' of ' + S.changes.length + ' selected for apply</span></div>' +
      S.changes.map(function (c, i) {
        var dir = c.path.replace(/[^/]+$/, '').replace(/\/$/, ''), name = c.path.slice(dir.length + 1);
        return '<div class="cx-ch-row' + (o.active === i ? ' is-active' : '') + (c.include ? '' : ' is-unsel') + '">' + checkbox(c.include ? 'on' : 'off') + statusLetter(c.status) +
          '<span class="cx-ch-name"><span class="cx-ch-file">' + esc(name) + (c.conflict ? '<span class="cx-warn-ink" title="Conflict: modified on disk">' + icon('warning') + '</span>' : '') + '</span><span class="cx-ch-dir">' + esc(dir.replace(/^src\//, '')) + '</span></span>' +
          '<span class="cx-ch-n"><span class="cx-add-ink">' + (c.adds ? '+' + c.adds : '') + '</span><span class="cx-del-ink">' + (c.dels ? '&minus;' + c.dels : '') + '</span></span></div>';
      }).join('') + '</div>' +
      (o.conflictNote !== false ? '<div class="cx-banner cx-banner-warn cx-banner-sm">' + icon('warning') + '<span><b>OrderEndpoints.cs</b> changed on disk after it was sent. Excluded from apply until you review it.</span></div>' : '') +
      '<div class="cx-ch-foot">' + button('Open diff', { icon: 'split' }) + '<button class="cx-btn cx-btn-primary cx-grow">Apply ' + inc + ' files…</button></div>' +
      '</div>';
  }

  // ---------------------------------------------------------------- window chrome
  function windowFrame(inner, o) {
    o = o || {};
    return '<div class="cx-window" style="' + (o.height ? 'height:' + o.height + 'px' : '') + '">' +
      '<div class="cx-titlebar"><span class="cx-logo">' + logo(16) + '</span>' +
      '<nav class="cx-menu"><span>File</span><span>Edit</span><span>View</span><span>Help</span></nav>' +
      '<span class="cx-grow"></span><span class="cx-title">' + (o.title || (S.project.name + ' — Contextual')) + '</span><span class="cx-grow"></span>' +
      '<span class="cx-winbtns"><i>' + icon('minus') + '</i><i><svg class="cx-ic" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="4" width="8" height="8"/></svg></i><i>' + icon('close') + '</i></span></div>' +
      '<div class="cx-winbody">' + inner + '</div>' +
      (o.status === false ? '' : statusBar(o)) +
      '</div>';
  }
  function statusBar(o) {
    var m = S.models[o.model || 0];
    return '<div class="cx-statusbar">' +
      (o.empty ? '<span>' + icon('cloud') + 'Foundry: connected</span><span class="cx-grow"></span><span>No project</span>' :
        '<span>' + icon('branch') + S.project.branch + '</span><span>' + icon('cloud') + 'Foundry · ' + m.name + '</span><span class="cx-sb-ok">' + icon('check') + 'Read-only: nothing is written until you apply</span><span class="cx-grow"></span><span>' + (o.statusRight || (function () { var t = totals(true); return t.files + ' files · ' + t.rules + ' rules · ~' + fmtN(t.tokens) + ' tokens'; })()) + '</span><span>' + icon('gear') + 'Settings</span>') +
      '</div>';
  }
  function logo(size) {
    size = size || 16;
    return '<svg viewBox="0 0 16 16" width="' + size + '" height="' + size + '" aria-hidden="true" class="cx-logo-svg"><rect x="1" y="1.5" width="9" height="11" rx="2" class="cx-logo-a"/><rect x="6" y="3.5" width="9" height="11" rx="2" class="cx-logo-b"/><path d="M8.5 7.5h4M8.5 10h2.5" class="cx-logo-c"/></svg>';
  }

  function sidebar(o) {
    o = o || {};
    return '<aside class="cx-side">' +
      '<div class="cx-side-tabs"><button class="is-active">' + icon('files') + 'Files</button><button>' + icon('book') + 'Skills &amp; Rules<span class="cx-count">3</span></button></div>' +
      fileTree(o.tree) + rulesPanel(o.rules) + '</aside>';
  }

  function tabs(list, active) {
    return '<div class="cx-tabs">' + list.map(function (t, i) {
      return '<div class="cx-tab' + (i === active ? ' is-active' : '') + '">' + (t.glyph || '') + '<span>' + t.label + '</span>' + (t.ro ? '<span class="cx-tag">read-only</span>' : '') + (t.close ? '<span class="cx-tab-x">' + icon('close') + '</span>' : '') + '</div>';
    }).join('') + '<span class="cx-grow"></span></div>';
  }

  // ---------------------------------------------------------------- screens
  function workspace(o) {
    o = o || {};
    setScenario(o.center === 'preview' ? 'heavy' : 'base');
    var center;
    var tabList = [{ label: 'Chat', glyph: icon('sparkle') }];
    if (o.center === 'preview') {
      tabList.push({ label: 'Order.cs', glyph: fileGlyph('Order.cs'), ro: true, close: true });
      center = tabs(tabList, 1) +
        '<div class="cx-crumbs">' + 'src <span>›</span> Inventory.Domain <span>›</span> Orders <span>›</span> <b>Order.cs</b><span class="cx-grow"></span>' +
        '<span class="cx-muted cx-small">C# · UTF-8 · 1,460 tokens · in context</span>' + button('Copy', { icon: 'copy', size: 'xs', ghost: true }) + button('Wrap lines', { icon: 'wrap', size: 'xs', ghost: true }) + '</div>' +
        '<div class="cx-editor">' + codeView(S.orderOld, { path: 'Order.cs', currentLine: 34, activeGuide: 1 }) + '</div>' +
        composer({ model: 1, hint: 'Migrations: 36k', text: 'Also add an <code class="cx-pill">OrderCancelled</code> domain event and raise it from <code class="cx-pill">Cancel</code>.', focus: true });
    } else {
      tabList.push({ label: 'Order.cs', glyph: fileGlyph('Order.cs'), ro: true, close: true });
      center = tabs(tabList, 0) + '<div class="cx-chat">' + chatThread({ view: 'response' }) + '</div>' + composer({ model: 0, mode: o.mode });
    }
    var right = o.center === 'preview' ? changesPanel({ active: 0 }) : changesPanel({});
    var body = '<div class="cx-ws">' + sidebar(o.side || { tree: { activeFromData: o.center === 'preview', diskBanner: true, showReason: o.showReason } }) +
      '<main class="cx-center">' + center + '</main>' +
      '<aside class="cx-right">' + right + '</aside></div>';
    if (o.overlay === 'diff') body = '<div class="cx-ws">' + sidebar({ tree: { diskBanner: true } }) + '<main class="cx-center cx-center-wide">' + diffScreen(o) + '</main></div>';
    if (o.overlay === 'apply') body += '<div class="cx-scrim">' + applyDialog() + '</div>';
    if (o.toast) body += toast();
    return windowFrame(body, { height: o.height || 900, model: o.center === 'preview' ? 1 : 0, statusRight: null });
  }

  function diffScreen(o) {
    o = o || {};
    var mode = o.mode || 'unified';
    var inc = S.changes.filter(function (c) { return c.include; }).length;
    var head = '<div class="cx-dv-bar">' +
      button('Back to chat', { icon: 'back', ghost: true, size: 'xs' }) +
      '<button class="cx-iconbtn' + ((o.fileList != null ? o.fileList : mode !== 'split') ? ' is-on' : '') + '" data-cx="files" title="Toggle file list">' + icon('files') + '</button>' +
      '<span class="cx-dv-sep"></span><b>Response #2</b><span class="cx-muted">gpt-4.1 · 14:32</span>' +
      '<span class="cx-muted">·</span><span>' + S.changes.length + ' files changed</span>' + statBar(total('adds'), total('dels')) +
      '<span class="cx-grow"></span>' +
      segmented([['unified', 'Unified'], ['split', 'Split']], mode === 'split' ? 1 : 0) +
      button('Expand all', { ghost: true, size: 'xs', icon: 'unfold' }) +
      '<button class="cx-btn cx-btn-primary">Apply ' + inc + ' files…</button></div>';
    var list = '<nav class="cx-dv-files"><div class="cx-pane-head"><span class="cx-caption">Files</span></div>' + S.changes.map(function (c, i) {
      var name = c.path.split('/').pop();
      return '<div class="cx-dv-file' + (i === 0 ? ' is-active' : '') + '">' + checkbox(c.include ? 'on' : 'off') + statusLetter(c.status) + '<span class="cx-ch-file">' + esc(name) + '</span>' + (c.conflict ? '<span class="cx-warn-ink">' + icon('warning') + '</span>' : '') + '<span class="cx-grow"></span><span class="cx-ch-n"><span class="cx-add-ink">' + (c.adds ? '+' + c.adds : '') + '</span><span class="cx-del-ink">' + (c.dels ? '&minus;' + c.dels : '') + '</span></span></div>';
    }).join('') + '</nav>';
    var files = S.changes.map(function (c, i) {
      var f = Object.assign({}, c);
      f.collapsed = o.expandAll ? false : (i === 1 || i === 2 || i === 4);
      return diffFile(f, { mode: mode });
    }).join('');
    // split needs the width: the file list folds away (toggle in the toolbar), as on GitHub
    var showList = o.fileList != null ? o.fileList : mode !== 'split';
    return '<div class="cx-dv">' + head + '<div class="cx-dv-main">' + (showList ? list : '') + '<div class="cx-dv-scroll">' + files + '</div></div></div>';
  }

  function applyDialog() {
    var sel = S.changes.filter(function (c) { return c.include; });
    var verb = { A: ['create', 'success'], M: ['overwrite', 'modified'], D: ['delete', 'danger'] };
    return '<div class="cx-dialog" role="dialog" aria-modal="true">' +
      '<div class="cx-dlg-head"><span class="cx-ui-title">Apply changes</span><span class="cx-grow"></span><button class="cx-iconbtn">' + icon('close') + '</button></div>' +
      '<div class="cx-dlg-body"><p class="cx-muted">These files will be written to <span class="cx-mono">' + esc(S.project.path) + '</span>. Nothing else is touched.</p>' +
      '<div class="cx-apply-list">' + sel.map(function (c) {
        var v = verb[c.status];
        return '<div class="cx-apply-row"><span class="cx-verb cx-verb-' + v[1] + '">' + v[0] + '</span><span class="cx-mono cx-apply-path">' + esc(c.path) + '</span><span class="cx-grow"></span><span class="cx-ch-n"><span class="cx-add-ink">' + (c.adds ? '+' + c.adds : '') + '</span><span class="cx-del-ink">' + (c.dels ? '&minus;' + c.dels : '') + '</span></span></div>';
      }).join('') + '</div>' +
      '<div class="cx-apply-skip">' + icon('warning') + '<span>Skipped: <span class="cx-mono">OrderEndpoints.cs</span> (conflict, not selected)</span></div>' +
      '<label class="cx-check-label">' + checkbox('on') + 'Create backup before overwriting <span class="cx-muted cx-mono cx-small">.contextual/backups/2026-11-05T14-35/</span></label>' +
      '<label class="cx-check-label">' + checkbox('off') + 'Show in explorer after apply</label>' +
      '</div>' +
      '<div class="cx-dlg-foot"><span class="cx-muted cx-small">Writes are exact file contents from the response. No merging.</span><span class="cx-grow"></span>' + button('Cancel') + '<button class="cx-btn cx-btn-primary">Apply ' + sel.length + ' files</button></div>' +
      '</div>';
  }

  function toast(n) {
    n = n || S.changes.filter(function (c) { return c.include; }).length;
    return '<div class="cx-toast" role="status">' + '<span class="cx-toast-ic">' + icon('check') + '</span><div><b>' + n + ' files written</b><div class="cx-muted cx-small">Backup saved · 1 deleted, 2 created, 1 overwritten</div></div>' + '<span class="cx-grow"></span>' + button('Undo', { icon: 'undo', size: 'xs' }) + '<button class="cx-iconbtn">' + icon('close') + '</button></div>';
  }

  function welcome() {
    var recents = [
      ['Inventory.Api', 'C:\\src\\inventory-api', '2 hours ago', '.NET'],
      ['Catalog.Web', 'C:\\src\\catalog-web', 'Yesterday', 'TypeScript'],
      ['scheduler-worker', 'D:\\work\\scheduler-worker', '3 days ago', 'Python'],
      ['Billing.Service', 'C:\\src\\billing-service', 'Oct 28', '.NET'],
      ['infra-bicep', 'C:\\src\\infra-bicep', 'Oct 12', 'Bicep']
    ];
    var body = '<div class="cx-welcome"><div class="cx-wl-inner">' +
      '<div class="cx-wl-brand">' + logo(40) + '<div><div class="cx-ui-display">Contextual</div><div class="cx-wl-tag">Pick files, chat, review diffs, apply what you want.</div></div></div>' +
      '<div class="cx-wl-grid"><div>' +
      '<div class="cx-caption cx-wl-h">Start</div>' +
      '<button class="cx-btn cx-btn-primary cx-btn-lg">' + icon('folderOpen') + 'Open folder…<kbd>Ctrl+O</kbd></button>' +
      '<div class="cx-wl-steps">' +
      '<div><span>1</span>Tick the files the model should see</div>' +
      '<div><span>2</span>Ask for a change in chat</div>' +
      '<div><span>3</span>Review the diff, file by file</div>' +
      '<div><span>4</span>Apply what you want. The model never writes to disk.</div></div>' +
      '<div class="cx-wl-conn">' + icon('cloud') + '<span>Foundry · <b>3 deployments</b></span><span class="cx-grow"></span><button class="cx-link">Settings</button></div>' +
      '</div><div>' +
      '<div class="cx-caption cx-wl-h">Recent</div><div class="cx-recent">' +
      recents.map(function (r, i) { return '<div class="cx-recent-row' + (i === 0 ? ' is-hover' : '') + '">' + '<span class="cx-fg cx-fg-folder">' + icon('folder') + '</span><div class="cx-recent-main"><b>' + r[0] + '</b><span class="cx-mono cx-muted cx-small">' + esc(r[1]) + '</span></div><span class="cx-tag">' + r[3] + '</span><span class="cx-muted cx-small cx-recent-when">' + r[2] + '</span></div>'; }).join('') +
      '</div><button class="cx-link cx-small">Clear recent</button></div></div>' +
      '</div></div>';
    return windowFrame(body, { height: 640, title: 'Contextual', empty: true });
  }

  function settings() {
    var nav = ['Azure AI Foundry', 'Exclusions', 'Output format', 'Appearance', 'Keyboard'];
    var deps = S.models.map(function (m, i) {
      return '<tr><td><span class="cx-radio' + (i === 0 ? ' is-on' : '') + '"></span></td><td class="cx-mono">' + m.name + '</td><td><span class="cx-input cx-input-sm cx-mono">' + fmtN(m.limit) + '</span></td><td><span class="cx-input cx-input-sm cx-mono">$' + m.inP.toFixed(2) + '</span></td><td><span class="cx-input cx-input-sm cx-mono">$' + m.outP.toFixed(2) + '</span></td><td><button class="cx-iconbtn">' + icon('trash') + '</button></td></tr>';
    }).join('');
    var globs = ['**/node_modules/**', '**/.venv/**', '**/venv/**', '**/__pycache__/**', '**/bin/**', '**/obj/**', '**/dist/**', '**/build/**', '**/.git/**', 'package-lock.json', 'yarn.lock', 'pnpm-lock.yaml', 'poetry.lock', 'Pipfile.lock', 'packages.lock.json', 'Cargo.lock'];
    var body = '<div class="cx-settings"><nav class="cx-set-nav"><div class="cx-pane-head"><span class="cx-caption">Settings</span></div>' +
      nav.map(function (n, i) { return '<div class="cx-set-nav-i' + (i === 0 ? ' is-active' : '') + '">' + n + '</div>'; }).join('') +
      '<div class="cx-set-scope"><span class="cx-caption">Scope</span>' + segmented(['User', 'Project'], 0, 'cx-seg-sm') + '</div></nav>' +
      '<div class="cx-set-main">' +
      // foundry
      '<section class="cx-set-sec"><h2 class="cx-ui-title">Azure AI Foundry</h2><p class="cx-muted">Requests go straight from this machine to your deployment. Keys are stored in Windows Credential Manager.</p>' +
      '<div class="cx-field"><label>Endpoint URL</label><span class="cx-input cx-mono">https://tl-foundry-dev.services.ai.azure.com/models</span></div>' +
      '<div class="cx-field-row"><div class="cx-field"><label>API key</label><span class="cx-input cx-mono">••••••••••••••••••••••••3f9a<span class="cx-grow"></span>' + icon('eye') + '</span></div>' +
      '<div class="cx-field"><label>API version</label><span class="cx-input cx-mono">2024-05-01-preview' + icon('chevronDown') + '</span></div>' +
      '<div class="cx-field cx-field-end">' + button('Test connection', { icon: 'refresh' }) + '<span class="cx-ok-ink cx-small">' + icon('check') + 'OK · 212 ms</span></div></div>' +
      '<div class="cx-field"><label>Deployments</label><table class="cx-table"><thead><tr><th>Default</th><th>Deployment name</th><th>Context limit (tokens)</th><th>Input / 1M</th><th>Output / 1M</th><th></th></tr></thead><tbody>' + deps + '</tbody></table>' +
      '<button class="cx-link cx-small">' + icon('plus') + 'Add deployment</button><p class="cx-muted cx-small">The context limit drives the context bar (amber at 75%, red at 95%). Set it below the model maximum to cap cost.</p></div></section>' +
      // exclusions
      '<section class="cx-set-sec"><h2 class="cx-ui-title">Exclusions</h2><p class="cx-muted">Matched paths are shown greyed out in the explorer and are never sent.</p>' +
      '<div class="cx-field"><label>Glob patterns <span class="cx-muted">· one per line</span></label><div class="cx-globs">' + globs.map(function (g) { return '<span class="cx-glob">' + esc(g) + icon('close') + '</span>'; }).join('') + '<span class="cx-glob cx-glob-add">' + icon('plus') + 'Add pattern</span></div>' +
      '<button class="cx-link cx-small">Reset to defaults</button></div>' +
      '<div class="cx-field-row"><div class="cx-field"><label>Max file size</label><span class="cx-input cx-mono cx-input-w">256<span class="cx-grow"></span><span class="cx-muted">KB</span></span></div>' +
      '<div class="cx-field cx-field-end"><label class="cx-check-label">' + toggle(true) + 'Respect .gitignore</label></div>' +
      '<div class="cx-field cx-field-end"><label class="cx-check-label">' + toggle(true) + 'Skip binary files</label></div>' +
      '<div class="cx-field cx-field-end"><label class="cx-check-label">' + toggle(false) + 'Include hidden files</label></div></div></section>' +
      // output
      '<section class="cx-set-sec"><h2 class="cx-ui-title">Output format</h2><p class="cx-muted">Sent as the system prompt after your enabled rules. The parser reads <code class="cx-pill">### FILE:</code> and <code class="cx-pill">### DELETE:</code> headers to build diffs.</p>' +
      '<div class="cx-prompt">' + '<div class="cx-cb-head"><span class="cx-cb-lang">System prompt · Markdown</span><span class="cx-grow"></span><span class="cx-muted cx-small">168 tokens</span><button class="cx-btn cx-btn-ghost cx-btn-xs">Reset</button></div>' + codeView(S.outputPrompt, { lang: 'markdown', guides: false, currentLine: 9 }) + '</div>' +
      '<div class="cx-field-row"><div class="cx-ok-ink cx-small">' + icon('check') + 'Parser test on last response: 5 files, 0 errors</div></div></section>' +
      '</div></div>';
    return windowFrame(body, { height: 1240, title: 'Settings — Contextual', statusRight: 'Saved' });
  }

  // ---------------------------------------------------------------- light interactivity
  function wire(root) {
    root.addEventListener('click', function (e) {
      var b = e.target.closest('button, .cx-tr.is-dir, .cx-cbx');
      if (!b) return;
      var cb = b.closest('.cx-cb');
      if (b.dataset && b.dataset.cx === 'wrap' && cb) { cb.classList.toggle('is-wrap'); cb.querySelector('.cx-code').classList.toggle('is-wrap'); }
      if (b.dataset && b.dataset.cx === 'copy') { var t = b.lastChild; if (t && t.nodeType === 3) { t.textContent = 'Copied'; setTimeout(function () { t.textContent = 'Copy'; }, 1200); } }
      if (b.dataset && b.dataset.cx === 'collapse') { var df = b.closest('.cx-df'); df.classList.toggle('is-collapsed'); }
      if (b.parentElement && b.parentElement.classList.contains('cx-seg')) { Array.prototype.forEach.call(b.parentElement.children, function (c) { c.classList.toggle('is-active', c === b); }); if (b.parentElement.dataset.onswitch) root.dispatchEvent(new CustomEvent('cx-seg', { detail: b.textContent })); }
      if (b.classList.contains('cx-cbx')) { var on = b.classList.contains('is-on'); b.className = 'cx-cbx is-' + (on ? 'off' : 'on'); b.innerHTML = on ? '' : icon('check'); }
    });
  }

  function mount(el, html) { el.innerHTML = html; wire(el); return el; }

  window.Contextual = {
    version: '1.0.0',
    esc: esc, icon: icon, logo: logo, fileGlyph: fileGlyph, langFromPath: langFromPath, tokenize: tokenize, highlight: hlLine,
    codeView: codeView, codeBlock: codeBlock, inlineCode: inline,
    computeDiff: computeDiff, diffFile: diffFile, statBar: statBar, statusLetter: statusLetter,
    checkbox: checkbox, toggle: toggle, segmented: segmented, button: button,
    fileTree: fileTree, rulesPanel: rulesPanel, contextBar: contextBar, composer: composer,
    chatThread: chatThread, changesPanel: changesPanel, applyDialog: applyDialog, toast: toast,
    windowFrame: windowFrame, screens: { welcome: welcome, workspace: workspace, diff: function (o) { return windowFrame('<div class="cx-ws">' + sidebar({ tree: { diskBanner: true } }) + '<main class="cx-center cx-center-wide">' + diffScreen(o) + '</main></div>', { height: (o && o.height) || 900 }); }, settings: settings },
    samples: S, totals: totals, setScenario: setScenario, mount: mount, wire: wire, dedent: dedent
  };
})();
