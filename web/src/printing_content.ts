import { initializeAttachments, rawBytes } from "./attachment_content";
import { prepare_print } from "./wasm/shep_mail_content";
export interface PrintOptions {
  generation: string;
  plain: boolean;
}
export interface PreparedPrint {
  signature: string;
  title: string;
  document: string;
  issues: string[];
}
export async function preparePrint(
  raw: string,
  options: PrintOptions,
): Promise<PreparedPrint> {
  await initializeAttachments();
  return JSON.parse(prepare_print(rawBytes(raw), JSON.stringify(options)));
}
