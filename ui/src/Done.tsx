// A course finished: said once, warmly, with the certificate in reach.

import { Link } from "react-router";
import { Icon } from "@ferrum/kit";
import { CertificateActions } from "./Certificate";
import { Glyph } from "./Glyph";
import { paths } from "./lesson";

export function Done({ title, certificate, onClose }: { title: string; certificate: string | null | undefined; onClose?: () => void }) {
  return (
    <section className="lms-done" aria-labelledby="lms-done-h">
      <span className="lms-done-mark">
        <Glyph name={certificate ? "award" : "circleCheck"} size={28} />
      </span>
      <div className="lms-done-text">
        <h2 id="lms-done-h">You completed {title}</h2>
        <p className="muted">{certificate ? "Your certificate is ready. Keep it, print it, or share the link that lets anyone check it." : "Well done. Every lesson the course asked for is finished."}</p>
        <div className="row">
          {certificate && <CertificateActions id={certificate} />}
          <Link className="btn btn-default btn-md" to={paths.home}>
            My learning
          </Link>
          <Link className="btn btn-default btn-md" to={paths.catalog}>
            Find another course
          </Link>
        </div>
      </div>
      {onClose && (
        <button type="button" className="icon-btn lms-done-close" aria-label="Dismiss" onClick={onClose}>
          <Icon name="x" size={16} />
        </button>
      )}
    </section>
  );
}
