// Phase 0 logic fixtures: runs the real TypeScript in src/web/lib on realistic inputs and writes
// the inputs and outputs as JSON next to this script. crates/core/tests/logic_fixtures.rs
// replays them against the Rust port.
//
//   node crates/core/tests/fixtures/logic/generate.mjs
//
// Needs Node 22.18+ (type stripping on by default). No npm packages are used.

import { writeFileSync } from 'node:fs'
import { register } from 'node:module'

register('./loader.mjs', import.meta.url)

const lib = (name) => import(`../../../../../src/web/lib/${name}.ts`)
const diff = await lib('diff')
const markdown = await lib('markdown')
const changes = await lib('changes')
const edits = await lib('edits')
const prompt = await lib('prompt')
const tree = await lib('tree')
const images = await lib('images')
const hash = await lib('hash')
const format = await lib('format')
const gitTabs = await lib('git-tabs')
const foundry = await lib('foundry-status')
const defaults = await import('../../../../../src/web/data/defaults.ts')

const out = (name, cases) => {
  // JSON.stringify drops `undefined` fields, exactly like the Rust side's skip_serializing_if.
  writeFileSync(new URL(`./${name}.json`, import.meta.url), JSON.stringify(cases, null, 1) + '\n')
  console.log(`${name}.json: ${cases.length} cases`)
}
const clone = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)))

// ------------------------------------------------------------------ sample sources

const CS = `using System;
using System.Threading.Tasks;

namespace Shop.Orders;

public class OrderService
{
    private readonly IOrderRepository _repo;

    public OrderService(IOrderRepository repo)
    {
        _repo = repo;
    }

    public async Task<Order> GetAsync(int id)
    {
        var order = await _repo.FindAsync(id);
        if (order is null) throw new NotFoundException(id);
        return order;
    }

    public void Cancel(Order order)
    {
        order.Status = OrderStatus.Cancelled;
        _repo.Save(order);
    }

    internal static decimal Total(Order order) => order.Lines.Sum(l => l.Price * l.Quantity);
}
`

const CS2 = CS.replace('throw new NotFoundException(id);', 'throw new OrderNotFoundException(id, "missing");')
  .replace('        _repo.Save(order);\n', '        order.CancelledAt = DateTime.UtcNow;\n        _repo.Save(order);\n')
  .replace('using System;\n', 'using System;\nusing System.Linq;\n')
  .replace('internal static decimal Total', 'public static decimal Total')

const TS = `import { useState } from 'react'

export function Counter({ start = 0 }: { start?: number }) {
  const [n, setN] = useState(start)
  return <button onClick={() => setN(n + 1)}>Clicked {n} times</button>
}
`

const PY = `def greet(name):
    print("Hello, " + name)


def main():
    greet("world")
    greet("émilie 👋")
`

const LONG = Array.from({ length: 40 }, (_, i) => `line ${i + 1}`).join('\n') + '\n'
const LONG2 = LONG.replace('line 5\n', 'line five\n').replace('line 30\n', 'line 30 changed\nextra line\n').replace('line 38\n', '')

// ------------------------------------------------------------------ diff

const diffPairs = [
  ['identical', CS, CS],
  ['csharp edits', CS, CS2],
  ['add file', '', TS],
  ['delete file', PY, ''],
  ['both empty', '', ''],
  ['no final newline', 'a\nb\nc', 'a\nB\nc'],
  ['final newline added', 'a\nb', 'a\nb\n'],
  ['only newline', '\n', '\n\n'],
  ['crlf lines', 'one\r\ntwo\r\nthree\r\n', 'one\r\ntwo 2\r\nthree\r\n'],
  ['unicode word change', 'const greeting = "héllo wörld"\n', 'const greeting = "héllo wörld 👋"\n'],
  ['emoji inside', 'x = "😀 smile"\n', 'x = "😀 grin"\n'],
  ['emoji swap shares surrogate', 'face 😀 end\n', 'face 😁 end\n'],
  ['indent change', '    return order;\n', '        return order;\n'],
  ['dissimilar lines', 'alpha beta\ngamma\n', 'completely different\nzzz\n'],
  ['long file two regions', LONG, LONG2],
  ['python unicode', PY, PY.replace('world', 'there').replace('👋', '🌍')],
  ['whitespace only line', '  \n\tfoo\n', '   \n\tfoo bar\n'],
  ['reorder', 'a\nb\nc\nd\n', 'd\nc\nb\na\n'],
]

