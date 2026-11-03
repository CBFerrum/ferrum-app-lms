import { describe, expect, it } from "vitest";
import { initialValues, launchUrl } from "./Scorm";

describe("what a SCORM package is told before it starts", () => {
  it("names the learner and resumes a SCORM 1.2 package where it stopped", () => {
    const v = initialValues({
      version: "1.2",
      launch_url: "/api/app/lms/lessons/l/scorm/f/index.html",
      mastery_score: "80",
      learner: { id: "asha@learn.test", name: "Asha Rao" },
      saved: { cmi: { core: { lesson_status: "incomplete", lesson_location: "p3", session_time: "00:05:00", exit: "suspend", total_time: "0000:05:00" }, suspend_data: "s" }, status: "Incomplete" },
    });
    expect(v).toEqual({
      core: { lesson_status: "incomplete", lesson_location: "p3", total_time: "0000:05:00", student_id: "asha@learn.test", student_name: "Asha Rao", entry: "resume", credit: "credit", lesson_mode: "normal" },
      suspend_data: "s",
      student_data: { mastery_score: "80" },
    });
  });

  it("starts a SCORM 2004 package afresh, and lets an instructor's preview run without credit", () => {
    expect(initialValues({ version: "2004", launch_url: "/x", mastery_score: null, learner: null, saved: null })).toEqual({
      learner_id: "preview",
      learner_name: "Preview",
      entry: "ab-initio",
      credit: "no-credit",
      mode: "browse",
    });
    const resumed = initialValues({ version: "2004", launch_url: "/x", mastery_score: null, learner: { id: "b@x.test", name: "Ben" }, saved: { cmi: { location: "7", session_time: "PT5M", exit: "suspend" }, status: "Incomplete" } });
    expect(resumed).toEqual({ location: "7", learner_id: "b@x.test", learner_name: "Ben", entry: "resume", credit: "credit", mode: "normal" });
  });
});

describe("what an xAPI package is launched with", () => {
  it("names Learn's endpoint on this origin, the learner, the enrolment and the activity", () => {
    const url = launchUrl(
      { launch_url: "/api/app/lms/lessons/l/package/f/story.html", endpoint: "/api/app/lms/xapi/l/", activity_id: "https://acme.test/rust", actor: { objectType: "Agent", name: "Asha Rao", mbox: "mailto:asha@learn.test" }, registration: "e1", preview: false },
      "https://acme.example",
    );
    const [path, query] = url.split("?");
    const q = new URLSearchParams(query);
    expect(path).toBe("/api/app/lms/lessons/l/package/f/story.html");
    expect([q.get("endpoint"), q.get("activity_id"), q.get("registration"), q.get("auth")]).toEqual(["https://acme.example/api/app/lms/xapi/l/", "https://acme.test/rust", "e1", "Basic Og=="]);
    expect(JSON.parse(q.get("actor")!)).toEqual({ objectType: "Agent", name: "Asha Rao", mbox: "mailto:asha@learn.test" });
    const withQuery = launchUrl({ launch_url: "/x/story.html?lang=en", endpoint: "/e/", activity_id: "a:b", actor: {}, registration: null, preview: true }, "https://o.test");
    expect(withQuery.startsWith("/x/story.html?lang=en&endpoint=")).toBe(true);
    expect(new URLSearchParams(withQuery.split("?")[1]).has("registration")).toBe(false);
  });
});
