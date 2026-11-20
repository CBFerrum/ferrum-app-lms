// What an instructor tells everyone in a course: the list on the course page (the newest open, the others
// folded) and the dialog an instructor writes one in. The server keeps the record and queues the messages.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { ApiError, errorMessage, formatDateTime, t as tr, isolate } from "@ferrum/client";
import { Alert, Button, Dialog, Field, toast } from "@ferrum/kit";
import { announce, getAnnouncements, keys } from "./api";
import { Md } from "./Md";

/** The course's announcements for someone in it; nothing is drawn when there are none or the reader is not in the course. */
export function Announcements({ slug }: { slug: string }) {
  const q = useQuery({ queryKey: keys.announcements(slug), queryFn: () => getAnnouncements(slug), staleTime: 60_000, retry: false });
  const list = q.data?.announcements ?? [];
  if (!list.length) return null;
  return (
    <section aria-label={tr("Announcements")}>
      <h2 className="lms-group">{tr("Announcements")}</h2>
      {list.map((a, i) => (
        <details key={a.id} open={i === 0} className="lms-announcement">
          <summary>
            <strong>{isolate(a.title)}</strong> <span className="faint">{isolate(a.author_name ?? "")}{a.sent_at ? ` · ${formatDateTime(a.sent_at)}` : ""}</span>
          </summary>
          <Md source={a.body} />
        </details>
      ))}
    </section>
  );
}

/** An instructor's announcement to everyone in the course. */
export function AnnounceDialog({ course, slug, open, onClose }: { course: string; slug: string; open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fields, setFields] = useState<Record<string, string>>({});
  const send = async (e?: FormEvent) => {
    e?.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    setFields({});
    try {
      const r = await announce(course, { title: title.trim(), body });
      toast(tr("Sent to {told} learners", { told: r.told }));
      void qc.invalidateQueries({ queryKey: keys.announcements(slug) });
      setTitle("");
      setBody("");
      onClose();
    } catch (err) {
      if (err instanceof ApiError && err.fields.length) setFields(Object.fromEntries(err.fields.map((f) => [f.field, f.message])));
      else setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onClose={onClose} title={tr("Announce to the course")}>
      <form className="stack" onSubmit={(e) => void send(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void send()}>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <Field label={tr("Title")} htmlFor="lms-announce-title" required error={fields.title}>
          <input id="lms-announce-title" className="input" maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        <Field label={tr("Announcement")} htmlFor="lms-announce-body" required error={fields.body} hint={tr("Everyone enrolled is told. Markdown works.")}>
          <textarea id="lms-announce-body" className="textarea" rows={8} maxLength={20000} value={body} onChange={(e) => setBody(e.target.value)} />
        </Field>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <Button variant="ghost" onClick={onClose}>
            {tr("Cancel")}
          </Button>
          <Button variant="primary" type="submit" loading={busy} disabled={!title.trim() || !body.trim()}>
            {tr("Send")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
