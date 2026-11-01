// A course's page, the same for people signed in and not: what it is, who teaches it, what it
// asks and gives, and its outline. What a visitor can do with it (enrol, continue, sign in) is
// the `aside` the page puts beside it.

import { type ReactNode } from "react";
import { Link } from "react-router";
import { Badge } from "@ferrum/kit";
import { courseFacts } from "./format";
import { Glyph } from "./Glyph";
import { Md } from "./Md";
import { OutlinePanel } from "./OutlinePanel";
import { Cover, LevelBadge } from "./parts";
import type { CoursePage, OutlineLesson } from "./types";

export function CourseLayout({
  course,
  crumb,
  tagHref,
  lessonHref,
  visitor,
  aside,
}: {
  course: CoursePage;
  /** Where "all courses" leads, and its name. */
  crumb: { to: string; label: string };
  tagHref: (tag: string) => string;
  lessonHref: (lesson: OutlineLesson) => string;
  /** Only the free previews open. */
  visitor: boolean;
  aside: ReactNode;
}) {
  const facts = courseFacts(course.lessons, course.total_minutes, course.estimated_hours);
  return (
    <div className="lms-page lms-course">
      <nav className="crumbs" aria-label="Breadcrumb">
        <Link to={crumb.to}>{crumb.label}</Link>
        <Glyph name="chevronRight" size={12} />
      </nav>
      <header className="lms-course-hero">
        <div className="lms-course-info">
          <div className="lms-card-tags">
            <LevelBadge level={course.level} />
            {course.category?.name && <span className="faint">{course.category.name}</span>}
            {course.visibility === "Internal" && <Badge>Team only</Badge>}
          </div>
          <h1 className="page-title lms-course-title">{course.title}</h1>
          {course.summary && <p className="lms-course-summary">{course.summary}</p>}
          <p className="lms-course-facts faint">
            {facts}
            {course.certificate && (
              <span className="lms-card-cert">
                <Glyph name="award" size={13} /> Certificate when you finish
              </span>
            )}
          </p>
          {course.instructors.length > 0 && <p className="lms-course-by">Taught by {course.instructors.join(", ")}</p>}
          {course.tags.length > 0 && (
            <ul className="lms-tags" aria-label="Topics">
              {course.tags.map((t) => (
                <li key={t}>
                  <Link to={tagHref(t)}>{t}</Link>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div className="lms-course-cover">
          <Cover title={course.title} slug={course.slug} image={course.cover_image} />
        </div>
      </header>
      <div className="lms-course-body">
        <div className="lms-course-main">
          {course.description && (
            <section aria-label="About this course">
              <h2 className="lms-group">About this course</h2>
              <Md source={course.description} />
            </section>
          )}
          <section aria-label="Course content">
            <h2 className="lms-group">Course content</h2>
            {course.chapters.length ? (
              <div className="lms-outline-static">
                <OutlinePanel href={lessonHref} chapters={course.chapters} progress={course.my_enrollment} visitor={visitor} />
              </div>
            ) : (
              <p className="muted">No lessons have been added yet.</p>
            )}
          </section>
        </div>
        <aside className="lms-course-aside">{aside}</aside>
      </div>
    </div>
  );
}

/** What a course asks for before it, as links. */
export function Prerequisites({ course, href }: { course: CoursePage; href: (slug: string) => string }) {
  if (!course.prerequisites.length) return null;
  return (
    <div className="lms-box-block">
      <h3 className="lms-box-head">Complete first</h3>
      <ul className="lms-plain">
        {course.prerequisites.map((p) => (
          <li key={p.id}>
            <Link to={href(p.slug)}>{p.title}</Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
