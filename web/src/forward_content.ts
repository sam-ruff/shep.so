import { initializeAttachments, rawBytes } from "./attachment_content";
import { prepare_forward } from "./wasm/shep_mail_content";
import type { DraftAttachment, ForwardQuote } from "./model";

export interface ForwardContent {
  subject: string;
  body: string;
  forward: ForwardQuote;
  files: { info: Omit<DraftAttachment, "id">; bytes: Uint8Array }[];
}
export interface ForwardPrepared extends ForwardContent {
  accountId: string;
}

// Run in the dedicated worker. Outgoing HTML is metadata for MIME construction,
// never a display document. Binary copies remain separate from metadata JSON.
export async function prepareForwardContent(
  raw: string,
): Promise<ForwardContent> {
  await initializeAttachments();
  const prepared = prepare_forward(rawBytes(raw));
  try {
    const { subject, body, forward, files } = JSON.parse(prepared.metadata());
    return {
      subject,
      body,
      forward,
      files: files.map((info: Omit<DraftAttachment, "id">, i: number) => ({
        info,
        bytes: prepared.file_bytes(i),
      })),
    };
  } finally {
    prepared.free();
  }
}
