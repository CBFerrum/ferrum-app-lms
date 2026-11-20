// Learn's endpoints, typed (src/site.rs and the package README): what a screen reads comes from
// /api/app/lms for people signed in and /api/site/{workspace}/lms for everyone; what it changes
// is a record or an action of the generic API, so the server's rules decide every time.

import { api, bootstrap, docPath, t } from "@ferrum/client";
import type {
  Attempt,
  AttemptRow,
  CatalogPage,
  CertificateCheck,
  CompletionSummary,
  CoursePage,
  Dashboard,
  Purchase,
  ScormSaved,
  LessonView,
  EnrolmentDue,
  GradeBook,
  Post,
  ReportedPost,
  ThreadList,
  ThreadPage,
  GradeEntry,
  Roster,
  RulePreview,
  SessionAttendance,
  SubmitResult,
  TeachingCourse,
} from "./types";
import type { Answers } from "./quiz";

const seg = encodeURIComponent;
const app = (path: string) => `/api/app/lms/${path}`;
const site = (workspace: string, path: string) => `/api/site/${seg(workspace)}/lms/${path}`;
const act = <T>(entity: string, id: string, name: string, body: unknown = {}) => api.post<T>(docPath(entity, id, "actions", name), body);

/** Which courses a catalog lists: the filters of its address. */
export interface CatalogQuery {
  q?: string;
  category?: string;
  level?: string;
  tag?: string;
  cursor?: string;
  limit?: number;
}

export const keys = {
  dashboard: ["lms", "dashboard"] as const,
  course: (slug: string) => ["lms", "course", slug] as const,
  lesson: (id: string) => ["lms", "lesson", id] as const,
  catalog: (q: CatalogQuery) => ["lms", "catalog", q] as const,
  categories: ["lms", "categories"] as const,
  teaching: ["lms", "teaching"] as const,
  roster: (course: string, cohort?: string) => ["lms", "roster", course, cohort ?? ""] as const,
  attempts: (quiz: string) => ["lms", "attempts", quiz] as const,
  dues: ["lms", "dues"] as const,
  cohort: (id: string) => ["lms", "cohort", id] as const,
  attendance: (cohort: string, session: string) => ["lms", "attendance", cohort, session] as const,
  gradebook: (course: string) => ["lms", "gradebook", course] as const,
  grades: (slug: string) => ["lms", "grades", slug] as const,
  threads: (slug: string, lesson?: string) => ["lms", "threads", slug, lesson ?? ""] as const,
  thread: (id: string) => ["lms", "thread", id] as const,
  reported: (course: string) => ["lms", "reported", course] as const,
};

// ───────────────────────────── signed in ─────────────────────────────

export const getDashboard = () => api.get<Dashboard>(app("dashboard"));
export const getCatalog = (q: CatalogQuery) => api.get<CatalogPage>(app("catalog"), { ...q });
export const getCourse = (slug: string) => api.get<{ course: CoursePage }>(app(`courses/${seg(slug)}`)).then((r) => r.course);
/** Opening a lesson makes it the enrolment's resume point, so nothing reads lessons ahead of time. */
export const getLesson = (id: string) => api.get<LessonView>(app(`lessons/${seg(id)}`));
export const getTeaching = () => api.get<{ courses: TeachingCourse[] }>(app("teaching")).then((r) => r.courses);
export const getRoster = (course: string, cohort?: string) => api.get<Roster>(app(`teaching/courses/${seg(course)}/roster`), { cohort });

/** The categories a learner may read, for the catalog's filter. */
export const getCategories = () =>
  api.get<{ rows: { id: string; category_name: string }[] }>(docPath("LmsCategory"), { fields: "category_name", sort: "category_name asc", limit: 200 }).then((r) => r.rows);

export const enrol = (course: string, cohort?: string | null) => api.post<{ id: string }>(docPath("LmsEnrollment"), { course, ...(cohort ? { cohort } : {}) });
export const withdraw = (enrollment: string) => act("LmsEnrollment", enrollment, "withdraw");
/** Invoices a paid course and asks for the money through the store (once: buying again asks again). */
export const buy = (course: string) => act<{ purchase: Purchase }>("LmsCourse", course, "buy");
export const reactivate = (enrollment: string) => act("LmsEnrollment", enrollment, "reactivate");
export const joinPath = (path: string) =>
  act<{ enrolled: unknown[]; already_enrolled: unknown[]; skipped: { course_title: string; reason: string }[] }>("LmsLearningPath", path, "join");

