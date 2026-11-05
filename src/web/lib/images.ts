// Pictures attached to a message: pasted, dropped or picked screenshots.

export interface Attachment {
  id: string
  name: string
  /** data: URL sent to the model. */
  url: string
  width: number
  height: number
  /** Estimated input tokens at high detail. */
  tokens: number
}

export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

/** Longest side sent; larger pictures are scaled down (the service would do so anyway). */
const MAX_SIDE = 2048
/** PNGs larger than this after scaling are re-encoded as JPEG. */
const MAX_PNG_BYTES = 4 * 1024 * 1024

/**
 * Vision input cost at high detail (OpenAI / Azure OpenAI): fit within 2048×2048, scale the
 * short side to 768, then 170 tokens per 512px tile plus 85.
 */
export function visionTokens(width: number, height: number): number {
  let w = width
  let h = height
  const fit = Math.min(1, 2048 / Math.max(w, h))
  w *= fit
  h *= fit
  const shrink = Math.min(1, 768 / Math.min(w, h))
  w *= shrink
  h *= shrink
  return 85 + 170 * Math.ceil(w / 512) * Math.ceil(h / 512)
}

function loadBitmap(file: Blob): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file)
    const img = new Image()
    img.onload = () => {
      URL.revokeObjectURL(url)
      resolve(img)
    }
    img.onerror = () => {
      URL.revokeObjectURL(url)
      reject(new Error(`Not a readable image: ${file instanceof File ? file.name : 'pasted image'}`))
    }
    img.src = url
  })
}

/** Reads an image file into an attachment, scaling it down if needed. */
export async function toAttachment(file: File, id: string): Promise<Attachment> {
  if (!IMAGE_TYPES.includes(file.type)) throw new Error(`${file.name}: only PNG, JPEG, GIF and WebP pictures can be sent.`)
  const img = await loadBitmap(file)
  const scale = Math.min(1, MAX_SIDE / Math.max(img.naturalWidth, img.naturalHeight))
  const width = Math.round(img.naturalWidth * scale)
  const height = Math.round(img.naturalHeight * scale)

  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  canvas.getContext('2d')!.drawImage(img, 0, 0, width, height)
  // Screenshots stay PNG (sharp text); very large ones become JPEG.
  let url = canvas.toDataURL(file.type === 'image/jpeg' ? 'image/jpeg' : 'image/png', 0.92)
  if (url.length * 0.75 > MAX_PNG_BYTES) url = canvas.toDataURL('image/jpeg', 0.9)

  const name = file.name && file.name !== 'image.png' ? file.name : `screenshot-${new Date().toTimeString().slice(0, 8).replace(/:/g, '')}.png`
  return { id, name, url, width, height, tokens: visionTokens(width, height) }
}