const diffCases = diffPairs.map(([name, oldText, newText]) => {
  const d = diff.computeDiff(oldText, newText)
  const hunks = diff.makeHunks(d.ops)
  return {
    name,
    input: { old: oldText, new: newText },
    output: {
      diff: clone(d),
      hunks: clone(hunks),
      hunks1: clone(diff.makeHunks(d.ops, 1)),
      hunks0: clone(diff.makeHunks(d.ops, 0)),
      headers: hunks.filter((h) => h.kind === 'hunk').map((h) => diff.hunkHeader(h.lines, d.newLines)),
      patchM: diff.unifiedPatch('src/File.cs', d, 'M'),
      patchA: diff.unifiedPatch('src/File.cs', d, 'A'),
      patchD: diff.unifiedPatch('src/File.cs', d, 'D'),
    },
  }
})
out('diff', diffCases)

// ------------------------------------------------------------------ markdown

const MD = {
  'headings and paragraphs': `# Title

Some **bold** and *italic* and \`code\` text
that continues on the next line.

## Second ##
###### Six
####### seven is a paragraph`,
  lists: `Changes:

- First item with [a link](https://example.com "Example")
- Second item
  continues here
  - nested bullet
    - deeper
1. one
2) two
10. ten

- [A] src/New.cs
- [M] src/Old.cs
- [D] src/Gone.cs
- [ ] todo
- [x] done
- [X] Done too
* star item
+ plus item`,
  'list separated by blank line': `- a

- b

not a list`,
  tables: `| Name | Type | Default |
| :--- | :---: | ---: |
| \`id\` | int | – |
| path \\| name | string | "" |
| **bold** | _it_ |

after table

a | b
--|--
1 | 2
paragraph | then
|---|---|`,
  'code fences': `Here is the fix:

\`\`\`csharp
public class A
{
    // \`\`\` inside is fine when indented? no, this closes only at line start
}
\`\`\`

~~~
tilde fence
~~~

\`\`\`\`md
\`\`\`
nested fence
\`\`\`
\`\`\`\`

\`\`\`unknownlang extra info
x
\`\`\`

\`\`\`
unterminated`,
  'file delete edit blocks': `I updated the service and removed the old helper.

### FILE: src/New.cs
\`\`\`csharp
public class New {}
\`\`\`

### FILE: \`src/Quoted.ts\`
\`\`\`
export const x = 1
\`\`\`

### FILE: docs/Ref.html (condensed reference, read-only: do not return this file)
\`\`\`html
<p>ref</p>
\`\`\`

## DELETE: src/Old.cs
#### DELETE: \`src/Older.cs\` (no longer used)

### EDIT: src/Service.cs
<<<<<<< SEARCH
    public void Run()
    {
=======
    public void Run(int times)
    {
>>>>>>> REPLACE

<<<<<<< SEARCH
        Console.WriteLine("hi");
=======
        Console.WriteLine("hello");
>>>>>>> REPLACE
That's all.`,
  'edit as whole file': `### EDIT: src/Small.py
\`\`\`python
print("whole file")
\`\`\`
`,
  'edit fenced blocks': `### EDIT: src/A.ts
\`\`\`
<<<<<<< SEARCH
a
=======
b
>>>>>>> REPLACE
\`\`\`
`,
  'edit partial': `### EDIT: src/Streaming.cs
<<<<<<< SEARCH
old line
=======
new li`,
  'edit loose markers': `### EDIT: src/Loose.cs
  <<<<<<<   SEARCH
x
  =========
y
>>>>>>>>> REPLACE
<<<<<<<<<< SEARCH
not a marker (10)`,
  'edit empty': `### EDIT: src/Empty.cs

Some text after.`,
  quotes: `> Quoted **text**
> - list in quote
>
> > nested

after`,
  'hr variants': `---
* * *
___
- - -
-- not hr
text
***`,
  inline: `Mix: \`code with **stars**\`, **bold _and italic_**, __under bold__, ~~strike~~, snake_case_name, 2*3*4,
*emph*, _emph_, ** not bold**, visit https://example.com/path?q=1. Or (http://x.io/a_b), [link **b**](./rel/path.md).
Unclosed \`tick and *star and [bracket](no close`,
  'crlf and unicode': '# Überschrift 🚀\r\n\r\nZeile mit **fett** und émoji 😀\r\n- punkt\r\n',
  'lone cr': 'para\rwith cr\n```ts\r\ncode\n```',
  'empty': '',
  'only blank': '\n\n   \n',
  'tabs indent list': '- top\n\t- tab nested\n\t\t- deeper',
  'bom and nbsp': '\ufeff# Title\n\u00a0\u00a0- item\n\u00a0',
  'heading closers': '# Title #\n## Title ## \n### #\n#\n#notheading',
  'file then plain fence': '## FILE: a/b.json\n\n```\n{"a": 1}\n```\n\n```\nno path\n```',
  'file header empty after clean': '### FILE: `\n```\nx\n```',
}

