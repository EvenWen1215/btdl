import type { AccountLibraryFolderKind, DownloadTask, NormalizedItem } from '../api/dto'

const SHARE_URL_PATTERN = /https?:\/\/[^\s<>"'`]+/gi
const INPUT_ID_PATTERN = /BV[0-9A-Za-z]{10}|[aA][vV]\d+/g
const TRAILING_SHARE_PUNCTUATION = /[,.;!?)\]}，。；！？、）】》」』]+$/u

const isBilibiliHost = (host: string): boolean => {
  const normalized = host.toLowerCase()
  return (
    normalized === 'bilibili.com' ||
    normalized.endsWith('.bilibili.com') ||
    normalized === 'b23.tv' ||
    normalized.endsWith('.b23.tv')
  )
}

const isDouyinHost = (host: string): boolean => {
  const normalized = host.toLowerCase()
  return (
    normalized === 'douyin.com' ||
    normalized.endsWith('.douyin.com') ||
    normalized === 'iesdouyin.com' ||
    normalized.endsWith('.iesdouyin.com')
  )
}

const isShareSourceHost = (host: string): boolean => isBilibiliHost(host) || isDouyinHost(host)

const DOUYIN_WORK_PATH = /^\/(?:share\/)?(?:video|note|slides)\/(\d{15,20})\/?$/i
const DOUYIN_SHORT_PATH = /^\/[A-Za-z0-9_-]+\/?$/

const douyinWorkId = (url: URL): string | null => {
  const fromPath = url.pathname.match(DOUYIN_WORK_PATH)?.[1]
  if (fromPath) return fromPath
  for (const key of ['modal_id', 'aweme_id', 'item_id', 'vid']) {
    const value = url.searchParams.get(key)
    if (value && /^\d{15,20}$/.test(value)) return value
  }
  return null
}

export const extractSourceInputs = (text: string): string[] => {
  const candidates: Array<{ index: number; value: string }> = []
  const urlRanges: Array<{ start: number; end: number }> = []

  for (const match of text.matchAll(SHARE_URL_PATTERN)) {
    const index = match.index ?? 0
    const raw = match[0]
    urlRanges.push({ start: index, end: index + raw.length })
    const candidate = raw.replace(TRAILING_SHARE_PUNCTUATION, '')
    try {
      const url = new URL(candidate)
      if (isShareSourceHost(url.hostname)) candidates.push({ index, value: candidate })
    } catch {
      // Ignore malformed URL-like text and let normal input validation handle the rest.
    }
  }

  for (const match of text.matchAll(INPUT_ID_PATTERN)) {
    const index = match.index ?? 0
    const value = match[0]
    const end = index + value.length
    if (urlRanges.some((range) => index >= range.start && index < range.end)) continue

    const before = index > 0 ? text[index - 1] : ''
    const after = end < text.length ? text[end] : ''
    if ((before && /[0-9A-Za-z]/.test(before)) || (after && /[0-9A-Za-z]/.test(after))) continue

    candidates.push({ index, value })
  }

  candidates.sort((left, right) => left.index - right.index)
  return Array.from(new Set(candidates.map((candidate) => candidate.value)))
}

export const isSingleWorkInput = (input: string): boolean => {
  const value = input.trim()
  if (/^(BV[0-9A-Za-z]{10}|av\d+)$/i.test(value)) return true
  try {
    const url = new URL(value)
    if (!['http:', 'https:'].includes(url.protocol)) return false
    const host = url.hostname.toLowerCase()
    if (host === 'v.douyin.com') return DOUYIN_SHORT_PATH.test(url.pathname)
    if (isDouyinHost(host)) return douyinWorkId(url) !== null
    if (['b23.tv', 'www.b23.tv'].includes(host)) return url.pathname.length > 1
    return ['bilibili.com', 'www.bilibili.com', 'm.bilibili.com'].includes(host)
      && (/^\/video\/(BV[0-9A-Za-z]{10}|av\d+)\/?$/i.test(url.pathname)
        || (/^\/(list\/)?watchlater\/?$/.test(url.pathname) && /^BV[0-9A-Za-z]{10}$/i.test(url.searchParams.get('bvid') ?? '')))
  } catch {
    return false
  }
}

export const bilibiliUserUrl = (mid: string | number | null | undefined): string | null => {
  const value = String(mid ?? '').trim()
  return /^\d+$/.test(value) ? `https://space.bilibili.com/${value}` : null
}

export const bilibiliFavoriteCategoryUrl = (
  mid: string | number | null | undefined,
  kind: AccountLibraryFolderKind,
): string | null => {
  const profileUrl = bilibiliUserUrl(mid)
  if (!profileUrl) return null
  return `${profileUrl}/favlist?ftype=${kind === 'created_favorite' ? 'create' : 'collect'}`
}

export const sourceVideoUrl = (item: NormalizedItem): string | null => {
  for (const part of item.parts) {
    const douyin = part.id.match(/^part:douyin:(\d+)$/)
    if (douyin) return `https://www.douyin.com/video/${douyin[1]}`
  }
  const part = item.parts.find((candidate) => candidate.bvid || candidate.aid)
  if (part?.bvid) return `https://www.bilibili.com/video/${part.bvid}`
  if (part?.aid) return `https://www.bilibili.com/video/av${part.aid}`
  return null
}

export const bilibiliTaskUrl = (task: DownloadTask): string | null => {
  const douyin = task.source_id.match(/^douyin:(\d+)$/)
  if (douyin) return `https://www.douyin.com/video/${douyin[1]}`

  const input = task.refresh_intent?.input
  const page = task.refresh_intent?.page_number
  const pageQuery = page && page > 1 ? `?p=${page}` : ''
  if (input?.kind === 'video_bvid') return `https://www.bilibili.com/video/${input.bvid}${pageQuery}`
  if (input?.kind === 'video_aid') return `https://www.bilibili.com/video/av${input.aid}${pageQuery}`
  if (input?.kind === 'bangumi_episode') return `https://www.bilibili.com/bangumi/play/ep${input.ep_id}`
  if (input?.kind === 'cheese_episode') return `https://www.bilibili.com/cheese/play/ep${input.ep_id}`

  const uploader = task.source_id.match(/^uploader:(\d+):videos$/)
  if (uploader) return `https://space.bilibili.com/${uploader[1]}/video`
  return null
}
