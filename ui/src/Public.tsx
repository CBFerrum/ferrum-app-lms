// What anyone with a link opens, signed in or not (the platform serves it below /lms/c/): the
// workspace's public catalog, a course's page with its free-preview lessons, and the check that a
// certificate is real. No sidebar and no session; the workspace is named in the address. Signing
// in to enrol is a link to the signed-in page, which the platform guards.

import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { ApiError, bootstrap, formatDate, routeBase, t as tr, isolate } from "@ferrum/client";
import { Badge, Button, EmptyState, Icon, PageLoading, StatePanel, useDebounced, useShellTitle } from "@ferrum/kit";
import { Failed } from "./states";
import { certificatePage, checkCertificate, publicCatalog, publicCategories, publicCourse, publicLesson } from "./api";
import { CourseLayout, Prerequisites } from "./CourseLayout";
import { type Filters, filtersFrom, hasFilters, LEVELS, paramsFor, queryFor } from "./filters";
import { duration, priceText } from "./format";
import { Glyph, LESSON_LABEL } from "./Glyph";
import { publicPaths } from "./lesson";
import { LessonDocument, LessonVideo } from "./LessonPage";
import { Md } from "./Md";
import { OutlinePanel } from "./OutlinePanel";
import { CourseCard } from "./parts";

/** The signed-in page of a course, where enrolling is done. */
const signInTo = (slug: string) => `${routeBase("lms")}course/${encodeURIComponent(slug)}`;

function Masthead({ workspace }: { workspace: string }) {
  const name = bootstrap()?.workspace.name || workspace;
  return (
    <header className="lms-public-head">
      <Link to={publicPaths.catalog(workspace)} className="lms-public-brand">
        <Glyph name="cap" size={20} /> {isolate(name)}
      </Link>
      <span className="faint">{tr("Courses")}</span>
    </header>
  );
}

// ───────────────────────────── the catalog ─────────────────────────────

function CheckCertificate({ workspace }: { workspace: string }) {
  const navigate = useNavigate();
  const [code, setCode] = useState("");
  const go = (e: FormEvent) => {
    e.preventDefault();
    if (code.trim()) navigate(publicPaths.certificate(workspace, code.trim()));
  };
  return (
    <form className="lms-check" onSubmit={go}>
      <label htmlFor="lms-code">{tr("Check a certificate")}</label>
      <div className="row">
        <input id="lms-code" className="input lms-code" placeholder={tr("Code on the certificate")} autoComplete="off" spellCheck={false} value={code} onChange={(e) => setCode(e.target.value)} />
        <Button type="submit" disabled={!code.trim()}>
          {tr("Check")}
        </Button>
      </div>
    </form>
  );
}

