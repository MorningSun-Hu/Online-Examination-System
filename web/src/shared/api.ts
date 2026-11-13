export type ErrBody = {
  error?: { code?: string; message?: string; details?: unknown };
};

export async function api<T = unknown>(
  path: string,
  opts: RequestInit & { token?: string | null } = {},
): Promise<{ status: number; data: T }> {
  const headers = new Headers(opts.headers);
  if (opts.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  if (opts.token) {
    headers.set("authorization", `Bearer ${opts.token}`);
  }
  const res = await fetch(path, { ...opts, headers });
  const text = await res.text();
  let data: T;
  try {
    data = (text ? JSON.parse(text) : {}) as T;
  } catch {
    data = { raw: text } as T;
  }
  if (res.status === 204) {
    data = {} as T;
  }
  return { status: res.status, data };
}

export function errMsg(data: unknown, fallback = "请求失败"): string {
  const e = data as ErrBody;
  return e?.error?.message || fallback;
}

export function clientId(key: string): string {
  const saved = localStorage.getItem(key);
  if (saved) {
    return saved;
  }
  const id = newClientId();
  localStorage.setItem(key, id);
  return id;
}

function newClientId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") {
    try {
      return c.randomUUID();
    } catch {
      /* http://局域网IP 不是 secure context，randomUUID 会抛错 */
    }
  }
  const bytes = new Uint8Array(16);
  if (c && typeof c.getRandomValues === "function") {
    c.getRandomValues(bytes);
  } else {
    for (let i = 0; i < 16; i++) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
