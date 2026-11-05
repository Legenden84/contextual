// Line-based, regex-driven syntax highlighter. Tuned for readability in a review tool,
// not for compiler accuracy. Ported from docs/design/contextual/components/bundle.js.

export type Lang =
  | 'csharp'
  | 'typescript'
  | 'python'
  | 'json'
  | 'yaml'
  | 'xml'
  | 'sql'
  | 'markdown'
  | 'shell'
  | 'plain'

export type TokenClass =
  | 'plain'
  | 'pun'
  | 'kw'
  | 'ctl'
  | 'type'
  | 'fn'
  | 'str'
  | 'num'
  | 'com'
  | 'var'
  | 'tag'
  | 'head'

export interface Token {
  t: string
  c: TokenClass
}

const EXT: Record<string, Lang> = {
  cs: 'csharp',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'typescript',
  jsx: 'typescript',
  mjs: 'typescript',
  py: 'python',
  json: 'json',
  yml: 'yaml',
  yaml: 'yaml',
  xml: 'xml',
  csproj: 'xml',
  props: 'xml',
  targets: 'xml',
  sln: 'plain',
  sql: 'sql',
  md: 'markdown',
  mdc: 'markdown',
  sh: 'shell',
  bash: 'shell',
  ps1: 'shell',
}

const FENCE: Record<string, Lang> = {
  cs: 'csharp',
  csharp: 'csharp',
  'c#': 'csharp',
  ts: 'typescript',
  typescript: 'typescript',
  js: 'typescript',
  javascript: 'typescript',
  tsx: 'typescript',
  py: 'python',
  python: 'python',
  json: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  xml: 'xml',
  csproj: 'xml',
  sql: 'sql',
  md: 'markdown',
  markdown: 'markdown',
  sh: 'shell',
  bash: 'shell',
  shell: 'shell',
  powershell: 'shell',
}

export const LANG_LABEL: Record<Lang, string> = {
  csharp: 'C#',
  typescript: 'TypeScript',
  python: 'Python',
  json: 'JSON',
  yaml: 'YAML',
  xml: 'XML',
  sql: 'SQL',
  markdown: 'Markdown',
  shell: 'Shell',
  plain: 'Text',
}

export function langFromPath(path: string | undefined): Lang {
  if (!path) return 'plain'
  if (/\.gitignore$/.test(path)) return 'shell'
  const m = /\.([A-Za-z0-9]+)$/.exec(path)
  return (m && EXT[m[1].toLowerCase()]) || 'plain'
}

export function langFromFence(fence: string): Lang {
  return FENCE[fence.trim().toLowerCase()] ?? 'plain'
}

const kwRe = (words: string, flags = '') => new RegExp('\\b(?:' + words.split(' ').join('|') + ')\\b', 'y' + flags)

const CS_CTL = 'if else return await throw for foreach while do switch case break continue try catch finally yield when goto'
const CS_KW =
  'using namespace public private protected internal static readonly sealed class record struct interface enum void var new async this base null true false is not and or get set init required override virtual abstract const in out ref params partial where typeof nameof default string int bool decimal long double float object char byte short uint ulong dynamic operator implicit explicit event delegate global with'
const TS_CTL = 'if else return await throw for while do switch case break continue try catch finally yield of'
const TS_KW =
  'import export from const let var function class interface type extends implements new async this super null undefined true false typeof instanceof in as public private readonly static enum declare keyof void string number boolean any unknown never'
const PY_CTL = 'if elif else return await raise for while try except finally with yield pass break continue'
const PY_KW = 'def class import from as None True False and or not in is lambda async global nonlocal self'
const SQL_KW =
  'select from where join inner left right outer on and or not null is insert into values update set delete create table alter add drop index primary key foreign references default as order by group having limit offset top distinct case when then else end exists in like between union all with begin commit transaction varchar nvarchar int bigint uniqueidentifier datetimeoffset decimal bit constraint'

const TYPE_AFTER = /^(new|typeof|nameof|is|as|class|record|struct|interface|enum|where)$/

