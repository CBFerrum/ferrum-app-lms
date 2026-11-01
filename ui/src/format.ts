// Words for numbers and dates, the way a learner would say them. Pure, with the day passed in, so
// they are tested without a clock.

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** 45 → "45 min", 90 → "1 h 30 min", 120 → "2 h"; nothing for no time. */
export function duration(minutes: number | null | undefined): string {
  const m = Math.round(minutes ?? 0);
  if (m <= 0) return "";
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

/** Seconds on a clock face: 75 → "1:15", 3725 → "1:02:05". */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const [h, m, r] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  const two = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${two(m)}:${two(r)}` : `${m}:${two(r)}`;
}

/** Whole days from one date (YYYY-MM-DD) to another; negative when `to` is earlier. */
export function daysBetween(from: string, to: string): number {
  const day = (d: string) => {
    const [y, m, dd] = d.split("-").map(Number);
    return Date.UTC(y, m - 1, dd);
  };
  return Math.round((day(to) - day(from)) / 86_400_000);
}

/** When something is due, said against today: "Due today", "Due in 3 days", "2 days late". */
export function dueLabel(due: string | null | undefined, today: string): string {
  if (!due) return "";
  const n = daysBetween(today, due);
  if (n === 0) return "Due today";
  if (n === 1) return "Due tomorrow";
  if (n > 1 && n <= 14) return `Due in ${n} days`;
  if (n < 0) return n === -1 ? "1 day late" : `${-n} days late`;
  return `Due ${new Date(`${due}T00:00:00`).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`;
}

/** A live session's start in the cohort's own time zone, with its name: "Thu, 6 Nov, 6:00 pm CET". */
export function sessionStart(startsAt: string, zone: string): string {
  const options: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit", timeZoneName: "short" };
  try {
    return new Date(startsAt).toLocaleString(undefined, { ...options, timeZone: zone });
  } catch {
    // A zone name this browser does not know.
    return new Date(startsAt).toLocaleString(undefined, options);
  }
}

/** "12 lessons · 3 h 20 min" for a card. */
export function courseFacts(lessons: number | null | undefined, minutes: number | null | undefined, hours?: string | null): string {
  const time = duration(minutes) || (hours && Number(hours) > 0 ? `${Number(hours)} h` : "");
  return [lessons ? plural(lessons, "lesson") : "", time].filter(Boolean).join(" · ");
}

/** A name to show for a person, from what the server knows. */
export function personName(name?: string | null, email?: string | null): string {
  return (name ?? "").trim() || email || "Someone";
}

/** Two letters for a picture of a course that has none. */
export function monogram(title: string): string {
  const words = title.split(/[\s\-_/]+/).filter((w) => /[\p{L}\p{N}]/u.test(w));
  const letters = words.length > 1 ? [words[0], words[1]].map((w) => [...w][0]).join("") : [...(words[0] ?? "?")].slice(0, 2).join("");
  return letters.toUpperCase();
}

/** Which of the chart colours a course's picture takes: the same every time, spread over titles. */
export function hue(seed: string, of = 8): number {
  let h = 0;
  for (const c of seed) h = (h * 31 + c.codePointAt(0)!) >>> 0;
  return (h % of) + 1;
}
