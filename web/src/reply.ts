import type { Draft } from "./model";
import type { CoreMail } from "./provider";
export interface ReplyAddress {
  email: string;
  text: string;
}
export interface ReplyEnvelope {
  reply_to: ReplyAddress[];
  to: ReplyAddress[];
  cc: ReplyAddress[];
  message_id: string | null;
  references: string[];
}
export function envelope(value: unknown): ReplyEnvelope {
  const v = value as ReplyEnvelope;
  if (
    !v ||
    ![v.reply_to, v.to, v.cc].every(
      (a) =>
        Array.isArray(a) &&
        a.every(
          (m) => typeof m?.email === "string" && typeof m.text === "string",
        ),
    ) ||
    !Array.isArray(v.references) ||
    !v.references.every((id) => typeof id === "string") ||
    !(v.message_id === null || typeof v.message_id === "string")
  )
    throw new Error(
      "Reply headers are missing from this cache. Refresh this folder before replying.",
    );
  return v;
}
// This mirrors compose::ReplyHeaders::draft using Rust-parsed mailboxes. Shared
// protocol fixtures check recipient exclusions, order, quoting and references.
export function replyDraft(
  mail: CoreMail,
  body: string,
  headers: ReplyEnvelope,
  ownEmails: string[],
  all: boolean,
): Draft {
  const own = new Set(ownEmails.map((e) => e.toLowerCase())),
    used = new Set<string>();
  const unique = (m: ReplyAddress) => {
    const key = m.email.toLowerCase();
    if (own.has(key) || used.has(key)) return false;
    used.add(key);
    return true;
  };
  const to = headers.reply_to.filter(unique);
  if (!to.length) to.push(...headers.to.filter(unique));
  if (all) to.push(...headers.to.filter(unique));
  const cc = all ? headers.cc.filter(unique) : [];
  const references = [...headers.references];
  if (headers.message_id && !references.includes(headers.message_id))
    references.push(headers.message_id);
  // Match chrono's %d %b %Y regardless of the browser's locale database.
  const stamp = new Date(mail.timestamp * 1000);
  const months = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  const date = `${String(stamp.getUTCDate()).padStart(2, "0")} ${months[stamp.getUTCMonth()]} ${stamp.getUTCFullYear()}`;
  const lines = body.split(/\r?\n/);
  if (lines.at(-1) === "") lines.pop();
  return {
    id: crypto.randomUUID(),
    accountId: mail.account_id,
    to: to.map((m) => m.text).join(", "),
    cc: cc.map((m) => m.text).join(", "),
    bcc: "",
    subject: mail.subject.toLowerCase().startsWith("re:")
      ? mail.subject
      : `Re: ${mail.subject}`,
    body: `\n\nOn ${date}, ${mail.sender} wrote:\n> ${lines.join("\n> ")}`,
    inReplyTo: headers.message_id,
    references: references.slice(-100),
    revision: 0,
    attachments: [],
  };
}