export function PublicCatalog() {
  const { workspace = "" } = useParams();
  const [sp, setSp] = useSearchParams();
  const f = filtersFrom(sp);
  const set = (patch: Partial<Filters>) => setSp(paramsFor({ ...f, ...patch }), { replace: true });
  const clear = () => {
    setTyped("");
    wrote.current = "";
    setSp({}, { replace: true });
  };
  const [typed, setTyped] = useState(f.q);
  const wrote = useRef(f.q);
  const settled = useDebounced(typed.trim(), 300);
  useEffect(() => {
    if (settled !== wrote.current) {
      wrote.current = settled;
      setSp(paramsFor({ ...filtersFrom(sp), q: settled }), { replace: true });
    }
  }, [settled]);
  useEffect(() => {
    if (f.q !== wrote.current) {
      wrote.current = f.q;
      setTyped(f.q);
    }
  }, [f.q]);
  useShellTitle(tr("Courses · {value}", { value: bootstrap()?.workspace.name ?? workspace }));

  const catalog = useInfiniteQuery({
    queryKey: ["lms", "public", workspace, f],
    initialPageParam: "",
    queryFn: ({ pageParam }) => publicCatalog(workspace, queryFor(f, pageParam || undefined)),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const categories = useQuery({ queryKey: ["lms", "public", workspace, "categories"], queryFn: () => publicCategories(workspace), staleTime: 300_000 });
  const courses = catalog.data?.pages.flatMap((p) => p.courses) ?? [];

  return (
    <div className="lms-public-page">
      <Masthead workspace={workspace} />
      <div className="lms-head">
        <div>
          <h1 className="page-title">{tr("Courses")}</h1>
          <p className="page-sub" aria-live="polite">
            {catalog.data ? `${(courses.length === 1 ? tr("1 course") : tr("{n} courses", { n: courses.length }))}${catalog.hasNextPage ? " and more" : ""}` : " "}
          </p>
        </div>
        <CheckCertificate workspace={workspace} />
      </div>
      <div className="lms-filters" role="search">
        <input className="input lms-search" type="search" aria-label={tr("Search courses")} placeholder={tr("Search courses…")} value={typed} onChange={(e) => setTyped(e.target.value)} />
        <select className="select lms-select" aria-label={tr("Category")} value={f.category} onChange={(e) => set({ category: e.target.value })}>
          <option value="">{tr("All categories")}</option>
          {(categories.data ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              {isolate(c.name)} ({c.courses})
            </option>
          ))}
        </select>
        <select className="select lms-select" aria-label={tr("Level")} value={f.level} onChange={(e) => set({ level: e.target.value })}>
          <option value="">{tr("Any level")}</option>
          {LEVELS.map((l) => (
            <option key={l}>{isolate(l)}</option>
          ))}
        </select>
        {f.tag && (
          <Badge tone="blue">
            {isolate(f.tag)}
            <button type="button" className="lms-badge-x" aria-label={tr("Remove the {tag} filter", { tag: f.tag })} onClick={() => set({ tag: "" })}>
              <Icon name="x" size={11} />
            </button>
          </Badge>
        )}
        {hasFilters(f) && (
          <button type="button" className="btn btn-ghost btn-md" onClick={clear}>
            {tr("Clear filters")}
          </button>
        )}
      </div>
      {catalog.error ? (
        <Failed error={catalog.error} onRetry={() => void catalog.refetch()} title={tr("Could not load the courses")} />
      ) : !catalog.data ? (
        <PageLoading label={tr("Loading courses")} />
      ) : !courses.length ? (
        hasFilters(f) ? <StatePanel kind="no-matches" title={tr("No course matches")} onClear={clear} /> : <StatePanel kind="empty" title={tr("No courses are open yet")} />
      ) : (
        <>
          <h2 className="sr-only">{tr("Courses")}</h2>
          <div className="lms-grid">
            {courses.map((c) => (
              <CourseCard key={c.id} card={c} to={publicPaths.course(workspace, c.slug)} />
            ))}
          </div>
          {catalog.hasNextPage && (
            <div className="lms-more">
              <Button loading={catalog.isFetchingNextPage} onClick={() => void catalog.fetchNextPage()}>
                {tr("Show more courses")}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ───────────────────────────── a course ─────────────────────────────

export function PublicCourse() {
  const { workspace = "", slug = "" } = useParams();
  const q = useQuery({ queryKey: ["lms", "public", workspace, "course", slug], queryFn: () => publicCourse(workspace, slug) });
  useShellTitle(q.data ? `${q.data.title} · ${bootstrap()?.workspace.name ?? workspace}` : null);
  const missing = q.error instanceof ApiError && q.error.status === 404;
  if (q.error)
    return (
      <div className="lms-public-page">
        <Masthead workspace={workspace} />
        {missing ? (
          <EmptyState title={tr("This course is not open to the public")} icon="search">
            <Link className="btn btn-default btn-md" to={publicPaths.catalog(workspace)}>
              {tr("See the courses that are")}
            </Link>
          </EmptyState>
        ) : (
          <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not open the course")} />
        )}
      </div>
    );
  const c = q.data;
  if (!c) return <PageLoading label={tr("Opening the course")} />;
  return (
    <div className="lms-public-page">
      <Masthead workspace={workspace} />
      <CourseLayout
        course={c}
        crumb={{ to: publicPaths.catalog(workspace), label: tr("All courses") }}
        tagHref={(t) => `${publicPaths.catalog(workspace)}?${paramsFor({ tag: t })}`}
        lessonHref={(l) => publicPaths.lesson(workspace, slug, l.id)}
        visitor
        aside={
          <>
            <div className="lms-box">
              <h3 className="lms-box-head">{c.price ? tr("Buy this course: {value}", { value: priceText(c.price) }) : tr("Take this course")}</h3>
              <p className="faint lms-box-line">
                {!c.self_enrollment
                  ? tr("Enrolment is by invitation. Sign in if you were invited.")
                  : c.price
                    ? tr("Sign in to buy it. You are enrolled as soon as your payment arrives.")
                    : tr("Sign in to enrol and keep track of your progress.")}
              </p>
              <div className="lms-box-actions">
                <a className="btn btn-primary btn-md" href={signInTo(slug)}>
                  {c.price && c.self_enrollment ? tr("Sign in to buy") : tr("Sign in to enrol")}
                </a>
              </div>
              {c.cohorts.length > 0 && (
                <div className="lms-box-block">
                  <h3 className="lms-box-head">{tr("Cohorts")}</h3>
                  <ul className="lms-plain">
                    {c.cohorts.map((g) => (
                      <li key={g.id}>
                        <strong>{isolate(g.title)}</strong>
                        <br />
                        <span className="faint">
                          {g.places.free === null ? tr("{start} to {end} · No limit on places", { start: formatDate(g.start_date), end: formatDate(g.end_date) }) : g.places.free === 0 ? tr("{start} to {end} · Full", { start: formatDate(g.start_date), end: formatDate(g.end_date) }) : g.places.free === 1 ? tr("{start} to {end} · 1 place left", { start: formatDate(g.start_date), end: formatDate(g.end_date) }) : tr("{start} to {end} · {n} places left", { start: formatDate(g.start_date), end: formatDate(g.end_date), n: g.places.free })}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <Prerequisites course={c} href={(s) => publicPaths.course(workspace, s)} />
            </div>
            {c.chapters.some((ch) => ch.lessons.some((l) => l.is_preview)) && <p className="faint lms-box-line">{tr("Lessons marked as free previews open without signing in.")}</p>}
          </>
        }
      />
    </div>
  );
}

// ───────────────────────────── a free-preview lesson ─────────────────────────────

export function PublicLesson() {
  const { workspace = "", slug = "", lesson = "" } = useParams();
  const course = useQuery({ queryKey: ["lms", "public", workspace, "course", slug], queryFn: () => publicCourse(workspace, slug) });
  const q = useQuery({ queryKey: ["lms", "public", workspace, "lesson", lesson], queryFn: () => publicLesson(workspace, slug, lesson) });
  useShellTitle(q.data && course.data ? `${q.data.title} · ${course.data.title}` : null);
  const here = course.data?.chapters.find((c) => c.lessons.some((l) => l.id === lesson));
  if (q.error)
    return (
      <div className="lms-public-page">
        <Masthead workspace={workspace} />
        <EmptyState title={tr("This lesson is not a free preview")} icon="search">
          <Link className="btn btn-default btn-md" to={publicPaths.course(workspace, slug)}>
            {tr("See the course")}
          </Link>
        </EmptyState>
      </div>
    );
  const l = q.data;
  if (!l) return <PageLoading label={tr("Opening the lesson")} />;
  return (
    <div className="lms-public-page">
      <Masthead workspace={workspace} />
      <div className="lms-player">
        <div className="lms-player-main">
          <div className="lms-player-top">
            <Link className="lms-back" to={publicPaths.course(workspace, slug)}>
              <Glyph name="arrowLeft" size={14} /> {course.data?.title ?? tr("The course")}
            </Link>
          </div>
          <article className="lms-lesson" aria-labelledby="lms-lesson-title">
            <header className="lms-lesson-head">
              <p className="lms-eyebrow">{isolate([here?.title, LESSON_LABEL[l.lesson_type], duration(l.duration_minutes)].filter(Boolean).join(" · "))}</p>
              <h1 id="lms-lesson-title">{isolate(l.title)}</h1>
              <Badge tone="blue">{tr("Free preview")}</Badge>
            </header>
            {l.lesson_type === "Video" && <LessonVideo lesson={l} />}
            {l.lesson_type === "Document" && l.file_url && <LessonDocument url={l.file_url} />}
            <Md source={l.content} className="lms-reading" />
            <div className="lms-lesson-nav">
              <Link className="btn btn-default btn-md" to={publicPaths.course(workspace, slug)}>
                {tr("Back to the course")}
              </Link>
              <a className="btn btn-primary btn-md" href={signInTo(slug)}>
                {course.data?.price ? tr("Sign in to buy") : tr("Sign in to enrol")}
              </a>
            </div>
          </article>
        </div>
        {course.data && (
          <aside className="lms-outline" aria-label={tr("Course outline")}>
            <OutlinePanel href={(x) => publicPaths.lesson(workspace, slug, x.id)} chapters={course.data.chapters} currentId={lesson} visitor />
          </aside>
        )}
      </div>
    </div>
  );
}

// ───────────────────────────── a certificate ─────────────────────────────

/** What the check says of a certificate that is not valid, by its stored state. */
function certificateHeading(status: string): string {
  return status === "Expired" ? tr("This certificate has expired") : status === "Superseded" ? tr("This certificate was replaced by a newer one") : tr("This certificate was revoked");
}

export function PublicCertificate() {
  const { workspace = "", code = "" } = useParams();
  const q = useQuery({ queryKey: ["lms", "public", workspace, "certificate", code], queryFn: () => checkCertificate(workspace, code), staleTime: 0 });
  useShellTitle(tr("Certificate check"));
  const missing = q.error instanceof ApiError && q.error.status === 404;
  return (
    <div className="lms-public-page lms-narrow">
      <Masthead workspace={workspace} />
      {q.isLoading ? (
        <PageLoading label={tr("Checking the certificate")} />
      ) : missing ? (
        <section className="lms-cert-card" data-state="unknown" aria-live="polite">
          <h1>{tr("No certificate has this code")}</h1>
          <p className="muted">{tr("Check that the code is copied in full. Letters of either case and dashes or spaces do not matter.")}</p>
          <CheckCertificate workspace={workspace} />
        </section>
      ) : q.error || !q.data ? (
        <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not check the certificate")} />
      ) : (
        <section className="lms-cert-card" data-state={q.data.valid ? "valid" : "revoked"} aria-live="polite">
          <span className="lms-done-mark">
            <Glyph name={q.data.valid ? "award" : "alert"} size={28} />
          </span>
          <h1>{q.data.valid ? tr("This certificate is valid") : certificateHeading(q.data.status)}</h1>
          {q.data.status === "Superseded" && <p className="muted">{tr("A newer certificate for this course replaced it.")}</p>}
          <dl className="lms-facts lms-cert-facts">
            <div>
              <dt>{tr("Awarded to")}</dt>
              <dd>{isolate(q.data.learner_name ?? "")}</dd>
            </div>
            <div>
              <dt>{tr("For completing")}</dt>
              <dd>{isolate(q.data.course_title ?? "")}</dd>
            </div>
            <div>
              <dt>{tr("Issued")}</dt>
              <dd>{q.data.issued_on ? formatDate(q.data.issued_on) : ""}</dd>
            </div>
            {q.data.issuer && (
              <div>
                <dt>{tr("By")}</dt>
                <dd>{isolate(q.data.issuer)}</dd>
              </div>
            )}
            {q.data.expires_on && (
              <div>
                <dt>{q.data.status === "Expired" ? tr("Expired") : tr("Valid until")}</dt>
                <dd>{formatDate(q.data.expires_on)}</dd>
              </div>
            )}
            <div>
              <dt>{tr("Number")}</dt>
              <dd>{isolate(q.data.number ?? "")}</dd>
            </div>
          </dl>
          <p className="faint lms-cert-code">{tr("Code {verification_code}", { verification_code: q.data.verification_code })}</p>
          <div className="row">
            <a className="btn btn-default btn-md" href={certificatePage(workspace, code)} target="_blank" rel="noopener noreferrer">
              <Icon name="external" size={14} /> {tr("See the certificate")}
            </a>
            <Link className="btn btn-ghost btn-md" to={publicPaths.catalog(workspace)}>
              {tr("Courses")}
            </Link>
          </div>
        </section>
      )}
    </div>
  );
}