export const completeLesson = (enrollment: string, lesson: string) => act<CompletionSummary>("LmsEnrollment", enrollment, "complete_lesson", { lesson });

/** Saves what a SCORM package recorded. The last save (`finishing`) is usually sent as the page
 *  goes away, so it is a request that outlives the page while it is small enough to (64 KB). */
export function scormCommit(enrollment: string, lesson: string, cmi: unknown, finishing = false): Promise<ScormSaved> {
  const body = JSON.stringify({ lesson, cmi });
  if (!finishing || body.length > 60_000) return act<ScormSaved>("LmsEnrollment", enrollment, "scorm_commit", { lesson, cmi });
  const b = bootstrap();
  const headers: Record<string, string> = { accept: "application/json", "content-type": "application/json" };
  if (b?.app) headers["x-ferrum-ui"] = `${b.app.id}@${b.app.version}`;
  return fetch(docPath("LmsEnrollment", enrollment, "actions", "scorm_commit"), { method: "POST", headers, body, credentials: "same-origin", keepalive: true }).then(async (r) => {
    if (!r.ok) throw new Error(t("the SCORM data was not saved ({status})", { status: r.status }));
    return (await r.json()) as ScormSaved;
  });
}
export const startQuiz = (enrollment: string, quiz: string) => act<{ resumed: boolean; attempt: Attempt }>("LmsEnrollment", enrollment, "start_quiz", { quiz });
/** A save after the deadline closes the attempt, and then answers like a submit. */
export const saveAnswers = (attempt: string, answers: Answers) => act<{ attempt: Attempt } & Partial<SubmitResult>>("LmsQuizAttempt", attempt, "save_answers", { answers });
export const submitAnswers = (attempt: string, answers: Answers) => act<SubmitResult>("LmsQuizAttempt", attempt, "submit_answers", { answers });

/** A finished or running attempt, read as its learner's own record (answers included). */
export const getAttempt = (id: string) => api.get<Attempt>(docPath("LmsQuizAttempt", id));
export const getAttempts = (quiz: string) =>
  api
    .get<{ rows: AttemptRow[] }>(docPath("LmsQuizAttempt"), {
      filters: JSON.stringify({ quiz }),
      fields: "attempt_no,status,percentage,score,max_score,passed,submitted_at",
      sort: "attempt_no desc",
      limit: 100,
    })
    .then((r) => r.rows);

// ───────────────────────────── everyone ─────────────────────────────

export const publicCatalog = (workspace: string, q: CatalogQuery) => api.get<CatalogPage>(site(workspace, "courses"), { ...q });
export const publicCategories = (workspace: string) => api.get<{ categories: { id: string; name: string; courses: number }[] }>(site(workspace, "categories")).then((r) => r.categories);
export const publicCourse = (workspace: string, slug: string) => api.get<{ course: CoursePage }>(site(workspace, `courses/${seg(slug)}`)).then((r) => r.course);
export const publicLesson = (workspace: string, slug: string, lesson: string) =>
  api.get<{ lesson: LessonView["lesson"] }>(site(workspace, `courses/${seg(slug)}/lessons/${seg(lesson)}`)).then((r) => r.lesson);
export const checkCertificate = (workspace: string, code: string) => api.get<{ certificate: CertificateCheck }>(site(workspace, `certificates/${seg(code)}`)).then((r) => r.certificate);
/** The certificate as a page of its own: show it, print it or save it as a PDF. */
export const certificatePage = (workspace: string, code: string) => site(workspace, `certificates/${seg(code)}/view`);

// ───────────────────────────── teaching ─────────────────────────────

export interface EnrolResult {
  enrolled: { learner: string; enrollment: string; number: string | null }[];
  already_enrolled: { learner: string }[];
  skipped: { learner: string; reason: string }[];
}

