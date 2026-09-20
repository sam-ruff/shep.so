import type { Draft } from "./model";

export interface DraftObservation {
  revision: number;
  text: string;
}

export function observeDraft(draft: Draft): DraftObservation {
  return {
    revision: draft.revision ?? 0,
    text: JSON.stringify([
      draft.id,
      draft.accountId ?? "",
      draft.to,
      draft.cc,
      draft.bcc,
      draft.subject,
      draft.body,
      draft.inReplyTo ?? null,
      draft.references ?? [],
    ]),
  };
}

export function sameDraftObservation(a: DraftObservation, b: DraftObservation) {
  return a.revision === b.revision && a.text === b.text;
}

export class DraftConflict extends Error {
  constructor() {
    super(
      "This draft has newer text or conflicting edits in another editor. Review the saved draft; your text is still here.",
    );
  }
}