const INLINE = [
  'plain text',
  '**bold** then *it*',
  '***both***',
  '`a` `b`',
  'a_b_c and _x_',
  '*not closed',
  '* list-like *',
  'x*y*z and *a*b',
  '[text](url) and [bad] (url)',
  '[t](u "title") [t2](u2 \'no\')',
  'https://a.b/c, https://d.e/(f)',
  'see http://example.com/x_y_z.',
  '~~gone~~ ~~ not~~',
  '__a__b __c__',
  'émoji **😀** _ü_',
  '_a*b_',
  '**a**b**c**',
  'nbsp\u00a0*x*\u00a0',
  'HTTPS://upper.case',
  'xhttps://no.boundary 9https://digit',
  'foo_bar_ baz and x*y* z',
  '**_x_ and _y_ __z__',
  '[a](b c) [`x`](y)',
]

out('markdown', [
  ...Object.entries(MD).map(([name, src]) => ({ name, fn: 'parseMarkdown', input: src, output: clone(markdown.parseMarkdown(src)) })),
  ...INLINE.map((src) => {
    const nodes = markdown.parseInline(src)
    return { name: src, fn: 'parseInline', input: src, output: { nodes: clone(nodes), text: markdown.inlineText(nodes) } }
  }),
])

// ------------------------------------------------------------------ edits

const EDIT_CASES = [
  ['exact', CS, [{ search: '        _repo = repo;\n', replace: '        _repo = repo ?? throw new ArgumentNullException(nameof(repo));\n' }]],
  ['trailing whitespace', 'a  \nb\t\nc\n', [{ search: 'a\nb', replace: 'A\nB' }]],
  [
    'indent differs (deeper in file)',
    CS,
    [{ search: 'public void Cancel(Order order)\n{\n    order.Status = OrderStatus.Cancelled;', replace: 'public void Cancel(Order order, string reason)\n{\n    order.Status = OrderStatus.Cancelled;\n    order.Reason = reason;' }],
  ],
  ['indent differs (shallower in file)', 'if x:\n  y = 1\n  z = 2\n', [{ search: '        y = 1\n        z = 2', replace: '        y = 10\n        z = 20\n            w = 3' }]],
  ['blank edges', 'a\nb\nc\n', [{ search: '\n\nb\n\n', replace: '\nB\n\n' }]],
  ['not found', CS, [{ search: 'does not exist', replace: 'x' }]],
  ['multiple in order', 'x\ny\nx\ny\n', [{ search: 'x', replace: 'X1' }, { search: 'x', replace: 'X2' }]],
  ['cursor wraps to first', 'x\ny\nx\n', [{ search: 'x\ny\nx', replace: 'z' }, { search: 'y', replace: 'Y' }]],
  ['empty search on empty file', '', [{ search: '', replace: 'new content\nline 2\n' }]],
  ['empty search on blank file', '  \n\n', [{ search: '\n', replace: 'filled' }]],
  ['empty search on nonempty', 'abc\n', [{ search: '  ', replace: 'x' }]],
  ['delete lines', 'a\nb\nc\n', [{ search: 'b\n', replace: '' }]],
  ['crlf original', 'one\r\ntwo\r\nthree\r\n', [{ search: 'two', replace: 'TWO' }]],
  ['crlf search', 'one\ntwo\nthree', [{ search: 'one\r\ntwo\r\n', replace: '1\r\n2\r\n' }]],
  ['no final newline kept', 'a\nb', [{ search: 'b', replace: 'B' }]],
  ['all lines removed', 'only\n', [{ search: 'only', replace: '' }]],
  ['unicode', 'grüße 👋\nwelt\n', [{ search: 'grüße 👋', replace: 'hallo 🌍' }]],
  ['mixed success', PY, [{ search: 'greet("world")', replace: 'greet("you")' }, { search: 'missing()', replace: 'x' }, { search: '    print("Hello, " + name)', replace: '    print(f"Hello, {name}")' }]],
  ['nbsp indent', '\u00a0\u00a0x = 1\n', [{ search: 'x = 1', replace: 'x = 2\ny = 3' }]],
  ['tab vs spaces', '\tfoo()\n\tbar()\n', [{ search: '    foo()\n    bar()', replace: '    foo(1)\n      bar(2)' }]],
  ['replace with blank lines', 'a\nb\n', [{ search: 'a', replace: '\n' }]],
]