/** Enrols people in a course by their sign-in addresses (at most 500 at a time). */
export const enrolLearners = (course: string, learners: string, cohort?: string) =>
  act<EnrolResult>("LmsCourse", course, "enroll_learners", { learners, ...(cohort ? { cohort } : {}) });

/** The records of the course's own, read as an instructor (their rights decide what comes back). */
export const rowsOf = <T>(entity: string, query: Record<string, string | number | boolean>) => api.get<{ rows: T[]; next_cursor?: string }>(docPath(entity), query).then((r) => r.rows);

/** The due dates and cycles of the viewer's own active enrolments (their records, read as their owner). */
export const getDues = () => rowsOf<EnrolmentDue>("LmsEnrollment", { filters: JSON.stringify({ status: "Active" }), fields: "due_date,cycle", limit: 500 });

/** An enrolment rule's Preview: how many people it would enrol now. */
export const previewRule = (rule: string) => act<RulePreview>("LmsEnrollmentRule", rule, "preview");

/** The learners recorded for a cohort's session so far, and the instructor's mark on one of them. */
export const sessionAttendance = (cohort: string, session: string) => act<SessionAttendance>("LmsCohort", cohort, "session_attendance", { session });
export const markAttendance = (cohort: string, session: string, learner: string, attended: boolean) =>
  act("LmsCohort", cohort, "mark_attendance", { session, learner, attended });

// ───────────────────────────── the gradebook ─────────────────────────────

export const getGradebook = (course: string) => api.get<GradeBook>(app(`teaching/courses/${seg(course)}/gradebook`));
/** The address of the book as a download (the server names the file). */
export const gradebookCsv = (course: string) => `${app(`teaching/courses/${seg(course)}/gradebook`)}?format=csv`;
export const getMyGrades = (slug: string) => api.get<GradeBook>(app(`courses/${seg(slug)}/grades`));
export const getGradeEntry = (item: string, enrollment: string) =>
  rowsOf<GradeEntry>("LmsGradeEntry", { filters: JSON.stringify({ item, enrollment }), fields: "points,excused,comment", limit: 1 }).then((r) => r[0] ?? null);
export const saveGradeEntry = (item: string, enrollment: string, body: { points: string | null; excused: boolean; comment: string | null }, existing: GradeEntry | null) =>
  existing ? api.patch<GradeEntry>(docPath("LmsGradeEntry", existing.id), body, { ifMatch: existing.version }) : api.post<GradeEntry>(docPath("LmsGradeEntry"), { item, enrollment, ...body });
export const clearGradeEntry = (entry: GradeEntry) => api.del(docPath("LmsGradeEntry", entry.id), { ifMatch: entry.version });

// ───────────────────────────── discussions ─────────────────────────────

export const getThreads = (slug: string, lesson?: string) => api.get<ThreadList>(app(`courses/${seg(slug)}/discussion`), { lesson });
export const getThread = (slug: string, thread: string) => api.get<ThreadPage>(app(`courses/${seg(slug)}/discussion`), { thread });
export const startThread = (course: string, body: { title: string; body: string; lesson?: string }) => act<{ post: Post }>("LmsCourse", course, "discuss", body);
export const reply = (course: string, thread: string, body: string) => act<{ post: Post }>("LmsCourse", course, "discuss", { thread, body });
export const follow = (course: string, thread: string, on: boolean) => act<{ following: boolean }>("LmsCourse", course, "follow", { thread, follow: on });
export const reportPost = (course: string, post: string, reason: string) => act<{ reported: boolean }>("LmsCourse", course, "report_post", { post, reason });
export const pin = (post: string) => act<{ post: Post }>("LmsPost", post, "pin");
export const closeThread = (post: string) => act<{ post: Post }>("LmsPost", post, "close");
export const markAnswer = (post: string) => act<{ post: Post }>("LmsPost", post, "mark_answer");
export const removePost = (post: string, reason: string) => act<{ post: Post }>("LmsPost", post, "remove", { reason });
export const getReported = (course: string) =>
  rowsOf<ReportedPost>("LmsPost", {
    filters: JSON.stringify({ course, reported: true }),
    fields: "title,body,author_name,author,report_reason,reported_by,removed,thread,created_at",
    sort: "created_at desc",
    limit: 200,
  }).then((rows) => rows.filter((r) => !r.removed));
