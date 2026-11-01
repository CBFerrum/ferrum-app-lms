// The catalog: published courses to browse and search, filtered by category and level, with the
// learning paths that bundle several. The filters live in the address, so a filtered catalog is a
// link anyone can open. A course shows where its viewer stands in it.

import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { errorMessage } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, EmptyState, ErrorState, Icon, PageLoading, toast, useDebounced, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { getCatalog, getCategories, joinPath, keys } from "./api";
import { type Filters, filtersFrom, hasFilters, LEVELS, paramsFor, queryFor } from "./filters";
import { plural } from "./format";
import { paths } from "./lesson";
import { CourseCard, LearnTabs } from "./parts";
import type { Path } from "./types";

export function Catalog() {
  useShellTitle("Catalog");
  const qc = useQueryClient();
  const [sp, setSp] = useSearchParams();
  const f = filtersFrom(sp);
  const set = (patch: Partial<Filters>) => setSp(paramsFor({ ...f, ...patch }), { replace: true });

  // The search box answers as it is typed; the address follows once the typing pauses.
  const [typed, setTyped] = useState(f.q);
  const wrote = useRef(f.q);
  const settled = useDebounced(typed.trim(), 300);
  useEffect(() => {
    if (settled !== wrote.current) {
      wrote.current = settled;
      setSp(paramsFor({ ...filtersFrom(sp), q: settled }), { replace: true });
    }
    // The address is read at the moment the typing settles, and nothing else should re-run this.
  }, [settled]);
  // A link, or Back, sets the search from outside.
  useEffect(() => {
    if (f.q !== wrote.current) {
      wrote.current = f.q;
      setTyped(f.q);
    }
  }, [f.q]);

  const catalog = useInfiniteQuery({
    queryKey: keys.catalog({ ...f }),
    initialPageParam: "",
    queryFn: ({ pageParam }) => getCatalog(queryFor(f, pageParam || undefined)),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const categories = useQuery({ queryKey: keys.categories, queryFn: getCategories, staleTime: 300_000 });
  useLiveEntity("LmsCourse", useCallback(() => void qc.invalidateQueries({ queryKey: ["lms", "catalog"] }), [qc]));

  const courses = catalog.data?.pages.flatMap((p) => p.courses) ?? [];
  const bundles = !hasFilters(f) ? (catalog.data?.pages[0]?.learning_paths ?? []) : [];
  const [skipped, setSkipped] = useState<string[]>([]);

  const join = async (p: Path) => {
    const ok = await confirmDialog({ title: `Join ${p.title}?`, message: "You will be enrolled in each course of the path that you may take yourself.", confirmLabel: "Join path" });
    if (!ok) return;
    try {
      const r = await joinPath(p.id);
      setSkipped(r.skipped.map((s) => `${s.course_title}: ${s.reason}`));
      toast(r.enrolled.length ? `Enrolled in ${plural(r.enrolled.length, "course")}` : "You are already in the courses you can take");
      void qc.invalidateQueries({ queryKey: ["lms"] });
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };

  const clear = () => {
    setTyped("");
    wrote.current = "";
    setSp({}, { replace: true });
  };

  return (
    <div className="lms-page">
      <header className="lms-head">
        <div>
          <h1 className="sr-only">Course catalog</h1>
          <LearnTabs />
          <p className="page-sub" aria-live="polite">
            {catalog.data ? `${plural(courses.length, "course")}${catalog.hasNextPage ? " and more" : ""}` : " "}
          </p>
        </div>
      </header>
      <div className="lms-filters" role="search">
        <input className="input lms-search" type="search" aria-label="Search courses" placeholder="Search courses…" value={typed} onChange={(e) => setTyped(e.target.value)} />
        <select className="select lms-select" aria-label="Category" value={f.category} onChange={(e) => set({ category: e.target.value })}>
          <option value="">All categories</option>
          {(categories.data ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              {c.category_name}
            </option>
          ))}
        </select>
        <select className="select lms-select" aria-label="Level" value={f.level} onChange={(e) => set({ level: e.target.value })}>
          <option value="">Any level</option>
          {LEVELS.map((l) => (
            <option key={l}>{l}</option>
          ))}
        </select>
        {f.tag && (
          <Badge tone="blue">
            {f.tag}
            <button type="button" className="lms-badge-x" aria-label={`Remove the ${f.tag} filter`} onClick={() => set({ tag: "" })}>
              <Icon name="x" size={11} />
            </button>
          </Badge>
        )}
        {hasFilters(f) && (
          <button type="button" className="btn btn-ghost btn-md" onClick={clear}>
            Clear filters
          </button>
        )}
      </div>
      {skipped.length > 0 && (
        <Alert tone="warning">
          Some courses of the path could not be joined:
          <ul>
            {skipped.map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
        </Alert>
      )}
      {bundles.length > 0 && (
        <section aria-label="Learning paths">
          <h2 className="lms-group">Learning paths</h2>
          <ul className="lms-lines">
            {bundles.map((p) => (
              <li key={p.id} className="lms-line">
                <div className="lms-line-main">
                  <strong>{p.title}</strong>
                  <span className="faint">
                    {plural(p.courses, "course")}
                    {p.summary ? ` · ${p.summary}` : ""}
                  </span>
                </div>
                <Button size="sm" onClick={() => void join(p)}>
                  Join path
                </Button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {catalog.error ? (
        <ErrorState error={catalog.error} onRetry={() => void catalog.refetch()} title="Could not load the catalog" />
      ) : !catalog.data ? (
        <PageLoading label="Loading courses" />
      ) : !courses.length ? (
        <EmptyState title={hasFilters(f) ? "No course matches" : "No courses are published yet"} icon="search">
          {hasFilters(f) ? (
            <button type="button" className="btn btn-default btn-md" onClick={clear}>
              Clear filters
            </button>
          ) : (
            <p className="muted">When an instructor publishes a course it will be here.</p>
          )}
        </EmptyState>
      ) : (
        <>
          <h2 className="sr-only">Courses</h2>
          <div className="lms-grid">
            {courses.map((c) => (
              <CourseCard key={c.id} card={c} to={paths.course(c.slug)} />
            ))}
          </div>
          {catalog.hasNextPage && (
            <div className="lms-more">
              <Button loading={catalog.isFetchingNextPage} onClick={() => void catalog.fetchNextPage()}>
                Show more courses
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