out('edits', [
  ...EDIT_CASES.map(([name, original, eds]) => ({ name, fn: 'applyEdits', input: { original, edits: eds }, output: clone(edits.applyEdits(original, eds)) })),
  { name: 'format', fn: 'formatEdits', input: { edits: EDIT_CASES[17][2] }, output: edits.formatEdits(EDIT_CASES[17][2]) },
  { name: 'format empty', fn: 'formatEdits', input: { edits: [] }, output: edits.formatEdits([]) },
])

// ------------------------------------------------------------------ changes

const CHANGE_CASES = [
  {
    name: 'file delete edit mix',
    response: MD['file delete edit blocks'],
    exists: ['src/Service.cs', 'src/Old.cs', 'src/Older.cs', 'docs/Ref.html'],
    originals: {
      'src/Service.cs': 'class S\n{\n    public void Run()\n    {\n        Console.WriteLine("hi");\n    }\n}\n',
      'src/Old.cs': 'old\n',
    },
    changedOnDisk: ['src/Service.cs'],
    readOnly: ['docs/Ref.html'],
  },
  {
    name: 'two edits same file stack',
    response: `### EDIT: a.txt
<<<<<<< SEARCH
one
=======
ONE
>>>>>>> REPLACE

### EDIT: a.txt
<<<<<<< SEARCH
ONE
two
=======
ONE
TWO
>>>>>>> REPLACE
<<<<<<< SEARCH
missing
=======
x
>>>>>>> REPLACE`,
    exists: ['a.txt'],
    originals: { 'a.txt': 'one\ntwo\n' },
    changedOnDisk: [],
    readOnly: [],
  },
  {
    name: 'edit unknown original',
    response: '### EDIT: new.txt\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\n',
    exists: [],
    originals: {},
    changedOnDisk: [],
    readOnly: [],
  },
  {
    name: 'unchanged file dropped',
    response: '### FILE: same.txt\n```\nsame\n```\n### FILE: added.txt\n```\nnew\n```\n',
    exists: ['same.txt'],
    originals: { 'same.txt': 'same' },
    changedOnDisk: [],
    readOnly: [],
  },
  {
    name: 'delete then file keeps order',
    response: '### FILE: b.txt\n```\nb\n```\n### DELETE: a.txt\n### FILE: a.txt\n```\nagain\n```\n',
    exists: ['a.txt'],
    originals: { 'a.txt': 'orig\n' },
    changedOnDisk: ['a.txt'],
    readOnly: [],
  },
  {
    name: 'edit after delete uses original',
    response: '### DELETE: a.txt\n### EDIT: a.txt\n<<<<<<< SEARCH\norig\n=======\nedited\n>>>>>>> REPLACE\n',
    exists: ['a.txt'],
    originals: { 'a.txt': 'orig\n' },
    changedOnDisk: [],
    readOnly: [],
  },
  {
    name: 'partial and read-only edits skipped',
    response: '### EDIT: ro.txt\n<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE\n### EDIT: p.txt\n<<<<<<< SEARCH\na\n',
    exists: ['ro.txt', 'p.txt'],
    originals: { 'ro.txt': 'a\n', 'p.txt': 'a\n' },
    changedOnDisk: [],
    readOnly: ['ro.txt'],
  },
  {
    name: 'edit no-op is dropped, failed edit kept',
    response: '### EDIT: x.txt\n<<<<<<< SEARCH\na\n=======\na\n>>>>>>> REPLACE\n### EDIT: y.txt\n<<<<<<< SEARCH\nzzz\n=======\na\n>>>>>>> REPLACE\n',
    exists: ['x.txt', 'y.txt'],
    originals: { 'x.txt': 'a\n', 'y.txt': 'a\n' },
    changedOnDisk: [],
    readOnly: [],
  },
  {
    name: 'edit as whole file',
    response: MD['edit as whole file'],
    exists: ['src/Small.py'],
    originals: { 'src/Small.py': 'print("old")\n' },
    changedOnDisk: [],
    readOnly: [],
  },
  { name: 'empty response', response: '', exists: [], originals: {}, changedOnDisk: [], readOnly: [] },
  {
    name: 'crlf response',
    response: '### FILE: w.txt\r\n```\r\nwin\r\nlines\r\n```\r\n',
    exists: [],
    originals: {},
    changedOnDisk: [],
    readOnly: [],
  },
]

