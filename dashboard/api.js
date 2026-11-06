// The one place that talks to the server. Every failure becomes an ApiError with a message
// fit to show a person.

export class ApiError extends Error {
  constructor(status, code, message, details) {
    super(message)
    this.status = status
    this.code = code
    this.details = details
  }
}

const TOKEN_KEY = 'athenadb-token'

export function getToken() {
  try { return sessionStorage.getItem(TOKEN_KEY) || '' } catch { return '' }
}

export function setToken(token) {
  try { sessionStorage.setItem(TOKEN_KEY, token) } catch { /* lasts until reload */ }
}

async function request(method, path, body) {
  const headers = {}
  const token = getToken()
  if (token) headers.Authorization = `Bearer ${token}`
  // POST must always be JSON: the server refuses anything else.
  if (method === 'POST') headers['Content-Type'] = 'application/json'
  let res
  try {
    res = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
  } catch {
    throw new ApiError(0, 'NETWORK', "Can't reach the server. Is athenadb still running?")
  }
  let data = null
  try { data = await res.json() } catch { /* an empty or non-JSON body */ }
  if (!res.ok) {
    const e = data?.error
    throw new ApiError(res.status, e?.code || 'HTTP_ERROR', e?.message || `The server answered ${res.status}.`, e?.details)
  }
  return data
}

function query(params) {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') q.set(k, v)
  }
  const s = q.toString()
  return s ? `?${s}` : ''
}

export const api = {
  stats: () => request('GET', '/api/stats'),
  list: (params) => request('GET', `/api/vectors${query(params)}`),
  get: (id) => request('GET', `/api/vectors/${encodeURIComponent(id)}`),
  add: (vectors) => request('POST', '/api/vectors', { vectors }),
  remove: (id) => request('DELETE', `/api/vectors/${encodeURIComponent(id)}`),
  search: (body) => request('POST', '/api/search', body),
  projection: (params) => request('GET', `/api/projection${query(params)}`),
  snapshot: () => request('POST', '/api/snapshots', {}),
  reload: () => request('POST', '/api/reload', {}),
}
