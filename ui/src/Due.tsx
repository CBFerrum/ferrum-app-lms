// When a course is due, whether it is overdue, and whether it renews an expiring certificate: read
// from the learner's own enrolment records (the learner endpoints do not carry them), so a failed
// read just leaves the marks out.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { isolate, t, todayIso } from "@ferrum/client";
import { Badge, useLiveEntity } from "@ferrum/kit";
import { getDues, keys } from "./api";
import { dueLabel } from "./format";
import type { EnrolmentDue } from "./types";

/** The viewer's active enrolments' due dates, by enrolment id. */
export function useDues(): Map<string, EnrolmentDue> {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.dues, queryFn: getDues });
  useLiveEntity("LmsEnrollment", useCallback(() => void qc.invalidateQueries({ queryKey: keys.dues }), [qc]));
  return new Map((q.data ?? []).map((d) => [d.id, d]));
}

/** Overdue, Due in 3 days, Recertification: marks for an active enrolment, nothing for the rest. */
export function DueMarks({ enrollment, dues, today = todayIso() }: { enrollment: string | null | undefined; dues: Map<string, EnrolmentDue>; today?: string }) {
  const d = enrollment ? dues.get(enrollment) : undefined;
  if (!d) return null;
  const recert = (d.cycle ?? 1) > 1;
  if (!d.due_date && !recert) return null;
  const late = !!d.due_date && d.due_date < today;
  return (
    <p className="lms-card-facts">
      {late && <Badge tone="red">{t("Overdue")}</Badge>}
      {d.due_date && <span className={late ? "lms-late" : "faint"}>{isolate(dueLabel(d.due_date, today))}</span>}
      {recert && <Badge tone="blue" title={t("Your certificate is expiring: this renews it")}>{t("Recertification")}</Badge>}
    </p>
  );
}