out(
  'changes',
  CHANGE_CASES.map((c) => ({
    name: c.name,
    input: c,
    output: clone(
      changes.parseChanges(c.response, {
        exists: (p) => c.exists.includes(p),
        original: (p) => c.originals[p],
        changedOnDisk: (p) => c.changedOnDisk.includes(p),
        readOnly: (p) => c.readOnly.includes(p),
      }),
    ),
  })),
)

// ------------------------------------------------------------------ prompt

const RULES = [
  { path: '.cursor/rules/style.mdc', type: 'Rule', tokens: 40, content: '\n  Use 4 spaces.\n' },
  { path: '.claude/skills/db/SKILL.md', type: 'Skill', tokens: 90, content: 'Use EF Core.' },
]
const FILES = [
  { path: 'src/Order.cs', content: CS },
  { path: 'src/app.tsx', content: TS },
  { path: 'web/index.HTML', content: '<p>x</p>', condensed: true },
  { path: 'icons/logo.svg', content: '<svg/>' },
  { path: 'scripts/run.sh', content: 'echo hi' },
  { path: 'README', content: '' },
  { path: 'shots/a.png', image: 'data:image/png;base64,AAAA' },
  { path: 'bin/blob.dat' },
]
const IMGS = [{ name: 'screenshot-101112.png', url: 'data:image/png;base64,BBBB' }]

const turnInputs = [
  { name: 'text only', text: 'Hi', files: [], unchanged: [], removed: [], attachments: [] },
  { name: 'everything', text: 'Fix the bug', files: FILES, unchanged: ['src/Keep.cs'], removed: ['src/Gone.cs', 'x.ts'], attachments: IMGS },
  { name: 'unchanged only', text: 'Again', files: [], unchanged: ['a', 'b'], removed: [], attachments: [] },
  { name: 'attachments only', text: 'Look', files: [], unchanged: [], removed: [], attachments: IMGS },
]

out('prompt', [
  { name: 'rules', fn: 'systemPrompt', input: { rules: RULES, outputFormat: '  ' + defaults.outputPrompt + '\n\n' }, output: prompt.systemPrompt(RULES, '  ' + defaults.outputPrompt + '\n\n') },
  { name: 'no rules, empty format', fn: 'systemPrompt', input: { rules: [], outputFormat: '  \n' }, output: prompt.systemPrompt([], '  \n') },
  { name: 'all kinds', fn: 'contextBlock', input: { files: FILES }, output: prompt.contextBlock(FILES) },
  { name: 'empty', fn: 'contextBlock', input: { files: [] }, output: prompt.contextBlock([]) },
  { name: 'none', fn: 'withImages', input: { text: 'x', images: [] }, output: prompt.withImages('x', []) },
  { name: 'two', fn: 'withImages', input: { text: 'x', images: [...IMGS, { name: 'b.jpg', url: 'data:image/jpeg;base64,CC' }] }, output: prompt.withImages('x', [...IMGS, { name: 'b.jpg', url: 'data:image/jpeg;base64,CC' }]) },
  ...turnInputs.map((t) => {
    const content = prompt.buildTurn(t)
    return {
      name: t.name,
      fn: 'buildTurn',
      input: t,
      output: { content, text: prompt.contentText(content), images: prompt.contentImages(content) },
    }
  }),
  { name: 'REFERENCE_NOTE', fn: 'const', input: null, output: prompt.REFERENCE_NOTE },
])

// ------------------------------------------------------------------ tree

