import { describe, expect, it } from "vitest";
import { hasMedia, embedSrc, finishedByLearner, isExternal, paths, publicPaths } from "./lesson";

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
