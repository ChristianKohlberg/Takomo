// Thin HTTP client over the takomo REST API.
//
// Two hard requirements come from the deployed store:
//   1. Render's edge WAF returns an HTML 403 block page for requests that send a
//      default library User-Agent. We always send an explicit User-Agent and treat
//      any non-JSON body as a WAF/proxy error rather than a store response.
//   2. The store speaks structured errors ({code,message,remedy,current_state,
//      allowed_transitions,...}). We preserve those verbatim so an agent can
//      self-correct instead of guessing.

export const USER_AGENT = "takomo-mcp/0.1";

export interface ClientConfig {
  baseUrl: string;
  token: string;
}

export interface RequestOptions {
  method?: string;
  path: string;
  query?: Record<string, string | number | undefined | null>;
  body?: unknown;
  idempotencyKey?: string;
}

// A structured error returned by the store (any non-2xx with a JSON body).
// The raw body is kept intact so the caller can relay message/remedy/
// allowed_transitions to the agent without reformatting.
export class StoreError extends Error {
  status: number;
  body: any;
  constructor(status: number, body: any) {
    const msg =
      (body && typeof body === "object" && (body.message || body.code)) ||
      `HTTP ${status}`;
    super(String(msg));
    this.name = "StoreError";
    this.status = status;
    this.body = body;
  }
}

// A transport-level failure (WAF block page, non-JSON body, network error).
export class TransportError extends Error {
  status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.name = "TransportError";
    this.status = status;
  }
}

function buildUrl(baseUrl: string, path: string, query?: RequestOptions["query"]): string {
  const base = baseUrl.replace(/\/+$/, "");
  const url = new URL(base + path);
  if (query) {
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null || v === "") continue;
      url.searchParams.set(k, String(v));
    }
  }
  return url.toString();
}

export class TakomoClient {
  private baseUrl: string;
  private token: string;

  constructor(cfg: ClientConfig) {
    this.baseUrl = cfg.baseUrl;
    this.token = cfg.token;
  }

  // Returns the parsed JSON body, or `null` for a 204 No Content response.
  async request<T = any>(opts: RequestOptions): Promise<T | null> {
    const url = buildUrl(this.baseUrl, opts.path, opts.query);
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
      "User-Agent": USER_AGENT,
      Accept: "application/json",
    };
    if (opts.body !== undefined) headers["Content-Type"] = "application/json";
    if (opts.idempotencyKey) headers["Idempotency-Key"] = opts.idempotencyKey;

    let res: Response;
    try {
      res = await fetch(url, {
        method: opts.method ?? "GET",
        headers,
        body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
      });
    } catch (err: any) {
      throw new TransportError(`Network error calling ${url}: ${err?.message ?? err}`);
    }

    if (res.status === 204) return null;

    const text = await res.text();
    const ctype = res.headers.get("content-type") ?? "";

    // An empty body is a legitimate response (e.g. a bare 404). Don't run it
    // through the WAF/non-JSON guard.
    if (text.trim().length === 0) {
      if (!res.ok) throw new StoreError(res.status, null);
      return null;
    }

    // WAF / proxy guard: the store only ever speaks JSON. An HTML body (or any
    // non-JSON body) means an edge block page or misconfigured proxy, not a
    // real store response.
    const looksJson = ctype.includes("application/json") || /^\s*[[{]/.test(text);
    if (!looksJson) {
      const snippet = text.slice(0, 200).replace(/\s+/g, " ").trim();
      throw new TransportError(
        `Non-JSON response (${res.status} ${ctype || "no content-type"}) from ${url}. ` +
          `This usually means an edge WAF block page. Verify the User-Agent is sent. Body starts: ${snippet}`,
        res.status
      );
    }

    let parsed: any = null;
    if (text.length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        throw new TransportError(`Could not parse JSON (${res.status}) from ${url}: ${text.slice(0, 200)}`, res.status);
      }
    }

    if (!res.ok) throw new StoreError(res.status, parsed);
    return parsed as T;
  }
}
