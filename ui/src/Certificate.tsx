// A certificate in a learner's hands: see it (the print format, to print or save as a PDF) and
// share the link anyone can check it with.

import { api, bootstrap, docPath, errorMessage, openPrint, routeBase } from "@ferrum/client";
import { Button, toast } from "@ferrum/kit";
import { publicPaths } from "./lesson";

/** Where anyone checks a certificate: the public page of this workspace. */
export function verifyUrl(code: string): string {
  const workspace = bootstrap()?.workspace.slug ?? "";
  return `${location.origin}${routeBase("lms").replace(/\/$/, "")}${publicPaths.certificate(workspace, code)}`;
}

export function CertificateActions({ id, code, primary = true }: { id: string; code?: string | null; primary?: boolean }) {
  const view = async () => {
    try {
      await openPrint(docPath("LmsCertificate", id, "print"));
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const copy = async () => {
    try {
      const known = code ?? (await api.get<{ verification_code: string }>(docPath("LmsCertificate", id))).verification_code;
      await navigator.clipboard.writeText(verifyUrl(known));
      toast("Link copied. Anyone who has it can check your certificate.");
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  return (
    <>
      <Button variant={primary ? "primary" : "default"} icon="printer" onClick={() => void view()}>
        View certificate
      </Button>
      <Button icon="copy" onClick={() => void copy()}>
        Copy link to check it
      </Button>
    </>
  );
}
