import type { CompatIssue } from "../../api/compat";
import { Banner } from "../../components";
import { t } from "../../i18n";
import { useCompat } from "./CompatContext";

function issueText(issue: CompatIssue): string {
  switch (issue.kind) {
    case "unreadable":
      return t.compat.issue.unreadable;
    case "too_large":
      return t.compat.issue.tooLarge;
    case "unsupported_version":
      return t.compat.issue.unsupportedVersion(issue.found, issue.supported);
  }
}

/** Bandeau affiché quand le fichier de statuts ne peut pas être exploité. */
export function CompatNotice() {
  const { issue } = useCompat();
  if (!issue) return null;
  return (
    <Banner tone="warning" title={t.compat.issue.title}>
      <p>{issueText(issue)}</p>
    </Banner>
  );
}
