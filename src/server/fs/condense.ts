// Condenses large HTML / SVG files for the model: embedded data the model can't use
// (base64 images and fonts, minified scripts and styles, long SVG paths, JSON blobs) is
// replaced by short placeholders. Structure, text, classes and readable code stay.
// The file on disk is never changed; condensed copies are sent as read-only references.

/** Files at least this large are condensed (smaller ones are cheap enough as they are). */
export const CONDENSE_MIN_BYTES = 32 * 1024

export const isCondensable = (name: string) => /\.(html?|svg)$/i.test(name)

const kb = (n: number) => `${Math.max(1, Math.round(n / 1024))} KB`

/** Minified code: few line breaks relative to its length. */
function looksMinified(code: string): boolean {
  const lines = code.split('\n').filter((l) => l.trim()).length || 1
  return code.length > 2000 && code.length / lines > 250
}

export function condense(text: string): string {
  let out = text

  // data: URIs (images, fonts) in attributes and CSS url(...).
  out = out.replace(/data:([\w/+.-]+)?(;[\w=-]+)*;base64,[A-Za-z0-9+/=\s]{200,}/g, (m, mime: string | undefined) => {
    return `data:${mime ?? 'application/octet-stream'};base64,[${kb(m.length)} omitted]`
  })

  // <script> blocks: minified code and large JSON payloads.
  out = out.replace(/(<script\b[^>]*>)([\s\S]*?)(<\/script>)/gi, (m, open: string, body: string, close: string) => {
    const json = /type=["']?application\/(ld\+)?json/i.test(open)
    if ((json && body.length > 5000) || looksMinified(body))
      return `${open}/* ${kb(body.length)} of ${json ? 'JSON data' : 'minified script'} omitted */${close}`
    return m
  })

  // <style> blocks with minified CSS.
  out = out.replace(/(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi, (m, open: string, body: string, close: string) =>
    looksMinified(body) ? `${open}/* ${kb(body.length)} of minified CSS omitted */${close}` : m,
  )

  // Long SVG geometry.
  out = out.replace(/\b(d|points)=(["'])([^"']{300,})\2/g, (_m, attr: string, q: string, v: string) => `${attr}=${q}[${v.length} chars of path data omitted]${q}`)

  return out
}