const TREE = [
  {
    name: 'src',
    path: 'src',
    kind: 'dir',
    mtimeMs: 1712345678901.25,
    children: [
      { name: 'App.tsx', path: 'src/App.tsx', kind: 'file', tokens: 1200 },
      {
        name: 'lib',
        path: 'src/lib',
        kind: 'dir',
        children: [
          { name: 'diff.ts', path: 'src/lib/diff.ts', kind: 'file', tokens: 900, changedOnDisk: true },
          { name: 'Big.json', path: 'src/lib/Big.json', kind: 'file', tokens: 90000, excluded: 'too large (90k tokens)' },
          { name: 'logo.png', path: 'src/lib/logo.png', kind: 'file', tokens: 765, image: true },
        ],
      },
      { name: 'page.html', path: 'src/page.html', kind: 'file', tokens: 3000, condensedFrom: 41000 },
      { name: 'node_modules', path: 'src/node_modules', kind: 'dir', excluded: 'ignored: node_modules', children: [{ name: 'x.js', path: 'src/node_modules/x.js', kind: 'file', tokens: 5 }] },
      { name: 'empty', path: 'src/empty', kind: 'dir', children: [] },
    ],
  },
  { name: 'package-lock.json', path: 'package-lock.json', kind: 'file', tokens: 50000, excluded: 'lock file' },
  { name: 'README.md', path: 'README.md', kind: 'file' },
  { name: 'docs', path: 'docs', kind: 'dir', children: [{ name: 'Guide.MD', path: 'docs/Guide.MD', kind: 'file', tokens: 400 }, { name: 'blank-excl.txt', path: 'docs/blank-excl.txt', kind: 'file', tokens: 7, excluded: '' }] },
  { name: 'lib', path: 'lib', kind: 'dir', children: [{ name: 'util.ts', path: 'lib/util.ts', kind: 'file', tokens: 2000 }] },
]

const allNodes = []
const walk = (ns) => ns.forEach((n) => (allNodes.push(n), n.children && walk(n.children)))
walk(TREE)

const SELECTIONS = [
  [],
  ['src/App.tsx'],
  ['src/App.tsx', 'src/lib/diff.ts', 'src/lib/logo.png', 'src/page.html'],
  ['src/lib/Big.json'],
  ['src/App.tsx', 'src/lib/diff.ts', 'src/lib/logo.png', 'src/page.html', 'src/lib/Big.json', 'README.md', 'docs/Guide.MD', 'lib/util.ts', 'docs/blank-excl.txt'],
  ['package-lock.json', 'src/node_modules/x.js', 'lib/util.ts'],
  ['lib/util.ts', 'src/lib/diff.ts'],
]

out('tree', [
  { name: 'pickable', fn: 'pickableFiles', input: { nodes: TREE }, output: [...tree.pickableFiles(TREE)].sort() },
  { name: 'selectable', fn: 'selectableFiles', input: { nodes: TREE }, output: tree.selectableFiles(TREE) },
  { name: 'sort', fn: 'sortNodes', input: { nodes: TREE }, output: tree.sortNodes(TREE).map((n) => n.path) },
  { name: 'sort src', fn: 'sortNodes', input: { nodes: TREE[0].children }, output: tree.sortNodes(TREE[0].children).map((n) => n.path) },
  ...['', 'src/lib/diff.ts', 'src', 'src/lib', 'lib/util.ts', 'missing', 'src/node_modules/x.js', 'docs/Guide.MD'].map((path) => ({
    name: `find ${path}`,
    fn: 'findNode',
    input: { nodes: TREE, path },
    output: tree.findNode(TREE, path)?.path ?? null,
  })),
  ...['', '  ', 'LIB', 'diff', 'md', 'nothing', '.json', ' App '].map((q) => ({
    name: `filter ${q}`,
    fn: 'filterTree',
    input: { nodes: TREE, query: q },
    output: tree.filterTree(TREE, q),
  })),
  ...SELECTIONS.map((sel, i) => {
    const s = new Set(sel)
    return {
      name: `selection ${i}`,
      fn: 'selection',
      input: { nodes: TREE, selected: sel },
      output: {
        stats: Object.fromEntries(allNodes.map((n) => [n.path, tree.nodeStats(n, s)])),
        check: Object.fromEntries(allNodes.map((n) => [n.path, tree.checkState(n, s)])),
        overridable: allNodes.filter(tree.isOverridable).map((n) => n.path),
        biggest: tree.biggestFolder(TREE, s),
      },
    }
  }),
])

// ------------------------------------------------------------------ images

