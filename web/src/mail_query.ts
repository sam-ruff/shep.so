import type { SelectionScope } from "./selection_types";

export const senderName = (sender: string) =>
  sender.replace(/\s*<[^<>]+>$/, "").replace(/^"|"$/g, "") ||
  sender.match(/<([^<>]+)>/)?.[1] ||
  sender;
export const inboxFolder = (folder: string) =>
  folder.toLowerCase() === "inbox" ? "INBOX" : folder;
export function mailMatches(
  mail: {
    folder: string;
    sender: string;
    subject: string;
    body: string;
    unread: boolean;
    starred: boolean;
  },
  scope: SelectionScope,
  sentFolders?: Set<string>,
) {
  const folder = inboxFolder(mail.folder),
    selected = inboxFolder(scope.folder);
  return (
    (folder === selected ||
      (selected === "Sent" && sentFolders?.has(mail.folder))) &&
    (scope.filter !== "Unread" || mail.unread) &&
    (scope.filter !== "Flagged" || mail.starred) &&
    (scope.query ?? "")
      .toLowerCase()
      .trim()
      .split(/\s+/)
      .every((word) =>
        `${mail.sender} ${mail.subject} ${mail.body}`
          .toLowerCase()
          .includes(word),
      )
  );
}
