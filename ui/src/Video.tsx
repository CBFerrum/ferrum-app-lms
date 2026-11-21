// An uploaded video, played from the links the server signed for this person, with its captions. What is
// watched is counted here (only play, never a seek) and reported in whole seconds; the server keeps the
// larger figure, caps it by elapsed time and alone decides that watching completes the lesson.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { t, isolate } from "@ferrum/client";
import { Alert } from "@ferrum/kit";
import { keys, videoProgress } from "./api";
import { counted, toReport } from "./lesson";
import { percentLabel } from "./outline";
import type { VideoPlayer } from "./types";

const REPORT_EVERY_MS = 15_000;
const RENEW_BEFORE_S = 300;

export function UploadedVideo({ video, title, enrollment, lessonId, onProgress }: { video: VideoPlayer; title: string; enrollment?: string; lessonId: string; onProgress: () => void }) {
  const qc = useQueryClient();
  const el = useRef<HTMLVideoElement>(null);
  const [broken, setBroken] = useState(false);
  const [unsaved, setUnsaved] = useState(false);
  const [done, setDone] = useState(false);
  const [watched, setWatched] = useState(video.percent_watched);
  const state = useRef({ last: 0, played: 0, sent: Math.floor(video.seconds_watched), fails: 0, at: 0, resume: 0 });
  const base = video.seconds_watched;
  const progress = useRef(onProgress);
  progress.current = onProgress;

  const report = (finishing = false) => {
    if (!enrollment) return;
    const s = state.current;
    const n = toReport(base, s.played, s.sent);
    if (n === null) return;
    s.sent = n;
    videoProgress(enrollment, lessonId, n, finishing)
      .then((r) => {
        s.fails = 0;
        setUnsaved(false);
        setWatched(r.percent_watched);
        if (r.completed) {
          setDone(true);
          progress.current();
        }
      })
      .catch(() => {
        // One lost report is never shown: the seconds stay and go with the next.
        s.sent = Math.min(s.sent, n - 1);
        s.fails += 1;
        if (s.fails >= 3) setUnsaved(true);
      });
  };
  const reportRef = useRef(report);
  reportRef.current = report;

  useEffect(() => {
    if (!enrollment) return;
    const tick = window.setInterval(() => reportRef.current(), REPORT_EVERY_MS);
    const leave = () => reportRef.current(true);
    window.addEventListener("pagehide", leave);
    return () => {
      window.clearInterval(tick);
      window.removeEventListener("pagehide", leave);
      reportRef.current(true);
    };
  }, [enrollment]);

  // The links last a few hours: read the lesson again when they are about to lapse, and carry on where it was.
  useEffect(() => {
    const left = video.expires_at - Date.now() / 1000;
    const id = window.setTimeout(() => void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) }), Math.max(0, left - RENEW_BEFORE_S) * 1000);
    return () => window.clearTimeout(id);
  }, [video.expires_at, lessonId, qc]);
  useEffect(() => {
    const v = el.current;
    if (v && state.current.resume > 0 && v.currentTime < 1) v.currentTime = state.current.resume;
  }, [video.src]);

  if (broken) {
    return (
      <Alert tone="warning">
        {t("This video cannot be played in this browser.")} <a href={video.src}>{t("Download it")}</a>
      </Alert>
    );
  }
  const seeking = () => {
    if (el.current) state.current.last = el.current.currentTime;
  };
  return (
    <>
      <video
        ref={el}
        className="lms-video lms-video-file"
        controls
        preload="metadata"
        src={video.src}
        aria-label={title}
        onLoadedMetadata={(e) => {
          const at = video.seconds_watched;
          if (at > 0 && at < (video.length_seconds || e.currentTarget.duration || 0)) e.currentTarget.currentTime = at;
          state.current.last = e.currentTarget.currentTime;
        }}
        onTimeUpdate={(e) => {
          const v = e.currentTarget;
          const s = state.current;
          s.played += counted(s.last, v.currentTime, !v.paused && !v.seeking);
          s.last = v.currentTime;
          s.resume = Math.floor(video.seconds_watched + s.played);
        }}
        onSeeking={seeking}
        onSeeked={seeking}
        onPause={() => report()}
        onEnded={() => report()}
        onError={() => {
          const code = el.current?.error?.code;
          // 4: the source is not supported here; a lapsed link is read again, not drawn as a failure.
          if (code === 4 && Date.now() / 1000 < video.expires_at) setBroken(true);
          else void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
        }}
      >
        {video.captions && <track kind="captions" src={video.captions} default />}
      </video>
      {enrollment && (
        <p className="faint lms-video-note" aria-live="polite">
          {done ? t("Watched. This lesson is complete.") : isolate(t("Watched {percent}", { percent: percentLabel(watched ?? 0) }))}
          {video.watch_percent ? <> · {t("This lesson completes at {percent}%", { percent: video.watch_percent })}</> : null}
        </p>
      )}
      {unsaved && <p className="faint">{t("Your watching is not being saved")}</p>}
    </>
  );
}
