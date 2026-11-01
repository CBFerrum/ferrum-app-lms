// Learn's endpoints, typed (src/site.rs and the package README): what a screen reads comes from
// /api/app/lms for people signed in and /api/site/{workspace}/lms for everyone; what it changes
// is a record or an action of the generic API, so the server's rules decide every time.

import { api, docPath } from "@ferrum/client";
import type {
  Attempt,
  AttemptRow,
  CatalogPage,
  CertificateCheck,
  CompletionSummary,
  CoursePage,
  Dashboard,
  LessonView,
  Roster,
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
export const reactivate = (enrollment: string) => act("LmsEnrollment", enrollment, "reactivate");
export const joinPath = (path: string) =>
  act<{ enrolled: unknown[]; already_enrolled: unknown[]; skipped: { course_title: string; reason: string }[] }>("LmsLearningPath", path, "join");

export const completeLesson = (enrollment: string, lesson: string) => act<CompletionSummary>("LmsEnrollment", enrollment, "complete_lesson", { lesson });
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
