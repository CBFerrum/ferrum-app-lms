// A graded answer's filled rubric as the learner reads it: each criterion, the level chosen and its points.

import { t, isolate } from "@ferrum/client";
import type { RubricResult } from "./types";

export function RubricTable({ result }: { result: RubricResult }) {
  return (
    <table className="table lms-rubric-result">
      <caption className="sr-only">{t("How this was graded")}</caption>
      <thead>
        <tr>
          <th scope="col">{t("Criterion")}</th>
          <th scope="col">{t("Level")}</th>
          <th scope="col" className="num">{t("Points")}</th>
        </tr>
      </thead>
      <tbody>
        {result.criteria.map((c) => (
          <tr key={c.criterion}>
            <th scope="row">{isolate(c.criterion)}</th>
            <td>{isolate(c.level)}</td>
            <td className="num">{Number(c.points)}</td>
          </tr>
        ))}
      </tbody>
      <tfoot>
        <tr>
          <th scope="row" colSpan={2}>{t("Total")}</th>
          <td className="num">{t("{points} of {max}", { points: Number(result.points), max: Number(result.max) })}</td>
        </tr>
      </tfoot>
    </table>
  );
}
