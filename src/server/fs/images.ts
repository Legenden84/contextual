// Image files that can be sent to a vision model as pictures.

/** Formats Azure OpenAI vision accepts. */
export const IMAGE_EXT = /\.(png|jpe?g|gif|webp)$/i

/** Azure OpenAI's per-image upload limit. */
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024

const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp' }

export function imageMime(name: string): string {
  return MIME[name.split('.').pop()!.toLowerCase()] ?? 'application/octet-stream'
}
