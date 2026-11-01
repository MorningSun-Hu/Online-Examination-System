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
  const id = crypto.randomUUID();
  localStorage.setItem(key, id);
  return id;
}
