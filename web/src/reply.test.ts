import { describe, expect, it } from "vitest";
import cases from "../../shared/compose-fixtures.json";
import { envelope, replyDraft } from "./reply";
import type { CoreMail } from "./provider";

describe("shared Rust/browser reply contract", () => {
  for (const c of cases)
    it(c.name, () => {
      const mail: CoreMail = {
        id: "fixture:INBOX:1.2",
        account_id: "fixture",
        remote_id: "1.2",
        folder: "INBOX",
        sender: c.sender,
        recipient: "",
        subject: c.subject,
        preview: "",
        timestamp: c.timestamp,
        unread: false,
        starred: false,
        attachment_count: 0,
      };
      const reply = replyDraft(
        mail,
        c.text,
        envelope(c.envelope),
        c.own,
        c.all,
      );
      const { in_reply_to, ...expected } = c.expected;
      expect(reply).toMatchObject({
        ...expected,
        inReplyTo: in_reply_to,
        bcc: "",
        accountId: "fixture",
        attachments: [],
      });
    });
  it("refuses missing or malformed cached reply headers", () => {
    for (const value of [undefined, null, {}, { reply_to: ["unknown"] }])
      expect(() => envelope(value)).toThrow("Refresh");
  });
});
