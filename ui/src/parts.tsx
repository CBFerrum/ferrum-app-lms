// Small pieces more than one screen uses: the tabs at the top of Learn's pages, a progress bar, a
// course's picture, a sheet that slides up on a phone, and the course card of the catalogs.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { Link, NavLink } from "react-router";
import { Badge, Icon, type Tone } from "@ferrum/kit";
import { useMe } from "@ferrum/records";
import { courseFacts, hue, monogram } from "./format";
import { Glyph } from "./Glyph";
import { percent, percentLabel } from "./outline";
import { safeImage } from "./markdown";
import type { Card, Level } from "./types";

/** Whether the person teaches here: instructors, LMS managers and administrators. */
export function useTeacher(): boolean {
  const user = useMe().data?.user;
  return !!user && (user.is_admin || user.roles.some((r) => r === "Instructor" || r === "LMS Manager"));
}

/** The pages of Learn, at the top of each, as CRM's pipeline and Today are. */
export function LearnTabs() {
  const teacher = useTeacher();
  return (
    <nav className="lms-tabs" aria-label="Learn">
      <NavLink to="/" end>
        My learning
      </NavLink>
      <NavLink to="/catalog">Catalog</NavLink>
      {teacher && <NavLink to="/teaching">Teaching</NavLink>}
    </nav>
  );
}

/** While a screen that wants the whole width shows (the player), the records frame's sidebar steps
 *  aside; the stylesheet does it, and a frame that changes its classes simply keeps its sidebar. */
export function useFocusMode() {
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.lmsFocus = "";
    return () => {
      delete root.dataset.lmsFocus;
    };
  }, []);
}

export function Progress({ value, label }: { value: number | string | null | undefined; label: string }) {
  const v = percent(value);
  return (
    <div className="lms-progress" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v)} data-done={v >= 100 || undefined}>
      <span style={{ width: `${v}%` }} />
    </div>
  );
}

/** "42%" beside its bar, as the cards and rows show progress. */
export function ProgressLine({ value, label }: { value: number | string | null | undefined; label: string }) {
  return (
    <div className="lms-progress-line">
      <Progress value={value} label={label} />
      <span className="lms-progress-text">{percentLabel(value)}</span>
    </div>
  );
}

/** A course's picture, or a coloured monogram where it has none (or none this person can load). */
export function Cover({ title, slug, image }: { title: string; slug: string; image?: string | null }) {
  const [failed, setFailed] = useState(false);
  const src = image ? safeImage(image) : null;
  if (src && !failed) return <img className="lms-cover" src={src} alt="" loading="lazy" onError={() => setFailed(true)} />;
  return (
    <div className="lms-cover lms-cover-mono" data-hue={hue(slug)} aria-hidden="true">
      <span>{monogram(title)}</span>
    </div>
  );
}

const LEVEL_TONE: Record<Level, Tone> = { Beginner: "green", Intermediate: "blue", Advanced: "purple" };

export function LevelBadge({ level }: { level: Level | null | undefined }) {
  return level ? <Badge tone={LEVEL_TONE[level] ?? "neutral"}>{level}</Badge> : null;
}

/** A card of a catalog, one link for the whole of it. */
export function CourseCard({ card, to, status }: { card: Card; to: string; status?: ReactNode }) {
  const enrolment = card.my_enrollment;
  return (
    <article className="lms-card">
      <Cover title={card.title} slug={card.slug} image={card.cover_image} />
      <div className="lms-card-body">
        <div className="lms-card-tags">
          <LevelBadge level={card.level} />
          {card.category?.name && <span className="faint">{card.category.name}</span>}
        </div>
        <h3 className="lms-card-title">
          <Link to={to}>{card.title}</Link>
        </h3>
        {card.summary && <p className="lms-card-summary">{card.summary}</p>}
        <p className="lms-card-facts faint">
          {courseFacts(card.lessons, card.total_minutes, card.estimated_hours)}
          {card.certificate && (
            <span className="lms-card-cert" title="You earn a certificate">
              <Glyph name="award" size={13} /> Certificate
            </span>
          )}
        </p>
        {card.instructors.length > 0 && <p className="lms-card-by faint">By {card.instructors.slice(0, 2).join(", ")}{card.instructors.length > 2 ? ` and ${card.instructors.length - 2} more` : ""}</p>}
        {status ??
          (enrolment && enrolment.status !== "Withdrawn" ? (
            enrolment.status === "Completed" ? (
              <Badge tone="green">Completed</Badge>
            ) : (
              <ProgressLine value={enrolment.progress_percent} label={`Your progress in ${card.title}`} />
            )
          ) : null)}
      </div>
    </article>
  );
}

/** Slides up from the bottom of a phone (a centred dialog elsewhere): the native dialog gives it
 *  focus handling, Escape and an inert page behind. */
export function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title: string; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    else if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      className="lms-sheet"
      aria-label={title}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onMouseDown={(e) => {
        if (e.target === ref.current) onClose();
      }}
    >
      {open && (
        <div className="lms-sheet-inner">
          <header>
            <h2>{title}</h2>
            <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
              <Icon name="x" />
            </button>
          </header>
          <div className="lms-sheet-body">{children}</div>
        </div>
      )}
    </dialog>
  );
}