/** Classifies an identifier from its surroundings (call, type position, namespace…). */
function classifyIdent(lang: Lang, src: string, pos: number): Token | null {
  const m = /[A-Za-z_@][A-Za-z0-9_]*/y
  m.lastIndex = pos
  const r = m.exec(src)
  if (!r) return null
  const w = r[0]
  const after = src.slice(pos + w.length)
  const before = src.slice(0, pos).replace(/\s+$/, '')
  const isCall = /^\s*(<[A-Za-z0-9_, <>?]*>)?\s*\(/.test(after)
  const dotted = /\.$/.test(before)
  const prevWord = (before.match(/([A-Za-z_]+)$/) || [])[1] || ''
  const pascal = /^[A-Z]/.test(w)
  let c: TokenClass = 'var'

  if (lang === 'csharp' || lang === 'typescript') {
    if (
      /^\s*(using|namespace)\s+[\w.]*$/.test(before) ||
      prevWord === 'namespace' ||
      (prevWord === 'using' && lang === 'csharp' && !/^\s*(var|\()/.test(after) && w !== 'var')
    )
      c = 'plain'
    else if (isCall) c = 'fn'
    else if (/^I[A-Z]/.test(w) && !dotted) c = 'type'
    else if (pascal && dotted) c = 'var'
    else if (pascal) {
      const typeish =
        /^\s*\./.test(after) ||
        /^\s*</.test(after) ||
        /^\??\s*\]/.test(after) ||
        /^\??\s+(?!is\b|and\b|or\b|not\b|with\b|when\b)[A-Za-z_@]/.test(after) ||
        (/[<:,([]$/.test(before) && /^\??\s*[>,)\]\s]/.test(after)) ||
        TYPE_AFTER.test(prevWord)
      c = typeish ? 'type' : 'var'
    }
  } else if (lang === 'python') {
    if (prevWord === 'def' || isCall) c = 'fn'
    else if (prevWord === 'class' || pascal) c = 'type'
  }
  return { t: w, c }
}

type Rule = [RegExp, TokenClass | TokenClass[]] | 'ident'

const RULES: Record<Lang, Rule[]> = {
  csharp: [
    [/\/\/.*/y, 'com'],
    [/\/\*.*?\*\//y, 'com'],
    [/\$?@?"(?:[^"\\]|\\.)*"/y, 'str'],
    [/'(?:[^'\\]|\\.)'/y, 'str'],
    [/\b\d[\d_]*(?:\.\d+)?[mMfFdDlLuU]?\b/y, 'num'],
    [/#(?:region|endregion|nullable|if|endif|pragma).*/y, 'kw'],
    [kwRe(CS_CTL), 'ctl'],
    [kwRe(CS_KW), 'kw'],
    'ident',
  ],
  typescript: [
    [/\/\/.*/y, 'com'],
    [/\/\*.*?\*\//y, 'com'],
    [/`(?:[^`\\]|\\.)*`/y, 'str'],
    [/"(?:[^"\\]|\\.)*"/y, 'str'],
    [/'(?:[^'\\]|\\.)*'/y, 'str'],
    [/\b\d[\d_]*(?:\.\d+)?\b/y, 'num'],
    [kwRe(TS_CTL), 'ctl'],
    [kwRe(TS_KW), 'kw'],
    'ident',
  ],
  python: [
    [/#.*/y, 'com'],
    [/@[\w.]+/y, 'fn'],
    [/[rbf]?"""[\s\S]*?"""|[rbf]?"(?:[^"\\]|\\.)*"|[rbf]?'(?:[^'\\]|\\.)*'/y, 'str'],
    [/\b\d[\d_]*(?:\.\d+)?\b/y, 'num'],
    [kwRe(PY_CTL), 'ctl'],
    [kwRe(PY_KW), 'kw'],
    'ident',
  ],
  json: [
    [/"(?:[^"\\]|\\.)*"(?=\s*:)/y, 'var'],
    [/"(?:[^"\\]|\\.)*"/y, 'str'],
    [/-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/y, 'num'],
    [/\b(?:true|false|null)\b/y, 'kw'],
    [/\/\/.*/y, 'com'],
  ],
  xml: [
    [/<!--.*?-->/y, 'com'],
    [/<\?[\w-]+/y, 'tag'],
    [/(<\/?)([\w.:-]+)/y, ['pun', 'tag']],
    [/([\w.:-]+)(=)/y, ['var', 'pun']],
    [/"[^"]*"/y, 'str'],
    [/\/?>|\?>/y, 'pun'],
  ],
  yaml: [
    [/#.*/y, 'com'],
    [/^(\s*-?\s*)([\w.\-/]+)(:)/y, ['plain', 'head', 'pun']],
    [/"(?:[^"\\]|\\.)*"|'[^']*'/y, 'str'],
    [/\b(?:true|false|null|yes|no)\b/y, 'kw'],
    [/\b\d+(?:\.\d+)?\b/y, 'num'],
  ],
  sql: [
    [/--.*/y, 'com'],
    [/'(?:[^']|'')*'/y, 'str'],
    [/\b\d+(?:\.\d+)?\b/y, 'num'],
    [kwRe(SQL_KW, 'i'), 'kw'],
    [/@\w+/y, 'var'],
    [/\[[^\]]+\]/y, 'type'],
  ],
  markdown: [
    [/^#{1,6} .*/y, 'head'],
    [/`[^`]+`/y, 'str'],
    [/\*\*[^*]+\*\*/y, 'kw'],
    [/^\s*(?:[-*]|\d+\.) /y, 'ctl'],
    [/\[[^\]]+\]\([^)]+\)/y, 'fn'],
    [/^```.*/y, 'com'],
    [/^> .*/y, 'com'],
  ],
  shell: [
    [/#.*/y, 'com'],
    [/"(?:[^"\\]|\\.)*"|'[^']*'/y, 'str'],
    [/\s--?[\w-]+/y, 'kw'],
    [/\$\w+|\$\{[^}]+\}/y, 'var'],
    [/^(\s*)(dotnet|git|npm|cd|docker|az|curl|echo|export)\b/y, ['plain', 'fn']],
  ],
  plain: [],
}

export function tokenize(line: string, lang: Lang): Token[] {
  const rules = RULES[lang] ?? []
  const out: Token[] = []
  let pos = 0
  let buf = ''
  const flush = () => {
    if (buf) {
      out.push({ t: buf, c: 'plain' })
      buf = ''
    }
  }

  while (pos < line.length) {
    let hit = 0
    for (let i = 0; i < rules.length && !hit; i++) {
      const rule = rules[i]
      if (rule === 'ident') {
        const atWordStart = /[A-Za-z_@]/.test(line[pos]) && (pos === 0 || !/[A-Za-z0-9_]/.test(line[pos - 1]))
        const tok = atWordStart ? classifyIdent(lang, line, pos) : null
        if (tok) {
          flush()
          out.push(tok)
          hit = tok.t.length
        }
        continue
      }
      const [re, cls] = rule
      if (re.source.startsWith('^') && pos !== 0) continue
      re.lastIndex = pos
      const m = re.exec(line)
      if (m && m[0].length) {
        flush()
        if (Array.isArray(cls)) {
          for (let g = 1; g < m.length; g++) if (m[g]) out.push({ t: m[g], c: cls[g - 1] })
        } else out.push({ t: m[0], c: cls })
        hit = m[0].length
      }
    }
    if (!hit) {
      // Swallow a plain word at once so keyword rules don't match mid-word.
      const w = /[A-Za-z0-9_]+/y
      w.lastIndex = pos
      const wm = w.exec(line)
      if (wm && rules.length) {
        buf += wm[0]
        pos += wm[0].length
      } else {
        buf += line[pos]
        pos++
      }
    } else pos += hit
  }
  flush()
  return out
}
