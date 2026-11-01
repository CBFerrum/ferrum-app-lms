// What Learn's endpoints answer (src/site.rs, enrollment.rs, quiz.rs, assignment.rs and the
// package README). Decimals arrive as strings and stay strings until they are shown.

export type LessonType = "Article" | "Video" | "Document" | "Quiz" | "Assignment";
export type Level = "Beginner" | "Intermediate" | "Advanced";
export type EnrollmentStatus = "Active" | "Completed" | "Withdrawn";
export type QuestionType = "Single Choice" | "Multiple Choice" | "True/False" | "Short Answer";

/** A lesson as an outline lists it. `completed` and the quiz and assignment links are the
 *  signed-in outline's; the public page leaves them out. */
export interface OutlineLesson {
  id: string;
  title: string;
  lesson_type: LessonType;
  duration_minutes: number;
  is_preview: boolean;
  required: boolean;
  quiz?: string | null;
  assignment?: string | null;
  completed?: boolean;
  completed_at?: string | null;
}

export interface OutlineChapter {
  id: string;
  title: string;
  summary?: string | null;
  lessons: OutlineLesson[];
}

/** The enrolment's standing, as the course page gives it. */
export interface Progress {
  enrollment: string;
  status: EnrollmentStatus;
  progress_percent: string | null;
  completed_lessons: number | null;
  total_lessons: number | null;
  cohort: string | null;
  certificate: string | null;
}

export interface Places {
  capacity: number;
  taken: number;
  /** Null: no limit. */
  free: number | null;
}

export interface CohortInfo {
  id: string;
  title: string;
  start_date: string;
  end_date: string;
  places: Places;
}

export interface Session {
  session: string;
  title: string;
  starts_at: string;
  local_start: string;
  time_zone: string;
  duration_minutes: number;
  join_url: string | null;
  cohort: string;
  cohort_title: string;
  course_title: string;
}

/** A course in a catalog. */
export interface Card {
  id: string;
  slug: string;
  title: string;
  summary: string | null;
  cover_image: string | null;
  level: Level | null;
  tags: string[];
  estimated_hours: string | null;
  lessons: number;
  total_minutes: number;
  certificate: boolean;
  published_at: string | null;
  visibility: "Internal" | "Public";
  category: { id: string; name: string | null } | null;
  instructors: string[];
  /** The signed-in catalog's: the caller's own enrolment. */
  my_enrollment?: { id: string; status: EnrollmentStatus; progress_percent: string | null } | null;
}

export interface CoursePage extends Omit<Card, "my_enrollment" | "lessons"> {
  lessons: number | null;
  description: string | null;
  self_enrollment: boolean;
  prerequisites: { id: string; slug: string; title: string }[];
  cohorts: CohortInfo[];
  chapters: OutlineChapter[];
  /** Signed in: */
  can_enroll?: boolean;
  enroll_blockers?: string[];
  is_author?: boolean;
  my_enrollment?: Progress;
  next_lesson?: OutlineLesson | null;
  upcoming_sessions?: Session[];
}

export interface Path {
  id: string;
  title: string;
  summary: string | null;
  courses: number;
}

export interface CatalogPage {
  courses: Card[];
  next_cursor: string | null;
  learning_paths?: Path[];
}

export interface LessonBody {
  id: string;
  title: string;
  lesson_type: LessonType;
  chapter?: string;
  course?: string;
  content: string | null;
  video_url?: string | null;
  video_embed_url: string | null;
  file_url: string | null;
  duration_minutes: number | null;
  is_preview?: boolean;
  required?: boolean;
}

export interface QuizState {
  id: string;
  title: string;
  description: string | null;
  passing_percentage: string;
  time_limit_minutes: number;
  questions: number;
  /** 0: no limit. */
  max_attempts: number;
  attempts_used: number;
  attempts_left: number | null;
  best_percentage: string | null;
  passed: boolean;
  in_progress: { attempt: string; deadline: string | null } | null;
}

export interface SubmissionRow {
  id: string;
  number: string | null;
  submission_no: number;
  status: "Submitted" | "Graded" | "Needs Revision";
  score: string | null;
  passed: boolean;
  submitted_at: string | null;
  late: boolean;
  feedback: string | null;
}

export interface AssignmentState {
  id: string;
  title: string;
  instructions: string | null;
  submission_type: "Text" | "File" | "Text or File";
  max_score: string;
  passing_score: string;
  due_date: string | null;
  overdue: boolean;
  accept_late: boolean;
  allow_resubmission: boolean;
  max_submissions: number;
  passed: boolean;
  can_submit: boolean;
  reason: string | null;
  submissions: SubmissionRow[];
}