const SIZES = [
  [1, 1], [512, 512], [513, 512], [768, 768], [1024, 768], [1920, 1080], [2048, 2048], [4096, 2048],
  [3000, 4000], [2560, 1440], [100, 5000], [5000, 100], [1366, 768], [801, 333], [2049, 1], [3001, 1999],
]
out(
  'images',
  SIZES.map(([w, h]) => {
    // toAttachment's sizing (lib/images.ts) — it needs a canvas, so the two lines are mirrored here.
    const scale = Math.min(1, 2048 / Math.max(w, h))
    const width = Math.round(w * scale)
    const height = Math.round(h * scale)
    return { name: `${w}x${h}`, input: { width: w, height: h }, output: { tokens: images.visionTokens(w, h), scaled: [width, height], scaledTokens: images.visionTokens(width, height) } }
  }),
)

// ------------------------------------------------------------------ small modules

const HASH_TEXTS = ['', 'a', 'hello world', CS, CS2, 'émilie 👋', '\r\n', '😀'.repeat(50), 'x'.repeat(100000), 'unavailable:ENOENT']
const FMT_K = [0, 1, 999, 1000, 1049, 1050, 1149, 1150, 1250, 1950, 1999, 9949, 9950, 9999, 10000, 12500, 13500, 128000, 999499, 999500, 1234567, 200000000]
const FMT_N = [0, 7, 999, 1000, 18400, 123456, 1234567, 1000000000]
const DEDENT = ['\n  a\n    b\n  c\n', '  x', '\n\n  a\n\n  b  \n\t\n', '', '\n   \n', 'no indent\n  indented', '\n    deep\n  \u00a0nbsp\n']
const PATHS = ['src/web/lib/diff.ts', 'file.txt', '/abs', 'dir/', '', 'ü/ö/ä.md']
const TOKENS_TEXT = ['', 'a', 'ab', 'abc', 'abcd', 'abcdef', 'abcdefghij', '😀', '😀😀😀', 'x'.repeat(4002), CS]

out('format', [
  ...HASH_TEXTS.map((t) => ({ name: 'hash', fn: 'hashText', input: { text: t, seed: 0 }, output: hash.hashText(t) })),
  ...[1, 42, 0x7fffffff, -1].map((seed) => ({ name: `hash seed ${seed}`, fn: 'hashText', input: { text: 'seeded', seed }, output: hash.hashText('seeded', seed) })),
  ...FMT_K.map((n) => ({ name: `fmtK ${n}`, fn: 'fmtK', input: n, output: format.fmtK(n) })),
  ...FMT_N.map((n) => ({ name: `fmtN ${n}`, fn: 'fmtN', input: n, output: format.fmtN(n) })),
  ...DEDENT.map((s) => ({ name: 'dedent', fn: 'dedent', input: s, output: format.dedent(s) })),
  ...PATHS.map((p) => ({ name: `split ${p}`, fn: 'splitPath', input: p, output: format.splitPath(p) })),
  ...TOKENS_TEXT.map((t) => ({ name: 'tokens', fn: 'estimateTokens', input: t, output: format.estimateTokens(t) })),
  ...[['src/a.ts', true], ['src/a.ts', false], ['dir with space/ü.md', true]].map(([p, s]) => ({ name: 'gitTabId', fn: 'gitTabId', input: { path: p, staged: s }, output: gitTabs.gitTabId(p, s) })),
  ...['git:staged:src/a.ts', 'git:changes:a:b.ts', 'git:staged:', 'git:other:x', 'preview:src/a.ts', 'git:changes:multi\nline'].map((t) => ({
    name: 'parseGitTab',
    fn: 'parseGitTab',
    input: t,
    output: gitTabs.parseGitTab(t),
  })),
  ...[
    { state: 'checking' },
    { state: 'ready', health: { configured: true, endpoint: 'https://x.openai.azure.com', apiVersion: 'v1', deployment: 'gpt-5', modelName: 'gpt-5', maxCompletionTokens: 1000 } },
    { state: 'unconfigured', missing: ['AZURE_OPENAI_ENDPOINT', 'AZURE_OPENAI_API_KEY'], invalid: ['AZURE_OPENAI_DEPLOYMENT'] },
    { state: 'offline', error: 'fetch failed' },
  ].map((s) => ({ name: `foundry ${s.state}`, fn: 'describeFoundry', input: s, output: foundry.describeFoundry(s) })),
])

