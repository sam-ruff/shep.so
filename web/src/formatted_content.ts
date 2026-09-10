import init, { prepare_message } from "./wasm/shep_mail_content";
export interface DocumentOptions {
  generation: string;
  dark: boolean;
  quotes: boolean;
}
export interface PreparedMessage {
  signature: string;
  text: string;
  document: string | null;
  remote_images: { url: string; alt: string }[];
  issues: string[];
}
export async function prepareDocument(
  raw: string,
  options: DocumentOptions,
): Promise<PreparedMessage> {
  if (raw.length > Math.ceil((25 * 1024 * 1024) / 3) * 4)
    throw new Error("This message exceeds the current 25 MiB limit.");
  await init({
    module_or_path: new URL(
      "./wasm/shep_mail_content_bg.wasm",
      import.meta.url,
    ),
  });
  const bytes = Uint8Array.from(atob(raw), (c) => c.charCodeAt(0));
  return JSON.parse(prepare_message(bytes, JSON.stringify(options)));
}
