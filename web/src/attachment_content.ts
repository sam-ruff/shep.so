import init, {
  attachment_catalog,
  attachment_bytes,
  attachment_filename,
} from "./wasm/shep_mail_content";
export interface ReceivedAttachment {
  id: string;
  name: string;
  media_type: string;
  size: number;
}
let ready: Promise<unknown> | undefined;
export const initializeAttachments = () =>
  (ready ??= init({
    module_or_path: new URL(
      "./wasm/shep_mail_content_bg.wasm",
      import.meta.url,
    ),
  }).catch((error) => {
    ready = undefined;
    throw error;
  }));
function rawBytes(raw: string) {
  const limit = 25 * 1024 * 1024;
  if (raw.length > Math.ceil(limit / 3) * 4)
    throw new Error("This message exceeds the current 25 MiB limit.");
  const binary = atob(raw);
  if (binary.length > limit)
    throw new Error("This message exceeds the current 25 MiB limit.");
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}
export const filename = (value: string) => attachment_filename(value);
export const catalogAttachments = (raw: string): ReceivedAttachment[] =>
  JSON.parse(attachment_catalog(rawBytes(raw)));
export const readAttachment = (raw: string, id: string): Uint8Array =>
  attachment_bytes(rawBytes(raw), id);
