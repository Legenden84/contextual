// Records the old server's git/PR results for the parity test (crates/core/tests/git_integration.rs).
//
//   node crates/core/tests/fixtures/git/record.ts --out crates/core/tests/fixtures/git/expected.json
//
// Builds the scripted repo with make-repo.sh, puts the fake `gh` from this folder first on PATH,
// and runs the same sequence of calls as the Rust test. Output paths contain the temp dir, which
// is reported as `tmp` so the test can replace it.

import { execFileSync } from 'node:child_process'
import { mkdtempSync, realpathSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
process.env.GIT_CONFIG_GLOBAL = '/dev/null'
process.env.GIT_CONFIG_NOSYSTEM = '1'
process.env.PATH = `${here}${path.delimiter}${process.env.PATH}`

// git.ts snapshots process.env at import time, so import after setting it.
const git = await import('../../../../../src/server/git.ts')

const tmp = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'ctx-git-')))
execFileSync('sh', [path.join(here, 'make-repo.sh'), tmp])
const root = path.join(tmp, 'work')

const error = async (p: Promise<unknown>) => {
  try {
    await p
    return null
  } catch (err) {
    return (err as Error).message
  }
}

const results: Record<string, unknown> = {}
results.status = await git.gitStatus(root)
results.branches = await git.listBranches(root)
results.compareDefault = await git.compare(root)
results.compareChild = await git.compare(root, undefined, 'feature/child')
results.compareMainChild = await git.compare(root, 'main', 'feature/child')
results.compareMissing = await git.compare(root, 'main', 'nope')
results.diffStaged = await git.fileDiff(root, 'README.md', true)
results.diffUnstaged = await git.fileDiff(root, 'src/app.ts', false)
results.diffDeleted = await git.fileDiff(root, 'crlf.txt', false)
results.diffRenamed = await git.fileDiff(root, 'renamed-again.txt', true)
results.commitDiff = await git.commitDiff(root)
results.prDiff = await git.prDiff(root, 'main')
results.errNotRepo = await error(git.gitStatus(tmp))
results.errEscape = await error(git.fileDiff(root, '../x', false))
results.errEmptyMessage = await error(git.commit(root, '  ', false))
results.errBadBranch = await error(git.createBranch(root, 'bad..name'))

// Mutations, in order.
await git.stage(root, ['src/app.ts', 'crlf.txt', 'notes/todo.md'])
results.afterStage = await git.gitStatus(root)
await git.unstage(root, ['README.md'])
results.afterUnstage = await git.gitStatus(root)
results.commit = await git.commit(root, '  Work in progress  ', false)
results.createBranch = await git.createBranch(root, 'feature/new')
results.push = await git.push(root)
results.commitAll = await git.commit(root, 'Everything else', true)
results.errNothingToCommit = await error(git.commit(root, 'x', false))
results.switchRemote = await git.switchBranch(root, 'origin/feature/login')
results.switchBack = await git.switchBranch(root, 'feature/new')
results.compareNew = await git.compare(root)
results.createPr = await git.createPullRequest(root, { title: ' New thing ', body: '', base: 'feature/login', draft: true })
results.errSameBranch = await error(git.createPullRequest(root, { title: 'x', body: '', base: 'feature/new', draft: false }))

const json = JSON.stringify({ tmp, results }, null, 2) + '\n'
const out = process.argv.indexOf('--out')
if (out > 0) writeFileSync(process.argv[out + 1], json)
else process.stdout.write(json)
