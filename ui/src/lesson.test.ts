import { describe, expect, it } from "vitest";
import { nextAction } from "./outline";
import { byHandOf, counted, toReport, hasMedia, embedSrc, finishedByLearner, isExternal, paths, publicPaths } from "./lesson";

describe("the lesson page", () => {
  it("frames only https videos without credentials", () => {
    expect(embedSrc("https://www.youtube-nocookie.com/embed/abcdefghijk")).toBe("https://www.youtube-nocookie.com/embed/abcdefghijk");
    for (const bad of ["http://player.vimeo.com/video/1", "javascript:alert(1)", "data:text/html,x", "//evil.test/x", "https://user:pw@evil.test/", "not a url", "", null, undefined]) {
      expect(embedSrc(bad as string)).toBeNull();
    }
  });

  it("tells a link elsewhere from a file of this site", () => {
    expect(isExternal("https://docs.rs/serde")).toBe(true);
    expect(isExternal("/api/app/lms/lessons/1/file")).toBe(false);
  });

  it("knows which lessons carry media", () => {
    expect(hasMedia({ lesson_type: "Video", video_embed_url: "https://x", file_url: null })).toBe(true);
    expect(hasMedia({ lesson_type: "Video", video_embed_url: null, file_url: null })).toBe(false);
    expect(hasMedia({ lesson_type: "Document", video_embed_url: null, file_url: "/f" })).toBe(true);
    expect(hasMedia({ lesson_type: "Article", video_embed_url: null, file_url: null })).toBe(false);
    expect(finishedByLearner("Document") && !finishedByLearner("Quiz")).toBe(true);
  });

  it("builds addresses with every part encoded", () => {
    expect(paths.learn("intro-to-rust")).toBe("/learn/intro-to-rust");
    expect(paths.learn("intro", "l 1")).toBe("/learn/intro/l%201");
    expect(paths.build("c1", "l1")).toBe("/build/c1?lesson=l1");
    expect(publicPaths.certificate("acme", "ABCD-1234")).toBe("/c/acme/certificate/ABCD-1234");
    expect(publicPaths.lesson("acme", "intro", "l1")).toBe("/c/acme/intro/lessons/l1");
  });
});

describe("watching an uploaded video", () => {
  it("watching_counts_play_and_never_a_seek", () => {
    let last = 0;
    let played = 0;
    for (const now of [0.25, 0.5, 0.75, 1]) {
      played += counted(last, now, true);
      last = now;
    }
    expect(played).toBeCloseTo(1);
    expect(counted(1, 301, true)).toBe(0);
    expect(counted(10, 5, true)).toBe(0);
    expect(counted(10, 10.5, false)).toBe(0);
  });

  it("a_report_is_whole_seconds_and_only_when_it_grew", () => {
    expect(toReport(30, 4.9, 30)).toBe(34);
    expect(toReport(30, 0.5, 30)).toBeNull();
    expect(toReport(30, 4.9, 34)).toBeNull();
  });

  it("a_video_that_completes_by_watching_offers_no_mark_as_done", () => {
    const chapters = [{ id: "c1", title: "Main", position: 1, lessons: [{ id: "l2", title: "V", lesson_type: "Video" as const, duration_minutes: 5, is_preview: false, required: true }] }];
    const lesson = chapters[0].lessons[0];
    expect(nextAction(chapters, lesson, false, true).completes).toBe(true);
    expect(nextAction(chapters, lesson, false, true, byHandOf({ video: { watch_percent: 80 } })).completes).toBe(false);
    expect(byHandOf({ video: { watch_percent: null } })).toBeUndefined();
  });
});