out('defaults', [
  { name: 'outputPrompt', input: null, output: defaults.outputPrompt },
  { name: 'LEGACY_OUTPUT_PROMPTS', input: null, output: defaults.LEGACY_OUTPUT_PROMPTS },
  { name: 'defaultExclusions', input: null, output: defaults.defaultExclusions },
  { name: 'PLACEHOLDER_DEPLOYMENT', input: null, output: defaults.PLACEHOLDER_DEPLOYMENT },
  { name: 'USER_MESSAGE_TOKENS', input: null, output: defaults.USER_MESSAGE_TOKENS },
  { name: 'EST_OUTPUT_TOKENS', input: null, output: defaults.EST_OUTPUT_TOKENS },
  { name: 'systemPrompt default', input: null, output: prompt.systemPrompt([], defaults.outputPrompt) },
])

// ------------------------------------------------------------------ random differential cases
// Deterministic pseudo-random inputs built from the tokens the parsers care about, to catch
// differences the hand-written cases miss.

let seed = 0x2545f491
const rand = (n) => {
  seed ^= seed << 13
  seed ^= seed >>> 17
  seed ^= seed << 5
  return (seed >>> 0) % n
}
const pick = (xs) => xs[rand(xs.length)]
const MD_TOKENS = [
  '*', '_', '`', '**', '__', '~~', '[', ']', '](', '(', ')', ' "t"', 'http://a.b/c_d', 'https://x.y/z.', '|', '\\|', ':--', '---',
  '#', '## ', '- ', '* ', '1. ', '2) ', '> ', '[x] ', '[A] ', '\n', '\n', '\n', '  ', '\t', ' ', ' ', 'x', 'word', 'é', '😀', ' ',
  '```', '```ts', '~~~', '<<<<<<< SEARCH', '=======', '>>>>>>> REPLACE', '### FILE: a.ts', '### EDIT: a.ts', '### DELETE: b (old)', '\r\n', '\r',
  '\na | b\n', '|--|:-:|\n', '| x | y |\n', '\n### EDIT: c.cs\n<<<<<<< SEARCH\n', '\n- [ ] ', '\n    - ',
]
out(
  'markdown_random',
  Array.from({ length: 400 }, (_, i) => {
    const src = Array.from({ length: 4 + rand(40) }, () => pick(MD_TOKENS)).join('')
    return { name: `random ${i}`, fn: 'parseMarkdown', input: src, output: clone(markdown.parseMarkdown(src)) }
  }),
)

const LINE_TOKENS = ['a', 'b', 'c', '  a', 'a ', '\ta', '', ' ', 'x = 1', 'x = 2', 'é😀', '😁', '{', '}', 'public class A', 'void F()', '\r']
const randLines = (n) => Array.from({ length: n }, () => pick(LINE_TOKENS) + (rand(4) ? '' : pick(LINE_TOKENS)))
const randText = (n) => randLines(n).join('\n') + (rand(2) ? '\n' : '')
out(
  'diff_random',
  Array.from({ length: 150 }, (_, i) => {
    const oldText = randText(rand(14))
    const lines = oldText.split('\n')
    for (let k = rand(5); k > 0; k--) lines.splice(rand(lines.length + 1), rand(2), ...randLines(rand(3)))
    const newText = lines.join('\n')
    const d = diff.computeDiff(oldText, newText)
    return {
      name: `random ${i}`,
      input: { old: oldText, new: newText },
      output: { diff: clone(d), hunks: clone(diff.makeHunks(d.ops, 2)), patch: diff.unifiedPatch('f', d, 'M') },
    }
  }),
)
out(
  'edits_random',
  Array.from({ length: 200 }, (_, i) => {
    const original = randText(rand(12))
    const lines = original.split('\n')
    const eds = Array.from({ length: 1 + rand(3) }, () => {
      const at = rand(lines.length)
      let search = lines.slice(at, at + 1 + rand(3)).join('\n')
      if (rand(3) === 0) search = search.replace(/^\s+/gm, rand(2) ? '' : '    ')
      if (rand(5) === 0) search = '\n' + search + '\n\n'
      if (rand(6) === 0) search = pick(LINE_TOKENS)
      return { search, replace: rand(5) ? randLines(rand(3)).join('\n') : '' }
    })
    return { name: `random ${i}`, fn: 'applyEdits', input: { original, edits: eds }, output: clone(edits.applyEdits(original, eds)) }
  }),
)
