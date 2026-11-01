// What anyone with a link opens, signed in or not (the platform serves it below /lms/c/): the
// workspace's public catalog, a course's page with its free-preview lessons, and the check that a
// certificate is real. No sidebar and no session; the workspace is named in the address. Signing
// in to enrol is a link to the signed-in page, which the platform guards.

import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { ApiError, bootstrap, formatDate, routeBase } from "@ferrum/client";
import { Badge, Button, EmptyState, ErrorState, Icon, PageLoading, useDebounced, useShellTitle } from "@ferrum/kit";
import { certificatePage, checkCertificate, publicCatalog, publicCategories, publicCourse, publicLesson } from "./api";
import { CourseLayout, Prerequisites } from "./CourseLayout";
import { type Filters, filtersFrom, hasFilters, LEVELS, paramsFor, queryFor } from "./filters";
import { duration, plural } from "./format";
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
        <Glyph name="cap" size={20} /> {name}
      </Link>
      <span className="faint">Courses</span>
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
      <label htmlFor="lms-code">Check a certificate</label>
      <div className="row">
        <input id="lms-code" className="input lms-code" placeholder="Code on the certificate" autoComplete="off" spellCheck={false} value={code} onChange={(e) => setCode(e.target.value)} />
        <Button type="submit" disabled={!code.trim()}>
          Check
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
  useShellTitle(`Courses · ${bootstrap()?.workspace.name ?? workspace}`);

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
          <h1 className="page-title">Courses</h1>
          <p className="page-sub" aria-live="polite">
            {catalog.data ? `${plural(courses.length, "course")}${catalog.hasNextPage ? " and more" : ""}` : " "}
          </p>
        </div>
        <CheckCertificate workspace={workspace} />
      </div>
      <div className="lms-filters" role="search">
        <input className="input lms-search" type="search" aria-label="Search courses" placeholder="Search courses…" value={typed} onChange={(e) => setTyped(e.target.value)} />
        <select className="select lms-select" aria-label="Category" value={f.category} onChange={(e) => set({ category: e.target.value })}>
          <option value="">All categories</option>
          {(categories.data ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.courses})
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
      {catalog.error ? (
        <ErrorState error={catalog.error} onRetry={() => void catalog.refetch()} title="Could not load the courses" />
      ) : !catalog.data ? (
        <PageLoading label="Loading courses" />
      ) : !courses.length ? (
        <EmptyState title={hasFilters(f) ? "No course matches" : "No courses are open yet"} icon="search" />
      ) : (
        <>
          <h2 className="sr-only">Courses</h2>
          <div className="lms-grid">
            {courses.map((c) => (
              <CourseCard key={c.id} card={c} to={publicPaths.course(workspace, c.slug)} />
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
          <EmptyState title="This course is not open to the public" icon="search">
            <Link className="btn btn-default btn-md" to={publicPaths.catalog(workspace)}>
              See the courses that are
            </Link>
          </EmptyState>
        ) : (
          <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not open the course" />
        )}
      </div>
    );
  const c = q.data;
  if (!c) return <PageLoading label="Opening the course" />;
  return (
    <div className="lms-public-page">
      <Masthead workspace={workspace} />
      <CourseLayout
        course={c}
        crumb={{ to: publicPaths.catalog(workspace), label: "All courses" }}
        tagHref={(t) => `${publicPaths.catalog(workspace)}?${paramsFor({ tag: t })}`}
        lessonHref={(l) => publicPaths.lesson(workspace, slug, l.id)}
        visitor
        aside={
          <>
            <div className="lms-box">
              <h3 className="lms-box-head">Take this course</h3>
              <p className="faint lms-box-line">{c.self_enrollment ? "Sign in to enrol and keep track of your progress." : "Enrolment is by invitation. Sign in if you were invited."}</p>
              <div className="lms-box-actions">
                <a className="btn btn-primary btn-md" href={signInTo(slug)}>
                  Sign in to enrol
                </a>
              </div>
              {c.cohorts.length > 0 && (
                <div className="lms-box-block">
                  <h3 className="lms-box-head">Cohorts</h3>
                  <ul className="lms-plain">
                    {c.cohorts.map((g) => (
                      <li key={g.id}>
                        <strong>{g.title}</strong>
                        <br />
                        <span className="faint">
                          {formatDate(g.start_date)} to {formatDate(g.end_date)} · {g.places.free === null ? "No limit on places" : g.places.free === 0 ? "Full" : `${plural(g.places.free, "place")} left`}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <Prerequisites course={c} href={(s) => publicPaths.course(workspace, s)} />
            </div>
            {c.chapters.some((ch) => ch.lessons.some((l) => l.is_preview)) && <p className="faint lms-box-line">Lessons marked as free previews open without signing in.</p>}
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
        <EmptyState title="This lesson is not a free preview" icon="search">
          <Link className="btn btn-default btn-md" to={publicPaths.course(workspace, slug)}>
            See the course
          </Link>
        </EmptyState>
      </div>
    );
  const l = q.data;
  if (!l) return <PageLoading label="Opening the lesson" />;
  return (
    <div className="lms-public-page">
      <Masthead workspace={workspace} />
      <div className="lms-player">
        <div className="lms-player-main">
          <div className="lms-player-top">
            <Link className="lms-back" to={publicPaths.course(workspace, slug)}>
              <Glyph name="arrowLeft" size={14} /> {course.data?.title ?? "The course"}
            </Link>
          </div>
          <article className="lms-lesson" aria-labelledby="lms-lesson-title">
            <header className="lms-lesson-head">
              <p className="lms-eyebrow">{[here?.title, LESSON_LABEL[l.lesson_type], duration(l.duration_minutes)].filter(Boolean).join(" · ")}</p>
              <h1 id="lms-lesson-title">{l.title}</h1>
              <Badge tone="blue">Free preview</Badge>
            </header>
            {l.lesson_type === "Video" && <LessonVideo lesson={l} />}
            {l.lesson_type === "Document" && l.file_url && <LessonDocument url={l.file_url} />}
            <Md source={l.content} className="lms-reading" />
            <div className="lms-lesson-nav">
              <Link className="btn btn-default btn-md" to={publicPaths.course(workspace, slug)}>
                Back to the course
              </Link>
              <a className="btn btn-primary btn-md" href={signInTo(slug)}>
                Sign in to enrol
              </a>
            </div>
          </article>
        </div>
        {course.data && (
          <aside className="lms-outline" aria-label="Course outline">
            <OutlinePanel href={(x) => publicPaths.lesson(workspace, slug, x.id)} chapters={course.data.chapters} currentId={lesson} visitor />
          </aside>
        )}
      </div>
    </div>
  );
}

// ───────────────────────────── a certificate ─────────────────────────────

export function PublicCertificate() {
  const { workspace = "", code = "" } = useParams();
  const q = useQuery({ queryKey: ["lms", "public", workspace, "certificate", code], queryFn: () => checkCertificate(workspace, code), staleTime: 0 });
  useShellTitle("Certificate check");
  const missing = q.error instanceof ApiError && q.error.status === 404;
  return (
    <div className="lms-public-page lms-narrow">
      <Masthead workspace={workspace} />
      {q.isLoading ? (
        <PageLoading label="Checking the certificate" />
      ) : missing ? (
        <section className="lms-cert-card" data-state="unknown" aria-live="polite">
          <h1>No certificate has this code</h1>
          <p className="muted">Check that the code is copied in full. Letters of either case and dashes or spaces do not matter.</p>
          <CheckCertificate workspace={workspace} />
        </section>
      ) : q.error || !q.data ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not check the certificate" />
      ) : (
        <section className="lms-cert-card" data-state={q.data.valid ? "valid" : "revoked"} aria-live="polite">
          <span className="lms-done-mark">
            <Glyph name={q.data.valid ? "award" : "alert"} size={28} />
          </span>
          <h1>{q.data.valid ? "This certificate is valid" : "This certificate was revoked"}</h1>
          <dl className="lms-facts lms-cert-facts">
            <div>
              <dt>Awarded to</dt>
              <dd>{q.data.learner_name}</dd>
            </div>
            <div>
              <dt>For completing</dt>
              <dd>{q.data.course_title}</dd>
            </div>
            <div>
              <dt>Issued</dt>
              <dd>{q.data.issued_on ? formatDate(q.data.issued_on) : ""}</dd>
            </div>
            {q.data.issuer && (
              <div>
                <dt>By</dt>
                <dd>{q.data.issuer}</dd>
              </div>
            )}
            <div>
              <dt>Number</dt>
              <dd>{q.data.number}</dd>
            </div>
          </dl>
          <p className="faint lms-cert-code">Code {q.data.verification_code}</p>
          <div className="row">
            <a className="btn btn-default btn-md" href={certificatePage(workspace, code)} target="_blank" rel="noopener noreferrer">
              <Icon name="external" size={14} /> See the certificate
            </a>
            <Link className="btn btn-ghost btn-md" to={publicPaths.catalog(workspace)}>
              Courses
            </Link>
          </div>
        </section>
      )}
    </div>
  );
}
