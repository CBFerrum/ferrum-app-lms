// What a catalog is filtered by lives in its address, so a filtered view is a link anyone can
// open, send and come back to.

export const LEVELS = ["Beginner", "Intermediate", "Advanced"] as const;

export interface Filters {
  q: string;
  category: string;
  level: string;
  tag: string;
}

export const NO_FILTERS: Filters = { q: "", category: "", level: "", tag: "" };

export function filtersFrom(sp: URLSearchParams): Filters {
  const level = sp.get("level") ?? "";
  return {
    q: (sp.get("q") ?? "").trim().slice(0, 100),
    category: sp.get("category") ?? "",
    level: (LEVELS as readonly string[]).includes(level) ? level : "",
    tag: (sp.get("tag") ?? "").trim(),
  };
}

/** The address' query for some filters: only those that are set. */
export function paramsFor(f: Partial<Filters>): URLSearchParams {
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(f)) if (v) sp.set(k, v);
  return sp;
}

export const hasFilters = (f: Filters) => Object.values(f).some(Boolean);

/** The request a catalog page makes: the filters that are set, nothing for the others. */
export function queryFor(f: Filters, cursor?: string | null): Record<string, string> {
  return { ...Object.fromEntries(Object.entries(f).filter(([, v]) => v)), ...(cursor ? { cursor } : {}) };
}