/** A lesson as a learner (or an author) opens it. */
export interface LessonView {
  lesson: LessonBody;
  completed: boolean;
  completed_at?: string | null;
  quiz?: QuizState;
  assignment?: AssignmentState;
}

/** What `complete_lesson` answers. */
export interface CompletionSummary {
  enrollment: string;
  status: EnrollmentStatus;
  progress_percent: string | null;
  completed_lessons: number | null;
  total_lessons: number | null;
  completed_at: string | null;
  certificate: string | null;
  newly_completed: boolean;
}

// ───────────────────────────── quizzes ─────────────────────────────

export type AttemptStatus = "In Progress" | "Submitted" | "Timed Out";

/** The learner's response: option keys (in the order shown) or a short answer's text. */
export type Response = string[] | string | null;

export interface AttemptAnswer {
  question: string;
  question_text: string;
  question_type: QuestionType;
  options: { key: string; text: string }[] | null;
  points: string;
  response: Response;
  is_correct: boolean;
  points_awarded: string | null;
  /** Shown once the quiz reveals answers. */
  correct_answer: string[] | null;
  explanation: string | null;
}

export interface Attempt {
  id: string;
  quiz: string;
  quiz_title: string;
  attempt_no: number;
  status: AttemptStatus;
  started_at: string;
  submitted_at?: string | null;
  deadline: string | null;
  time_limit_minutes: number;
  passing_percentage: string;
  max_score: string;
  score?: string | null;
  percentage?: string | null;
  passed: boolean;
  answers: AttemptAnswer[];
  /** The server's count of seconds to the deadline when it answered; null without a limit.
   *  Only actions' answers carry it. */
  seconds_left?: number | null;
  version?: number;
}

export interface SubmitResult {
  attempt: Attempt;
  status: AttemptStatus;
  score: string;
  max_score: string;
  percentage: string;
  passed: boolean;
  late_answers_ignored: boolean;
}

/** A finished attempt in a quiz's history. */
export interface AttemptRow {
  id: string;
  attempt_no: number;
  status: AttemptStatus;
  percentage: string | null;
  score: string | null;
  max_score: string | null;
  passed: boolean;
  submitted_at: string | null;
}

// ───────────────────────────── dashboards ─────────────────────────────

export interface MyCourse {
  enrollment: string;
  course: { id: string; slug: string; title: string; cover_image: string | null };
  status: EnrollmentStatus;
  progress_percent: string | null;
  completed_lessons: number;
  total_lessons: number;
  enrolled_on: string | null;
  last_activity_at: string | null;
  completed_at: string | null;
  next_lesson: OutlineLesson | null;
  cohort: { id: string; title: string | null } | null;
  certificate: { id: string; number: string | null; verification_code: string | null } | null;
}

export interface DueAssignment {
  assignment: string;
  title: string;
  course: { id: string; title: string };
  lesson: string;
  due_date: string | null;
  status: "Not Submitted" | "Submitted" | "Needs Revision" | "Failed";
  overdue: boolean;
}

export interface MyCertificate {
  id: string;
  number: string | null;
  course_title: string;
  verification_code: string | null;
}

export interface MyPath {
  id: string;
  title: string;
  courses: number;
  completed: number;
  progress_percent: string;
}

export interface Dashboard {
  courses: MyCourse[];
  due_assignments: DueAssignment[];
  upcoming_sessions: Session[];
  certificates: MyCertificate[];
  learning_paths: MyPath[];
}

export interface TeachingCourse {
  id: string;
  slug: string;
  title: string;
  status: "Draft" | "Published" | "Archived";
  enrolled: number;
  active: number;
  completed: number;
  completion_rate: string;
  average_progress: string;
  submissions_to_grade: number;
}

export interface RosterLearner {
  enrollment: string;
  number: string | null;
  learner: string;
  learner_name: string | null;
  status: EnrollmentStatus;
  progress_percent: string | null;
  completed_lessons: number;
  total_lessons: number;
  enrolled_on: string | null;
  last_activity_at: string | null;
  completed_at: string | null;
  cohort: { id: string; title: string | null } | null;
  certificate: string | null;
  quizzes_passed: number;
  best_quiz_percentage: string | null;
  submissions_to_grade: number;
}

export interface Roster {
  course: { id: string; slug: string; title: string };
  enrolled: number;
  completed: number;
  completion_rate: string;
  learners: RosterLearner[];
}

/** The certificate check's answer. */
export interface CertificateCheck {
  valid: boolean;
  status: "Valid" | "Revoked";
  number: string | null;
  learner_name: string | null;
  course_title: string | null;
  issued_on: string | null;
  issuer: string | null;
  verification_code: string;
}
