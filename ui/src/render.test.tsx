// The screens' parts drawn to markup without a browser: what a learner or an instructor is shown,
// checked on the page's first paint (effects and requests wait for a browser).

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { CourseLayout } from "./CourseLayout";
import { Md } from "./Md";
import { OutlinePanel } from "./OutlinePanel";
import { CourseCard } from "./parts";
import type { Card, CoursePage, OutlineChapter } from "./types";

const page = (node: React.ReactNode) =>
  renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  );

const chapters: OutlineChapter[] = [
  {
    id: "c1",
    title: "Basics",
    lessons: [
      { id: "a", title: "Hello", lesson_type: "Article", duration_minutes: 10, is_preview: true, required: true, completed: true },
      { id: "b", title: "Ownership", lesson_type: "Video", duration_minutes: 25, is_preview: false, required: true },
      { id: "c", title: "Extras", lesson_type: "Quiz", duration_minutes: 0, is_preview: false, required: false },
    ],
  },
  { id: "c2", title: "Empty one", lessons: [] },
];

describe("the outline", () => {
  const draw = (visitor: boolean, currentId?: string) => page(<OutlinePanel href={(l) => `/learn/x/${l.id}`} chapters={chapters} currentId={currentId} visitor={visitor} progress={visitor ? null : { enrollment: "e", status: "Active", progress_percent: "33.33", completed_lessons: 1, total_lessons: 2, cohort: null, certificate: null }} />);

  it("ticks what is done, marks the open lesson and says what is optional", () => {
    const html = draw(false, "b");
    expect(html).toContain("Completed. ");
    expect(html).toContain('aria-current="page"');
    expect(html).toContain("Optional");
    expect(html).toContain("1 of 2 required lessons");
    expect(html).toContain('href="/learn/x/b"');
    expect(html).toContain("No lessons yet.");
  });

  it("locks what a visitor cannot open and offers the previews", () => {
    const html = draw(true);
    expect(html).toContain("Locked. ");
    expect(html).toContain("Free preview");
    expect(html).not.toContain('href="/learn/x/b"');
    expect(html).toContain('href="/learn/x/a"');
    expect(html).toContain("3 lessons");
  });
});

describe("markdown on the page", () => {
  it("draws headings one level down, code as code, and nothing a reader could run", () => {
    const html = page(<Md source={"# Title\n\n```rust\nlet a = 1 < 2;\n```\n\n<script>x</script> [bad](javascript:x)"} />);
    expect(html).toContain("<h2>Title</h2>");
    expect(html).toContain("<pre");
    expect(html).toContain("let a = 1 &lt; 2;");
    expect(html).toContain("&lt;script&gt;x&lt;/script&gt;");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("javascript:");
  });
});

const card: Card = {
  id: "1",
  slug: "intro-to-rust",
  title: "Intro to Rust",
  summary: "Ownership first.",
  cover_image: null,
  level: "Beginner",
  tags: ["rust"],
  estimated_hours: "2.00",
  lessons: 12,
  total_minutes: 200,
  certificate: true,
  published_at: null,
  visibility: "Public",
  category: { id: "g", name: "General" },
  instructors: ["Asha Rao"],
};

describe("a course card", () => {
  it("is one link, with the facts and a monogram for a missing picture", () => {
    const html = page(<CourseCard card={card} to="/course/intro-to-rust" />);
    expect(html.match(/<a /g)).toHaveLength(1);
    expect(html).toContain("12 lessons · 3 h 20 min");
    expect(html).toContain("Certificate");
    expect(html).toContain("By Asha Rao");
    expect(html).toContain(">IT<");
  });

  it("shows where the viewer stands", () => {
    const html = page(<CourseCard card={{ ...card, my_enrollment: { id: "e", status: "Active", progress_percent: "40.00" } }} to="/x" />);
    expect(html).toContain('role="progressbar"');
    expect(html).toContain('aria-valuenow="40"');
    expect(page(<CourseCard card={{ ...card, my_enrollment: { id: "e", status: "Completed", progress_percent: "100" } }} to="/x" />)).toContain("Completed");
  });
});

describe("a course page", () => {
  const course: CoursePage = { ...card, lessons: 3, description: "About it.", self_enrollment: true, prerequisites: [], cohorts: [], chapters, my_enrollment: undefined };
  it("puts what to do beside what it is", () => {
    const html = page(<CourseLayout course={course} crumb={{ to: "/catalog", label: "Catalog" }} tagHref={(t) => `/catalog?tag=${t}`} lessonHref={(l) => `/learn/x/${l.id}`} visitor aside={<button>Enrol</button>} />);
    expect(html).toContain("Intro to Rust");
    expect(html).toContain("Taught by Asha Rao");
    expect(html).toContain("About it.");
    expect(html).toContain("Enrol");
    expect(html).toContain('href="/catalog?tag=rust"');
  });
});
