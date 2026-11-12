// The catalog: published courses to browse and search, filtered by category and level, with the
// learning paths that bundle several. The filters live in the address, so a filtered catalog is a
// link anyone can open. A course shows where its viewer stands in it.

import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Icon, StatePanel, PageLoading, toast, useDebounced, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { Failed } from "./states";
import { DueMarks, useDues } from "./Due";
import { getCatalog, getCategories, joinPath, keys } from "./api";
import { type Filters, filtersFrom, hasFilters, LEVELS, paramsFor, queryFor } from "./filters";
import { paths } from "./lesson";
import { CourseCard, LearnTabs } from "./parts";
import type { Path } from "./types";

export function Catalog() {
  useShellTitle(t("Catalog"));
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
  const dues = useDues();
  const categories = useQuery({ queryKey: keys.categories, queryFn: getCategories, staleTime: 300_000 });
  useLiveEntity("LmsCourse", useCallback(() => void qc.invalidateQueries({ queryKey: ["lms", "catalog"] }), [qc]));

  const courses = catalog.data?.pages.flatMap((p) => p.courses) ?? [];
  const bundles = !hasFilters(f) ? (catalog.data?.pages[0]?.learning_paths ?? []) : [];
  const [skipped, setSkipped] = useState<string[]>([]);

  const join = async (p: Path) => {
    const ok = await confirmDialog({ title: t("Join {title}?", { title: p.title }), message: t("You will be enrolled in each course of the path that you may take yourself."), confirmLabel: t("Join path") });
    if (!ok) return;
    try {
      const r = await joinPath(p.id);
      setSkipped(r.skipped.map((s) => `${s.course_title}: ${s.reason}`));
      toast(r.enrolled.length ? t("Enrolled in {value}", { value: (r.enrolled.length === 1 ? t("1 course") : t("{n} courses", { n: r.enrolled.length })) }) : t("You are already in the courses you can take"));
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
          <h1 className="sr-only">{t("Course catalog")}</h1>
          <LearnTabs />
          <p className="page-sub" aria-live="polite">
            {catalog.data ? `${(courses.length === 1 ? t("1 course") : t("{n} courses", { n: courses.length }))}${catalog.hasNextPage ? " and more" : ""}` : " "}
          </p>
        </div>
      </header>
      <div className="lms-filters" role="search">
        <input className="input lms-search" type="search" aria-label={t("Search courses")} placeholder={t("Search courses…")} value={typed} onChange={(e) => setTyped(e.target.value)} />
        <select className="select lms-select" aria-label={t("Category")} value={f.category} onChange={(e) => set({ category: e.target.value })}>
          <option value="">{t("All categories")}</option>
          {(categories.data ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              {isolate(c.category_name)}
            </option>
          ))}
        </select>
        <select className="select lms-select" aria-label={t("Level")} value={f.level} onChange={(e) => set({ level: e.target.value })}>
          <option value="">{t("Any level")}</option>
          {LEVELS.map((l) => (
            <option key={l}>{isolate(l)}</option>
          ))}
        </select>
        {f.tag && (
          <Badge tone="blue">
            {isolate(f.tag)}
            <button type="button" className="lms-badge-x" aria-label={t("Remove the {tag} filter", { tag: f.tag })} onClick={() => set({ tag: "" })}>
              <Icon name="x" size={11} />
            </button>
          </Badge>
        )}
        {hasFilters(f) && (
          <button type="button" className="btn btn-ghost btn-md" onClick={clear}>
            {t("Clear filters")}
          </button>
        )}
      </div>
      {skipped.length > 0 && (
        <Alert tone="warning">
          {t("Some courses of the path could not be joined:")}
          <ul>
            {skipped.map((s) => (
              <li key={s}>{isolate(s)}</li>
            ))}
          </ul>
        </Alert>
      )}
      {bundles.length > 0 && (
        <section aria-label={t("Learning paths")}>
          <h2 className="lms-group">{t("Learning paths")}</h2>
          <ul className="lms-lines">
            {bundles.map((p) => (
              <li key={p.id} className="lms-line">
                <div className="lms-line-main">
                  <strong>{isolate(p.title)}</strong>
                  <span className="faint">
                    {isolate((p.courses === 1 ? t("1 course") : t("{n} courses", { n: p.courses })))}
                    {p.summary ? ` · ${p.summary}` : ""}
                  </span>
                </div>
                <Button size="sm" onClick={() => void join(p)}>
                  {t("Join path")}
                </Button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {catalog.error ? (
        <Failed error={catalog.error} onRetry={() => void catalog.refetch()} title={t("Could not load the catalog")} />
      ) : !catalog.data ? (
        <PageLoading label={t("Loading courses")} />
      ) : !courses.length ? (
        hasFilters(f) ? (
          <StatePanel kind="no-matches" title={t("No course matches")} onClear={clear} />
        ) : (
          <StatePanel kind="empty" title={t("No courses are published yet")}>
            {t("When an instructor publishes a course it will be here.")}
          </StatePanel>
        )
      ) : (
        <>
          <h2 className="sr-only">{t("Courses")}</h2>
          <div className="lms-grid">
            {courses.map((c) => (
              <CourseCard key={c.id} card={c} to={paths.course(c.slug)} extra={c.my_enrollment?.status === "Active" ? <DueMarks enrollment={c.my_enrollment.id} dues={dues} /> : undefined} />
            ))}
          </div>
          {catalog.hasNextPage && (
            <div className="lms-more">
              <Button loading={catalog.isFetchingNextPage} onClick={() => void catalog.fetchNextPage()}>
                {t("Show more courses")}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

