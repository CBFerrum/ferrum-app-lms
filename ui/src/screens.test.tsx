// Whole screens drawn to markup from data already in the query cache, without a browser: the
// first paint of the player at each stage of a course, and of the pages around it. They catch a
// screen that cannot draw what the server sends; how it feels is for a person with a browser.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { keys } from "./api";
import { AssignmentPanel } from "./Assignment";
import { BuilderRoute, KEY as BUILD, type Data } from "./Builder";
import { Catalog } from "./Catalog";
import { CourseDashboardRoute } from "./CourseDashboard";
import { CourseRoute } from "./Course";
import { Done } from "./Done";
import { Home } from "./Home";
import { PlayerRoute } from "./Player";
import { PublicCatalog, PublicCertificate, PublicCourse, PublicLesson } from "./Public";
import { Intro, Result, Sitting } from "./QuizPlayer";
import { Teaching } from "./Teaching";
import type { Attempt, AssignmentState, CoursePage, Dashboard, LessonView, OutlineChapter, QuizState, Roster } from "./types";

const draw = (path: string, route: string, element: React.ReactElement, fill: (qc: QueryClient) => void) => {
  const qc = new QueryClient();
  fill(qc);
  return renderToStaticMarkup(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path={route} element={element} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
};

const chapters = (done: string[] = []): OutlineChapter[] => [
  {
    id: "c1",
    title: "Basics",
    lessons: [
      { id: "l1", title: "Hello Rust", lesson_type: "Article", duration_minutes: 10, is_preview: true, required: true, completed: done.includes("l1") },
      { id: "l2", title: "Ownership on video", lesson_type: "Video", duration_minutes: 25, is_preview: false, required: true, completed: done.includes("l2") },
    ],
  },
  { id: "c2", title: "Practice", lessons: [{ id: "l3", title: "Ownership quiz", lesson_type: "Quiz", duration_minutes: 15, is_preview: false, required: true, quiz: "q1", completed: done.includes("l3") }] },
];

const course = (over: Partial<CoursePage> = {}): CoursePage => ({
  id: "course-1",
  slug: "intro-to-rust",
  title: "Intro to Rust",
  summary: "Ownership first.",
  description: "A **practical** start.",
  cover_image: null,
  level: "Beginner",
  tags: ["rust"],
  category: { id: "g", name: "General" },
  estimated_hours: "2.00",
  lessons: 3,
  total_minutes: 50,
  certificate: true,
  published_at: null,
  visibility: "Internal",
  self_enrollment: true,
  instructors: ["Asha Rao"],
  prerequisites: [],
  cohorts: [],
  chapters: chapters(),
  can_enroll: true,
  enroll_blockers: [],
  is_author: false,
  ...over,
});

const mine = (over = {}) => ({ enrollment: "enr-1", status: "Active" as const, progress_percent: "33.33", completed_lessons: 1, total_lessons: 3, cohort: null, certificate: null, ...over });

const article: LessonView = {
  lesson: { id: "l1", title: "Hello Rust", lesson_type: "Article", content: "# Welcome\n\nLet us begin.", video_embed_url: null, file_url: null, duration_minutes: 10 },
  completed: false,
};

const learnPath = "/learn/intro-to-rust/l1";
const learnRoute = "/learn/:slug/:lesson";

describe("the course player", () => {
  const player = (c: CoursePage, v: LessonView, path = learnPath) => draw(path, learnRoute, <PlayerRoute />, (qc) => (qc.setQueryData(keys.course("intro-to-rust"), c), qc.setQueryData(keys.lesson(v.lesson.id), v)));

  it("offers one step on an unfinished reading: finish it and go on", () => {
    const html = player(course({ my_enrollment: mine(), next_lesson: chapters()[0].lessons[0] }), article);
    expect(html).toContain("Hello Rust");
    expect(html).toContain("Complete and continue");
    expect(html).toContain("Lesson 1 of 3");
    expect(html).toContain("Welcome");
    expect(html).toContain("Course content");
    expect(html).toContain("1 of 3 lessons");
    expect(html).toContain('role="progressbar"');
    expect(html).not.toContain("Previous</a>");
  });

  it("offers Next once the lesson is done, with Previous on a later lesson", () => {
    const view = { ...article, lesson: { ...article.lesson, id: "l2", title: "Ownership on video", lesson_type: "Video" as const, video_embed_url: "https://www.youtube-nocookie.com/embed/abcdefghijk" }, completed: true };
    const html = player(course({ chapters: chapters(["l1", "l2"]), my_enrollment: mine({ progress_percent: "66.67", completed_lessons: 2 }) }), view, "/learn/intro-to-rust/l2");
    expect(html).toContain("Next lesson");
    expect(html).toContain("Previous");
    expect(html).toContain('<iframe src="https://www.youtube-nocookie.com/embed/abcdefghijk"');
    expect(html).toContain("Not playing?");
    expect(html).toContain("Completed");
  });

  it("leaves a quiz to be passed", () => {
    const quiz: QuizState = { id: "q1", title: "Ownership quiz", description: "Read carefully.", passing_percentage: "70.00", time_limit_minutes: 15, questions: 5, max_attempts: 3, attempts_used: 1, attempts_left: 2, best_percentage: "40.00", passed: false, in_progress: null };
    const view: LessonView = { lesson: { id: "l3", title: "Ownership quiz", lesson_type: "Quiz", content: null, video_embed_url: null, file_url: null, duration_minutes: 15 }, completed: false, quiz };
    const html = player(course({ chapters: chapters(["l1", "l2"]), my_enrollment: mine() }), view, "/learn/intro-to-rust/l3");
    expect(html).toContain("Passing the quiz completes this lesson.");
    expect(html).toContain("Start quiz".replace("Start quiz", "Try again"));
    expect(html).toContain("2 of 3 attempts left");
    expect(html).toContain("15 min");
    expect(html).toContain("70%");
    expect(html).not.toContain("Complete and continue");
  });

  it("shows a visitor a free preview, the lock on the rest and the way in", () => {
    const html = player(course({ can_enroll: true }), article);
    expect(html).toContain("free preview");
    expect(html).toContain("Locked. ");
    expect(html).toContain("See the course");
    expect(html).not.toContain("Complete and continue");
  });

  it("tells an instructor nothing counts, and where the builder is", () => {
    const html = player(course({ is_author: true }), article);
    expect(html).toContain("as its instructor");
    expect(html).toContain("/build/course-1?lesson=l1");
  });

  it("asks a learner who left to enrol again", () => {
    const html = player(course({ my_enrollment: mine({ status: "Withdrawn" }) }), article);
    expect(html).toContain("You left this course");
    expect(html).toContain("Enrol again");
  });
});

describe("the end of a course", () => {
  it("says it once, with the certificate in reach", () => {
    const html = renderToStaticMarkup(
      <QueryClientProvider client={new QueryClient()}>
        <MemoryRouter>
          <Done title="Intro to Rust" certificate="cert-1" onClose={() => {}} />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(html).toContain("You completed Intro to Rust");
    expect(html).toContain("View certificate");
    expect(html).toContain("Copy link to check it");
    expect(html).toContain('aria-label="Dismiss"');
  });
});

const attempt = (over: Partial<Attempt> = {}): Attempt => ({
  id: "att-1",
  quiz: "q1",
  quiz_title: "Ownership quiz",
  attempt_no: 2,
  status: "In Progress",
  started_at: "2026-11-01T10:00:00.000Z",
  deadline: "2026-11-01T10:15:00.000Z",
  time_limit_minutes: 15,
  passing_percentage: "70.00",
  max_score: "3.00",
  passed: false,
  seconds_left: 600,
  answers: [
    { question: "a", question_text: "Who **owns** a value?", question_type: "Single Choice", options: [{ key: "k1", text: "One owner" }, { key: "k2", text: "Everyone" }], points: "1.00", response: ["k1"], is_correct: false, points_awarded: null, correct_answer: null, explanation: null },
    { question: "b", question_text: "Pick the borrows", question_type: "Multiple Choice", options: [{ key: "m1", text: "&T" }, { key: "m2", text: "&mut T" }], points: "1.00", response: null, is_correct: false, points_awarded: null, correct_answer: null, explanation: null },
    { question: "c", question_text: "Name the checker", question_type: "Short Answer", options: null, points: "1.00", response: null, is_correct: false, points_awarded: null, correct_answer: null, explanation: null },
  ],
  ...over,
});

const render = (node: React.ReactElement) => renderToStaticMarkup(<QueryClientProvider client={new QueryClient()}>{node}</QueryClientProvider>);

describe("the quiz player", () => {
  it("draws a sitting: timer, question chips, the open question and the way to submit", () => {
    const html = render(<Sitting attempt={attempt()} onFinished={() => {}} onGone={() => {}} />);
    expect(html).toContain('role="timer"');
    expect(html).toContain("10:00");
    expect(html).toContain('aria-label="Question 1, answered"');
    expect(html).toContain('aria-label="Question 2, not answered"');
    expect(html).toContain('aria-current="step"');
    expect(html).toContain("Question 1 of 3");
    expect(html).toContain("<strong>owns</strong>");
    expect(html).toContain('role="radiogroup"');
    expect(html).toContain('type="radio"');
    expect(html).toContain("1 of 3 answered");
    expect(html).toContain("Submit quiz");
    expect(html).toContain("Clear my answer");
    expect(html).toContain("Your answers are saved as you go");
  });

  it("draws a sitting with no time limit without a clock", () => {
    const html = render(<Sitting attempt={attempt({ time_limit_minutes: 0, deadline: null, seconds_left: null })} onFinished={() => {}} onGone={() => {}} />);
    expect(html).not.toContain('role="timer"');
  });

  it("draws the result with what was right, what was wrong and the answers the quiz showed", () => {
    const done = attempt({
      status: "Submitted",
      score: "1.00",
      percentage: "33.33",
      passed: false,
      answers: attempt().answers.map((a, i) => ({ ...a, is_correct: i === 0, points_awarded: i === 0 ? "1.00" : "0.00", correct_answer: i === 0 ? ["One owner"] : ["&T", "&mut T"], explanation: i === 1 ? "Both are borrows." : null })),
    });
    const quiz: QuizState = { id: "q1", title: "Ownership quiz", description: null, passing_percentage: "70.00", time_limit_minutes: 15, questions: 3, max_attempts: 3, attempts_used: 2, attempts_left: 1, best_percentage: "33.33", passed: false, in_progress: null };
    const html = render(<Result attempt={done} late={false} quiz={quiz} starting={false} onAgain={() => {}} onBack={() => {}} />);
    expect(html).toContain("Not passed this time");
    expect(html).toContain("33.3%");
    expect(html).toContain("1 of 3 questions right");
    expect(html).toContain("Correct answer:");
    expect(html).toContain("&amp;mut T");
    expect(html).toContain("Both are borrows.");
    expect(html).toContain("Try again");
    expect(html).toContain("(1 left)");
  });

  it("says when time ran out", () => {
    const quiz: QuizState = { id: "q1", title: "Q", description: null, passing_percentage: "70", time_limit_minutes: 15, questions: 3, max_attempts: 0, attempts_used: 1, attempts_left: null, best_percentage: null, passed: false, in_progress: null };
    const html = render(<Result attempt={attempt({ status: "Timed Out", score: "0", percentage: "0" })} late quiz={quiz} starting={false} onAgain={() => {}} onBack={() => {}} />);
    expect(html).toContain("Time ran out");
  });

  it("lets a learner resume an attempt under way, by choice", () => {
    const quiz: QuizState = { id: "q1", title: "Q", description: null, passing_percentage: "70", time_limit_minutes: 15, questions: 3, max_attempts: 1, attempts_used: 1, attempts_left: 0, best_percentage: null, passed: false, in_progress: { attempt: "att-1", deadline: "2026-11-01T10:15:00.000Z" } };
    const html = render(<Intro quiz={quiz} starting={false} error={null} onStart={() => {}} onReview={() => {}} />);
    expect(html).toContain("You have an attempt under way");
    expect(html).toContain("Resume quiz");
    expect(html).not.toContain("disabled");
  });

  it("closes the door when the attempts are used up", () => {
    const quiz: QuizState = { id: "q1", title: "Q", description: null, passing_percentage: "70", time_limit_minutes: 0, questions: 3, max_attempts: 2, attempts_used: 2, attempts_left: 0, best_percentage: "50", passed: false, in_progress: null };
    const html = render(<Intro quiz={quiz} starting={false} error={null} onStart={() => {}} onReview={() => {}} />);
    expect(html).toContain("All 2 attempts used");
    expect(html).toContain("disabled");
  });
});

describe("assignments", () => {
  const state = (over: Partial<AssignmentState> = {}): AssignmentState => ({
    id: "as1",
    title: "Write a parser",
    instructions: "Parse **this**.",
    submission_type: "Text or File",
    max_score: "100.00",
    passing_score: "60.00",
    due_date: "2030-01-01",
    overdue: false,
    accept_late: true,
    allow_resubmission: true,
    max_submissions: 0,
    passed: false,
    can_submit: true,
    reason: null,
    submissions: [],
    ...over,
  });

  it("asks for work not yet handed in", () => {
    const html = render(<AssignmentPanel state={state()} lessonId="l9" onProgress={() => {}} />);
    expect(html).toContain("Write a parser");
    expect(html).toContain("<strong>this</strong>");
    expect(html).toContain("Your answer");
    expect(html).toContain("Choose a file");
    expect(html).toContain("Hand in");
  });

  it("shows a grade and the feedback, and takes a new version when changes were asked for", () => {
    const html = render(
      <AssignmentPanel
        state={state({ submissions: [{ id: "s1", number: "SUB-1", submission_no: 1, status: "Needs Revision", score: null, passed: false, submitted_at: "2026-11-01T10:00:00.000Z", late: false, feedback: "Add *tests*." }] })}
        lessonId="l9"
        onProgress={() => {}}
      />,
    );
    expect(html).toContain("Your instructor asked for changes.");
    expect(html).toContain("Add <em>tests</em>.");
    expect(html).toContain("Hand in");
  });

  it("lets work waiting for a grade be changed", () => {
    const html = render(
      <AssignmentPanel state={state({ can_submit: false, reason: "your submission is waiting to be graded: edit it instead", submissions: [{ id: "s1", number: null, submission_no: 1, status: "Submitted", score: null, passed: false, submitted_at: "2026-11-01T10:00:00.000Z", late: true, feedback: null }] })} lessonId="l9" onProgress={() => {}} />,
    );
    expect(html).toContain("(late)");
    expect(html).toContain("Change my work");
    expect(html).not.toContain("waiting to be graded: edit it instead");
  });

  it("gives the server's reason when nothing more can be handed in", () => {
    const html = render(<AssignmentPanel state={state({ can_submit: false, reason: "the deadline (2026-01-01) has passed", overdue: true })} lessonId="l9" onProgress={() => {}} />);
    expect(html).toContain("The deadline (2026-01-01) has passed.");
    expect(html).not.toContain("Hand in</button>");
  });
});

describe("my learning", () => {
  const dash: Dashboard = {
    courses: [
      { enrollment: "e1", course: { id: "course-1", slug: "intro-to-rust", title: "Intro to Rust", cover_image: null }, status: "Active", progress_percent: "33.33", completed_lessons: 1, total_lessons: 3, enrolled_on: "2026-10-01", last_activity_at: "2026-10-30T10:00:00.000Z", completed_at: null, next_lesson: chapters()[0].lessons[1], cohort: { id: "co", title: "Autumn" }, certificate: null },
      { enrollment: "e2", course: { id: "course-2", slug: "safety", title: "Site safety", cover_image: null }, status: "Completed", progress_percent: "100", completed_lessons: 4, total_lessons: 4, enrolled_on: "2026-09-01", last_activity_at: null, completed_at: "2026-09-20T10:00:00.000Z", next_lesson: null, cohort: null, certificate: { id: "cert-1", number: "CERT-2026-000001", verification_code: "ABCD-EFGH" } },
    ],
    due_assignments: [{ assignment: "as1", title: "Write a parser", course: { id: "course-1", title: "Intro to Rust" }, lesson: "l9", due_date: "2020-01-01", status: "Not Submitted", overdue: true }],
    upcoming_sessions: [{ session: "s1", title: "Office hour", starts_at: "2030-01-01T10:00:00.000Z", local_start: "2030-01-01T11:00:00+01:00", time_zone: "Europe/Berlin", duration_minutes: 60, join_url: "https://meet.example/x", cohort: "co", cohort_title: "Autumn", course_title: "Intro to Rust" }],
    certificates: [{ id: "cert-1", number: "CERT-2026-000001", course_title: "Site safety", verification_code: "ABCD-EFGH" }],
    learning_paths: [{ id: "p1", title: "Systems track", courses: 3, completed: 1, progress_percent: "33.33" }],
  };

  it("leads with where to go on, then everything else", () => {
    const html = draw("/", "/", <Home />, (qc) => qc.setQueryData(keys.dashboard, dash));
    expect(html).toContain("Pick up where you left off");
    expect(html).toContain("Up next: <strong>Ownership on video</strong>");
    expect(html).toContain("Continue");
    expect(html).toContain('href="/learn/intro-to-rust/l2"');
    expect(html).toContain("Completed");
    expect(html).toContain("Write a parser");
    expect(html).toContain("Not handed in");
    expect(html).toContain("days late");
    expect(html).toContain("Office hour");
    expect(html).toContain('href="https://meet.example/x"');
    expect(html).toContain("Systems track");
    expect(html).toContain("CERT-2026-000001");
    expect(html).toContain("2 courses · 1 completed");
  });

  it("welcomes someone who has not started", () => {
    const html = draw("/", "/", <Home />, (qc) => qc.setQueryData(keys.dashboard, { courses: [], due_assignments: [], upcoming_sessions: [], certificates: [], learning_paths: [] } satisfies Dashboard));
    expect(html).toContain("You have not started a course yet");
    expect(html).toContain('href="/catalog"');
  });

  it("shows the Teaching tab to an instructor", () => {
    const html = draw("/", "/", <Home />, (qc) => (qc.setQueryData(keys.dashboard, dash), qc.setQueryData(["me"], { user: { id: "u", email: "i@x.test", full_name: "I", is_admin: false, roles: ["Instructor"] } })));
    expect(html).toContain('href="/teaching"');
    expect(draw("/", "/", <Home />, (qc) => qc.setQueryData(keys.dashboard, dash))).not.toContain('href="/teaching"');
  });
});

describe("the catalog and a course", () => {
  it("lists courses with the viewer's own standing and the learning paths", () => {
    const card = { ...course(), lessons: 3, my_enrollment: { id: "e", status: "Active", progress_percent: "40.00" } };
    const html = draw("/catalog", "/catalog", <Catalog />, (qc) => qc.setQueryData(keys.catalog({ q: "", category: "", level: "", tag: "" }), { pages: [{ courses: [card], next_cursor: "more", learning_paths: [{ id: "p1", title: "Systems track", summary: null, courses: 3 }] }], pageParams: [""] }));
    expect(html).toContain("1 course and more");
    expect(html).toContain("Intro to Rust");
    expect(html).toContain('aria-valuenow="40"');
    expect(html).toContain("Systems track");
    expect(html).toContain("Show more courses");
    expect(html).toContain('type="search"');
  });

  it("says the reasons when enrolling is not possible", () => {
    const html = draw("/course/intro-to-rust", "/course/:ref", <CourseRoute />, (qc) => qc.setQueryData(keys.course("intro-to-rust"), course({ can_enroll: false, enroll_blockers: ["complete these courses first: Basics"], prerequisites: [{ id: "p", slug: "basics", title: "Basics" }] })));
    expect(html).toContain("You cannot enrol yet");
    expect(html).toContain("Complete these courses first: Basics");
    expect(html).toContain('href="/course/basics"');
  });

  it("offers cohorts with their places when enrolling is possible", () => {
    const html = draw("/course/intro-to-rust", "/course/:ref", <CourseRoute />, (qc) =>
      qc.setQueryData(keys.course("intro-to-rust"), course({ cohorts: [{ id: "co1", title: "Autumn", start_date: "2026-11-02", end_date: "2026-12-20", places: { capacity: 20, taken: 20, free: 0 } }, { id: "co2", title: "Spring", start_date: "2027-03-01", end_date: "2027-05-01", places: { capacity: 0, taken: 4, free: null } }] })),
    );
    expect(html).toContain("Take this course");
    expect(html).toContain("On my own");
    expect(html).toContain("Full");
    expect(html).toContain("No limit on places");
    expect(html).toContain("Enrol");
  });

  it("continues an enrolled course and shows the instructor's tools", () => {
    const html = draw("/course/intro-to-rust", "/course/:ref", <CourseRoute />, (qc) => qc.setQueryData(keys.course("intro-to-rust"), course({ is_author: true, my_enrollment: mine(), can_enroll: false })));
    expect(html).toContain("Your progress");
    expect(html).toContain("Continue");
    expect(html).toContain("You teach this course");
    expect(html).toContain("Open the builder");
  });

  it("shows the certificate of a finished course", () => {
    const html = draw("/course/intro-to-rust", "/course/:ref", <CourseRoute />, (qc) => qc.setQueryData(keys.course("intro-to-rust"), course({ chapters: chapters(["l1", "l2", "l3"]), my_enrollment: mine({ status: "Completed", progress_percent: "100", certificate: "cert-1" }), can_enroll: false })));
    expect(html).toContain("You completed this course");
    expect(html).toContain("View certificate");
  });
});

describe("teaching", () => {
  it("lists the courses with what is waiting", () => {
    const html = draw("/teaching", "/teaching", <Teaching />, (qc) =>
      qc.setQueryData(keys.teaching, [{ id: "course-1", slug: "intro-to-rust", title: "Intro to Rust", status: "Published", enrolled: 20, active: 12, completed: 8, completion_rate: "40.00", average_progress: "55.50", submissions_to_grade: 3 }]),
    );
    expect(html).toContain("1 course");
    expect(html).toContain("3 to grade");
    expect(html).toContain('href="/teaching/course-1"');
    expect(html).toContain('href="/build/course-1"');
  });

  it("invites an instructor with no courses to start one", () => {
    const html = draw("/teaching", "/teaching", <Teaching />, (qc) => qc.setQueryData(keys.teaching, []));
    expect(html).toContain("You do not teach a course yet");
  });

  const roster: Roster = {
    course: { id: "course-1", slug: "intro-to-rust", title: "Intro to Rust" },
    enrolled: 2,
    completed: 1,
    completion_rate: "50.00",
    learners: [
      { enrollment: "e1", number: "ENR-1", learner: "asha@school.test", learner_name: "Asha Rao", status: "Active", progress_percent: "40.00", completed_lessons: 2, total_lessons: 5, enrolled_on: "2026-10-01", last_activity_at: "2026-10-02T10:00:00.000Z", completed_at: null, cohort: null, certificate: null, quizzes_passed: 1, best_quiz_percentage: "80.00", submissions_to_grade: 2 },
      { enrollment: "e2", number: "ENR-2", learner: "bo@school.test", learner_name: null, status: "Completed", progress_percent: "100.00", completed_lessons: 5, total_lessons: 5, enrolled_on: "2026-10-01", last_activity_at: "2026-11-01T10:00:00.000Z", completed_at: "2026-11-01T10:00:00.000Z", cohort: null, certificate: "CERT-1", quizzes_passed: 2, best_quiz_percentage: "95.00", submissions_to_grade: 0 },
    ],
  };

  it("shows a class: the numbers, the bands and each learner", () => {
    const html = draw("/teaching/course-1", "/teaching/:course", <CourseDashboardRoute />, (qc) => qc.setQueryData(keys.roster("course-1"), roster));
    expect(html).toContain("Intro to Rust");
    expect(html).toContain("Where the class is");
    expect(html).toContain("Asha Rao");
    expect(html).toContain("bo@school.test");
    expect(html).toContain("2 to grade");
    expect(html).toContain("Finished");
    expect(html).toContain("Add learners");
    expect(html).toContain('aria-expanded="false"');
  });

  it("starts the builder from the data of a course", () => {
    const data: Data = {
      course: { id: "course-1", version: 3, title: "Intro to Rust", slug: "intro-to-rust", status: "Draft" },
      chapters: [{ id: "c1", title: "Basics", position: 1, version: 1 }, { id: "c2", title: "Practice", position: 2, version: 1 }],
      lessons: [
        { id: "l1", title: "Hello Rust", chapter: "c1", position: 1, version: 1, lesson_type: "Article", duration_minutes: 10, is_preview: true, required: true, quiz: null, assignment: null },
        { id: "l3", title: "Ownership quiz", chapter: "c2", position: 1, version: 1, lesson_type: "Quiz", duration_minutes: 15, is_preview: false, required: true, quiz: "q1", assignment: null },
      ],
      quizzes: { q1: 0 },
    };
    const html = draw("/build/course-1", "/build/:course", <BuilderRoute />, (qc) => qc.setQueryData(BUILD("course-1"), data));
    expect(html).toContain("Basics");
    expect(html).toContain("Move Hello Rust down");
    expect(html).toContain("Move Ownership quiz up");
    expect(html).toContain("Move Basics down");
    expect(html).toContain("Publish (1 to fix)");
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain("Choose a lesson to write");
    expect(html).toContain('draggable="true"');
  });
});

describe("pages for anyone", () => {
  const pub = (path: string, route: string, element: React.ReactElement, fill: (qc: QueryClient) => void) => draw(path, route, element, fill);

  it("shows the public catalog with a way to check a certificate", () => {
    const html = pub("/c/acme", "/c/:workspace", <PublicCatalog />, (qc) => {
      qc.setQueryData(["lms", "public", "acme", { q: "", category: "", level: "", tag: "" }], { pages: [{ courses: [{ ...course(), my_enrollment: undefined }], next_cursor: null }], pageParams: [""] });
      qc.setQueryData(["lms", "public", "acme", "categories"], [{ id: "g", name: "General", courses: 1 }]);
    });
    expect(html).toContain("Check a certificate");
    expect(html).toContain('href="/c/acme/intro-to-rust"');
    expect(html).toContain("General (1)");
  });

  it("shows a course page with its free previews and a way to sign in", () => {
    const html = pub("/c/acme/intro-to-rust", "/c/:workspace/:slug", <PublicCourse />, (qc) => qc.setQueryData(["lms", "public", "acme", "course", "intro-to-rust"], course({ chapters: chapters() })));
    expect(html).toContain("Sign in to enrol");
    expect(html).toContain('href="/lms/course/intro-to-rust"');
    expect(html).toContain('href="/c/acme/intro-to-rust/lessons/l1"');
    expect(html).toContain("Locked. ");
  });

  it("shows a free-preview lesson", () => {
    const html = pub("/c/acme/intro-to-rust/lessons/l1", "/c/:workspace/:slug/lessons/:lesson", <PublicLesson />, (qc) => {
      qc.setQueryData(["lms", "public", "acme", "course", "intro-to-rust"], course());
      qc.setQueryData(["lms", "public", "acme", "lesson", "l1"], article.lesson);
    });
    expect(html).toContain("Hello Rust");
    expect(html).toContain("Free preview");
    expect(html).toContain("Sign in to enrol");
  });

  it("says a certificate is valid, or that it was revoked", () => {
    const cert = (valid: boolean) => ({ valid, status: valid ? "Valid" : "Revoked", number: "CERT-2026-000001", learner_name: "Asha Rao", course_title: "Intro to Rust", issued_on: "2026-11-01", issuer: "Acme", verification_code: "ABCD-EFGH" });
    const draws = (valid: boolean) => pub("/c/acme/certificate/ABCD-EFGH", "/c/:workspace/certificate/:code", <PublicCertificate />, (qc) => qc.setQueryData(["lms", "public", "acme", "certificate", "ABCD-EFGH"], cert(valid)));
    expect(draws(true)).toContain("This certificate is valid");
    expect(draws(true)).toContain("Asha Rao");
    expect(draws(false)).toContain("This certificate was revoked");
    expect(draws(true)).toContain("/api/site/acme/lms/certificates/ABCD-EFGH/view");
  });
});
