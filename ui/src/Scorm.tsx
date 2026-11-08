// A SCORM lesson: its package plays in a frame of this page, which gives it the SCORM API
// (scorm-again, ../vendor/scorm-again) where SCORM content looks for it, in a window above its
// own. What the package records is saved to the learner's SCORM Progress (scorm_commit on the
// enrolment) when it commits and when it finishes, and once more as the page goes away; the
// next visit resumes from it. The server decides from it whether the lesson is complete.
// The package's files come from this server, below the lesson's address: the frame must be this
// page's origin for the package to reach the API.

import { useEffect, useRef, useState } from "react";
import { errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Spinner, toast } from "@ferrum/kit";
import scorm12 from "../vendor/scorm-again/scorm12.min.js?url";
import scorm2004 from "../vendor/scorm-again/scorm2004.min.js?url";
import { scormCommit } from "./api";
import { Glyph } from "./Glyph";
import type { ScormPlayer, XapiPlayer } from "./types";

/** The parts of a scorm-again API the player uses. */
interface Runtime {
  on(event: string, callback: (...args: unknown[]) => void): void;
  loadFromJSON(json: Record<string, unknown>, element?: string): void;
  renderCommitCMI(terminate: boolean): unknown;
}

type RuntimeClass = new (settings: Record<string, unknown>) => Runtime;

declare global {
  interface Window {
    Scorm12API?: RuntimeClass;
    Scorm2004API?: RuntimeClass;
    API?: Runtime;
    API_1484_11?: Runtime;
  }
}

const SCRIPTS = { "1.2": scorm12, "2004": scorm2004 } as const;

/** Loads a runtime's script once per page. */
function loadRuntime(version: ScormPlayer["version"]): Promise<RuntimeClass> {
  const have = version === "1.2" ? window.Scorm12API : window.Scorm2004API;
  if (have) return Promise.resolve(have);
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = SCRIPTS[version];
    script.async = true;
    script.onload = () => {
      const loaded = version === "1.2" ? window.Scorm12API : window.Scorm2004API;
      if (loaded) resolve(loaded);
      else reject(new Error(t("the SCORM runtime did not load")));
    };
    script.onerror = () => reject(new Error(t("the SCORM runtime could not be loaded")));
    document.head.appendChild(script);
  });
}

/** What the package is told about the learner and the attempt before it starts: what it saved
 *  last time, less that session's own time and way out (the runtime adds a session's time to the
 *  total once, as it ends). */
export function initialValues(player: ScormPlayer): Record<string, unknown> {
  const { session_time: _time, exit: _exit, ...saved } = player.saved?.cmi ?? {};
  const resuming = !!player.saved;
  const who = player.learner ?? { id: "preview", name: "Preview" };
  if (player.version === "1.2") {
    const { session_time: _coreTime, exit: _coreExit, ...core } = (saved.core as Record<string, unknown> | undefined) ?? {};
    const mastery = player.mastery_score ? { student_data: { ...((saved.student_data as object) ?? {}), mastery_score: player.mastery_score } } : {};
    return {
      ...saved,
      ...mastery,
      core: { ...core, student_id: who.id, student_name: who.name, entry: resuming ? "resume" : "ab-initio", credit: player.learner ? "credit" : "no-credit", lesson_mode: player.learner ? "normal" : "browse" },
    };
  }
  return { ...saved, learner_id: who.id, learner_name: who.name, entry: resuming ? "resume" : "ab-initio", credit: player.learner ? "credit" : "no-credit", mode: player.learner ? "normal" : "browse" };
}

/** The package's data out of what the runtime commits (`{cmi: {...}}`). */
const cmiOf = (committed: unknown) => ((committed as { cmi?: unknown } | null)?.cmi ?? committed) as Record<string, unknown>;

