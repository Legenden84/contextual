// Resolve hook for running src/web/lib/*.ts under Node's type stripping: maps `@/` to src/web/
// and adds the `.ts` extension that the Vite sources leave out.
import { existsSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

const WEB = new URL('../../../../../src/web/', import.meta.url)

export async function resolve(specifier, context, next) {
  let spec = specifier
  if (spec.startsWith('@/')) spec = new URL(spec.slice(2), WEB).href
  if (spec.startsWith('.') || spec.startsWith('file:')) {
    const url = new URL(spec, context.parentURL)
    const path = fileURLToPath(url)
    if (!existsSync(path) && existsSync(path + '.ts')) return next(pathToFileURL(path + '.ts').href, context)
    return next(url.href, context)
  }
  return next(spec, context)
}