export function ScormLesson({ player, title, enrollment, lessonId, onProgress }: { player: ScormPlayer; title: string; enrollment?: string; lessonId: string; onProgress: () => void }) {
  const [ready, setReady] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const frame = useRef<HTMLDivElement>(null);
  const progress = useRef(onProgress);
  progress.current = onProgress;

  useEffect(() => {
    let runtime: Runtime | null = null;
    let stopped = false;
    let busy = false;
    let again = false;
    let finished = false;
    // One save at a time; commits that arrive meanwhile are sent once it is done, with the latest data.
    const save = (finishing: boolean) => {
      if (!runtime || !enrollment) return;
      if (busy && !finishing) {
        again = true;
        return;
      }
      busy = true;
      const cmi = cmiOf(runtime.renderCommitCMI(finishing));
      scormCommit(enrollment, lessonId, cmi, finishing)
        .then((r) => {
          if (r.completed && !stopped) progress.current();
        })
        .catch((e: unknown) => {
          if (!stopped) toast(t("Your progress in this lesson was not saved: {e}", { e: errorMessage(e) }), "error");
        })
        .finally(() => {
          busy = false;
          if (again && !stopped) {
            again = false;
            save(false);
          }
        });
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      save(true);
    };
    loadRuntime(player.version)
      .then((Runtime) => {
        if (stopped) return;
        runtime = new Runtime({ autocommit: false, lmsCommitUrl: false, logLevel: 5 });
        runtime.loadFromJSON(initialValues(player));
        const v12 = player.version === "1.2";
        runtime.on(v12 ? "LMSCommit" : "Commit", () => save(false));
        runtime.on(v12 ? "LMSFinish" : "Terminate", finish);
        if (v12) window.API = runtime;
        else window.API_1484_11 = runtime;
        setReady(true);
      })
      .catch((e: unknown) => setProblem(errorMessage(e)));
    window.addEventListener("pagehide", finish);
    return () => {
      window.removeEventListener("pagehide", finish);
      finish();
      stopped = true;
      if (window.API === runtime) delete window.API;
      if (window.API_1484_11 === runtime) delete window.API_1484_11;
    };
  }, [player, enrollment, lessonId]);

  if (problem) return <Alert tone="error">{isolate(problem)}</Alert>;
  return (
    <div className="lms-stack-sm">
      {!player.learner && <Alert tone="info">{t("A preview: the package runs, and nothing it records is kept.")}</Alert>}
      <div className="lms-scorm" ref={frame}>
        {ready ? <iframe src={player.launch_url} title={title} allow="fullscreen; autoplay" /> : <Spinner />}
      </div>
      <div className="row">
        <Button variant="ghost" onClick={() => void frame.current?.requestFullscreen?.()}>
          <Glyph name="eye" size={14} /> {t("Full screen")}
        </Button>
      </div>
    </div>
  );
}

/** An xAPI package's launch address with the parameters such packages read: Learn's endpoint (on
 *  this origin), a credential they send back (the session is the real one), the learner as an
 *  actor, the enrolment as the registration, and the activity. */
export function launchUrl(p: XapiPlayer, origin: string): string {
  const q = new URLSearchParams({ endpoint: `${origin}${p.endpoint}`, auth: "Basic Og==", actor: JSON.stringify(p.actor), activity_id: p.activity_id });
  if (p.registration) q.set("registration", p.registration);
  return `${p.launch_url}${p.launch_url.includes("?") ? "&" : "?"}${q.toString()}`;
}

/** An xAPI lesson: the package plays in a frame and sends its statements to Learn's endpoint
 *  itself; this page looks again now and then for the lesson's completion. */
export function XapiLesson({ player, title, done, onProgress }: { player: XapiPlayer; title: string; done: boolean; onProgress: () => void }) {
  const frame = useRef<HTMLDivElement>(null);
  const progress = useRef(onProgress);
  progress.current = onProgress;
  useEffect(() => {
    if (done || player.preview) return;
    const every = window.setInterval(() => progress.current(), 15_000);
    return () => {
      window.clearInterval(every);
      progress.current();
    };
  }, [done, player.preview]);
  return (
    <div className="lms-stack-sm">
      {player.preview && <Alert tone="info">{t("A preview: the package runs, and nothing it records is kept.")}</Alert>}
      <div className="lms-scorm" ref={frame}>
        <iframe src={launchUrl(player, window.location.origin)} title={title} allow="fullscreen; autoplay" />
      </div>
      <div className="row">
        <Button variant="ghost" onClick={() => void frame.current?.requestFullscreen?.()}>
          <Glyph name="eye" size={14} /> {t("Full screen")}
        </Button>
      </div>
    </div>
  );
}
